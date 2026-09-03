use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::sync::mpsc;

use hypergraft::{PatchSet, PatchStatus};

use rig_core::completion::{AssistantContent, Message};

use crate::{
    agents::{AgentId, DirectoryPolicy, ToolId},
    providers::{ChatTurn, ModelEvent, ProviderConnection, ProviderError},
    sandbox::GuestSandbox,
    sessions::{Job, JobEventKind, JobStatus, SessionId},
    state::AppState,
    tools,
};

use super::page::{
    JobCursorContents, JobObserveContents, TranscriptContents, TurnArticle, TurnBody, TurnView,
    assistant_turn, user_turn,
};

pub(crate) struct AgentRunSpec {
    pub(crate) agent_id: AgentId,
    pub(crate) revision: u32,
    pub(crate) preamble: String,
    pub(crate) tools: Vec<rig_core::completion::ToolDefinition>,
    pub(crate) tool_ids: Vec<ToolId>,
    pub(crate) policy: DirectoryPolicy,
    pub(crate) connection: ProviderConnection,
    pub(crate) sandbox: std::sync::Arc<GuestSandbox>,
    pub(crate) output_drafts:
        Option<std::sync::Arc<std::sync::Mutex<crate::workflows::artefacts::output::OutputDrafts>>>,
    pub(crate) required_outputs: Vec<crate::workflows::definition::RequiredOutput>,
}

pub(super) const MIN_PROGRESS_INTERVAL: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(200)
};

// A long job cannot occupy one HTTP response. Each observation ends first.
const OBSERVE_FIRST_WAIT: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(15)
};
const OBSERVE_IDLE_WAIT: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(400)
};
const OBSERVE_SEGMENT_MAX: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(20)
};

// Stay below the 1 MiB envelope after Markdown HTML and the job-observe patch.
pub(super) const MAXIMUM_REPLY_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Eq, PartialEq)]
enum ProgressOffer {
    Sent,
    Skipped,
    Exhausted,
    Closed,
}

pub(super) fn observe_response(
    state: AppState,
    job: Arc<Job>,
    cursor: u64,
    desk_href: String,
) -> axum::response::Response {
    let (tx, rx) = mpsc::channel::<hypergraft::StreamFrame>(4);
    tokio::spawn(observe_segment(tx, state, job, cursor, desk_href));
    let frames = futures_util::stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|item| (item, rx))
    });
    hypergraft::outcome::stream_response(frames)
}

const MAXIMUM_TOOL_ROUNDS: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentOutcome {
    Completed,
    ProviderFailure,
    ToolFailure,
    Cancelled,
}

pub(crate) struct AgentActionEnd {
    pub(crate) outcome: AgentOutcome,
    pub(crate) error: Option<String>,
    pub(crate) reply: String,
}

pub(crate) async fn run_agent_action(
    state: &AppState,
    session_id: SessionId,
    spec: AgentRunSpec,
    turns: Vec<ChatTurn>,
    job: Arc<Job>,
) -> AgentActionEnd {
    let agent_id = spec.agent_id;
    tracing::debug!(
        agent_id = %agent_id,
        agent_revision = spec.revision,
        "agent job started"
    );
    let secret = match spec.connection.auth {
        crate::providers::AuthMethod::ApiKey => Some(spec.connection.api_key.expose().to_owned()),
        crate::providers::AuthMethod::Plan => None,
    };
    let secret = secret.as_deref();
    let mut extra: Vec<Message> = Vec::new();
    let mut reply = String::new();
    let mut published = 0usize;
    let mut last_emit = Instant::now();
    let mut output_visible = false;

    for _ in 0..MAXIMUM_TOOL_ROUNDS {
        if job.cancel_requested() {
            return cancel_action(state, &session_id, &agent_id, &job, &reply);
        }
        let mut events = tokio::select! {
            biased;
            _ = job.cancelled() => {
                return cancel_action(state, &session_id, &agent_id, &job, &reply);
            }
            result = state.chat.stream_turn(
                &spec.connection,
                &turns,
                &extra,
                &spec.tools,
                &spec.preamble,
            ) => match result {
                Ok(stream) => stream,
                Err(error) => {
                    publish_remaining(&job, &reply, published);
                    persist_failure(state, &session_id, &agent_id, &job, &reply);
                    return AgentActionEnd {
                        outcome: AgentOutcome::ProviderFailure,
                        error: Some(error.message().to_owned()),
                        reply: reply.clone(),
                    };
                }
            },
        };

        let mut text = String::new();
        let mut calls = Vec::new();
        loop {
            let chunk = tokio::select! {
                biased;
                _ = job.cancelled() => {
                    return cancel_action(state, &session_id, &agent_id, &job, &reply);
                }
                chunk = events.next() => chunk,
            };
            let Some(chunk) = chunk else {
                break;
            };
            match chunk {
                Ok(ModelEvent::Text(piece)) => {
                    text.push_str(&piece);
                    if append_and_publish(
                        &job,
                        &mut reply,
                        &mut published,
                        &mut last_emit,
                        &mut output_visible,
                        &piece,
                    ) {
                        persist_failure(state, &session_id, &agent_id, &job, &reply);
                        return AgentActionEnd {
                            outcome: AgentOutcome::ProviderFailure,
                            error: Some(ProviderError::ReplyTooLong.message().to_owned()),
                            reply: reply.clone(),
                        };
                    }
                }
                Ok(ModelEvent::ToolCall {
                    id,
                    name,
                    arguments,
                }) => calls.push((id, name, arguments)),
                Err(error) => {
                    publish_remaining(&job, &reply, published);
                    persist_failure(state, &session_id, &agent_id, &job, &reply);
                    return AgentActionEnd {
                        outcome: AgentOutcome::ProviderFailure,
                        error: Some(error.message().to_owned()),
                        reply: reply.clone(),
                    };
                }
            }
        }

        if job.cancel_requested() {
            return cancel_action(state, &session_id, &agent_id, &job, &reply);
        }

        if calls.is_empty() {
            publish_remaining(&job, &reply, published);
            if reply.trim().is_empty() {
                persist_failure(state, &session_id, &agent_id, &job, "");
                return AgentActionEnd {
                    outcome: AgentOutcome::ProviderFailure,
                    error: Some(ProviderError::EmptyReply.message().to_owned()),
                    reply: reply.clone(),
                };
            }
            persist_success(state, &session_id, &agent_id, &job, &reply);
            return AgentActionEnd {
                outcome: AgentOutcome::Completed,
                error: None,
                reply: reply.clone(),
            };
        }

        extra.push(assistant_tool_message(&text, &calls));
        let context = tools::AgentToolContext {
            sandbox: &spec.sandbox,
            policy: &spec.policy,
            job: &job,
            tools: &spec.tool_ids,
            output_drafts: spec.output_drafts.as_deref(),
            required_outputs: &spec.required_outputs,
        };
        for (id, name, arguments) in calls {
            let trace = tools::invoke(&context, &name, &arguments).await;
            if job.cancel_requested() {
                return cancel_action(state, &session_id, &agent_id, &job, &reply);
            }
            let output = tools::redact(&trace.output, secret);
            let visible = tools::render_trace(&trace.label, &output);
            if append_and_publish(
                &job,
                &mut reply,
                &mut published,
                &mut last_emit,
                &mut output_visible,
                &format!("\n\n{visible}"),
            ) {
                persist_failure(state, &session_id, &agent_id, &job, &reply);
                return AgentActionEnd {
                    outcome: AgentOutcome::ProviderFailure,
                    error: Some(ProviderError::ReplyTooLong.message().to_owned()),
                    reply: reply.clone(),
                };
            }
            extra.push(Message::tool_result(id, name, output));
        }
    }

    publish_remaining(&job, &reply, published);
    persist_failure(state, &session_id, &agent_id, &job, &reply);
    AgentActionEnd {
        outcome: AgentOutcome::ToolFailure,
        error: Some(TOOL_LOOP_LIMIT.to_owned()),
        reply,
    }
}

const TOOL_LOOP_LIMIT: &str = "The agent stopped after too many tool calls. Try again.";

fn assistant_tool_message(text: &str, calls: &[(String, String, serde_json::Value)]) -> Message {
    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(AssistantContent::text(text));
    }
    for (id, name, arguments) in calls {
        content.push(AssistantContent::tool_call(
            id.clone(),
            name.clone(),
            arguments.clone(),
        ));
    }
    Message::Assistant { id: None, content }
}

fn append_and_publish(
    job: &Job,
    reply: &mut String,
    published: &mut usize,
    last_emit: &mut Instant,
    output_visible: &mut bool,
    piece: &str,
) -> bool {
    let truncated = append_bounded(reply, piece);
    if progress_due(*output_visible, *last_emit) {
        publish_remaining(job, reply, *published);
        *published = reply.len();
        *output_visible = *published > 0;
        *last_emit = Instant::now();
    }
    truncated
}

#[cfg(any())]
pub(super) async fn run_job(
    state: AppState,
    session_id: SessionId,
    connection: ProviderConnection,
    turns: Vec<ChatTurn>,
    job: Arc<Job>,
) {
    let mut tokens = tokio::select! {
        biased;
        _ = job.cancelled() => {
            finish_cancelled(&state, &session_id, &job, "");
            return;
        }
        result = state.chat.stream(&connection, &turns) => match result {
            Ok(stream) => stream,
            Err(error) => {
                let _ = state
                    .sessions
                    .fail_turn(&session_id, &job.id(), String::new());
                job.finish(JobStatus::Failed, Some(error.message()));
                return;
            }
        },
    };

    let mut reply = String::new();
    let mut published = 0usize;
    let mut last_emit = Instant::now();
    let mut output_visible = false;

    loop {
        let chunk = tokio::select! {
            biased;
            _ = job.cancelled() => {
                finish_cancelled(&state, &session_id, &job, &reply);
                return;
            }
            chunk = tokens.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break;
        };
        let piece = match chunk {
            Ok(text) => text,
            Err(error) => {
                publish_remaining(&job, &reply, published);
                persist_failure(&state, &session_id, &job, &reply);
                job.finish(JobStatus::Failed, Some(error.message()));
                return;
            }
        };
        let truncated = append_bounded(&mut reply, &piece);

        if progress_due(output_visible, last_emit) {
            publish_remaining(&job, &reply, published);
            published = reply.len();
            output_visible = published > 0;
            last_emit = Instant::now();
        }

        if truncated {
            publish_remaining(&job, &reply, published);
            persist_failure(&state, &session_id, &job, &reply);
            job.finish(
                JobStatus::Failed,
                Some(ProviderError::ReplyTooLong.message()),
            );
            return;
        }
    }

    if job.cancel_requested() {
        finish_cancelled(&state, &session_id, &job, &reply);
        return;
    }

    publish_remaining(&job, &reply, published);

    if reply.trim().is_empty() {
        persist_failure(&state, &session_id, &job, "");
        job.finish(JobStatus::Failed, Some(ProviderError::EmptyReply.message()));
        return;
    }

    persist_success(&state, &session_id, &job, &reply);
    job.finish(JobStatus::Completed, None);
}

pub(crate) fn bound_reply(text: &str) -> &str {
    if text.len() <= MAXIMUM_REPLY_BYTES {
        return text;
    }
    let mut end = MAXIMUM_REPLY_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn append_bounded(reply: &mut String, piece: &str) -> bool {
    let remaining = MAXIMUM_REPLY_BYTES.saturating_sub(reply.len());
    if piece.len() <= remaining {
        reply.push_str(piece);
        return false;
    }
    let mut end = remaining;
    while end > 0 && !piece.is_char_boundary(end) {
        end -= 1;
    }
    reply.push_str(&piece[..end]);
    true
}

fn persist_success(_state: &AppState, _id: &SessionId, _agent: &AgentId, _job: &Job, _reply: &str) {
    // Workflow execution settles the active turn after all durable outputs are visible.
}

fn persist_failure(_state: &AppState, _id: &SessionId, _agent: &AgentId, _job: &Job, _reply: &str) {
    // Workflow execution owns the single terminal settlement.
}

fn cancel_action(
    state: &AppState,
    id: &SessionId,
    agent: &AgentId,
    job: &Job,
    reply: &str,
) -> AgentActionEnd {
    publish_remaining(job, reply, job.snapshot().output.len().min(reply.len()));
    persist_failure(state, id, agent, job, reply);
    AgentActionEnd {
        outcome: AgentOutcome::Cancelled,
        error: None,
        reply: reply.to_owned(),
    }
}

fn publish_remaining(job: &Job, reply: &str, published: usize) {
    if published >= reply.len() {
        return;
    }
    let _ = job.push_output(reply[published..].to_owned());
}

fn progress_due(output_visible: bool, last_emit: Instant) -> bool {
    !output_visible || last_emit.elapsed() >= MIN_PROGRESS_INTERVAL
}

async fn observe_segment(
    tx: mpsc::Sender<hypergraft::StreamFrame>,
    state: AppState,
    job: Arc<Job>,
    cursor: u64,
    desk_href: String,
) {
    let mut budget = hypergraft::StreamBudget::new();
    let mut sent = cursor;
    let mut assistant_visible = job.has_output_at_or_before(cursor);
    let job_id = job.id().as_hex();

    job.wait_after(sent, OBSERVE_FIRST_WAIT).await;
    let started = Instant::now();

    loop {
        let events = job.events_after(sent);
        if events.is_empty() {
            break;
        }
        let mut text = job.output_up_to(sent);
        let mut exhausted = false;
        for event in events {
            match event.kind {
                JobEventKind::Output { delta } => {
                    text.push_str(&delta);
                    match offer_output_progress(
                        &tx,
                        &mut budget,
                        &job,
                        &job_id,
                        event.seq,
                        &text,
                        assistant_visible,
                    )
                    .await
                    {
                        ProgressOffer::Sent => {
                            assistant_visible = true;
                            sent = event.seq;
                        }
                        ProgressOffer::Skipped => sent = event.seq,
                        ProgressOffer::Exhausted => {
                            exhausted = true;
                            break;
                        }
                        ProgressOffer::Closed => return,
                    }
                }
                JobEventKind::Completed | JobEventKind::Failed | JobEventKind::Cancelled => {
                    sent = event.seq;
                }
            }
        }
        if exhausted {
            break;
        }
        if job.latest_seq() > sent {
            continue;
        }
        if !should_keep_open(&job, started) {
            break;
        }
        job.wait_after(sent, OBSERVE_IDLE_WAIT).await;
        if job.latest_seq() == sent {
            break;
        }
    }

    send_observe_final(
        &tx,
        &state,
        &job,
        &job_id,
        sent,
        assistant_visible,
        &desk_href,
    )
    .await;
}

fn should_keep_open(job: &Job, started: Instant) -> bool {
    if !job.is_running() || OBSERVE_SEGMENT_MAX.is_zero() {
        return false;
    }
    started.elapsed() < OBSERVE_SEGMENT_MAX
}

async fn offer_output_progress(
    tx: &mpsc::Sender<hypergraft::StreamFrame>,
    budget: &mut hypergraft::StreamBudget,
    job: &Job,
    job_id: &str,
    cursor: u64,
    text: &str,
    already_visible: bool,
) -> ProgressOffer {
    let turn = assistant_turn(job.assistant_index(), text);
    let frame = encode_output_progress(job_id, cursor, &turn, already_visible);
    offer_progress(tx, budget, frame, "construct job progress frame").await
}

fn encode_output_progress(
    job_id: &str,
    cursor: u64,
    turn: &TurnView,
    already_visible: bool,
) -> Result<hypergraft::StreamFrame, hypergraft::PatchBuildError> {
    let mut patches = PatchSet::new();
    if already_visible {
        patches.children(&turn.id, &TurnBody { turn })?;
    } else {
        patches.append("transcript", &TurnArticle { turn })?;
    }
    // Cursor travels with the event so a retry cannot replay an applied seq.
    patches.children("job-cursor", &JobCursorContents { job_id, cursor })?;
    patches.encode_progress()
}

async fn offer_progress(
    tx: &mpsc::Sender<hypergraft::StreamFrame>,
    budget: &mut hypergraft::StreamBudget,
    frame: Result<hypergraft::StreamFrame, hypergraft::PatchBuildError>,
    operation: &'static str,
) -> ProgressOffer {
    let frame = match frame {
        Ok(frame) => frame,
        Err(error) => {
            crate::error::trace_patch_build_failure(operation, &error);
            return ProgressOffer::Skipped;
        }
    };
    if budget.try_progress(&frame).is_err() {
        return ProgressOffer::Exhausted;
    }
    if tx.send(frame).await.is_ok() {
        ProgressOffer::Sent
    } else {
        ProgressOffer::Closed
    }
}

async fn send_observe_final(
    tx: &mpsc::Sender<hypergraft::StreamFrame>,
    state: &AppState,
    job: &Job,
    job_id: &str,
    cursor: u64,
    assistant_visible: bool,
    desk_href: &str,
) {
    match encode_observe_final(state, job, job_id, cursor, assistant_visible, desk_href) {
        Ok(frame) => {
            let _ = tx.send(frame).await;
        }
        Err(build_error) => {
            crate::error::trace_patch_build_failure(
                "construct job observation final",
                &build_error,
            );
            match encode_observe_final(state, job, job_id, cursor, false, desk_href) {
                Ok(frame) => {
                    let _ = tx.send(frame).await;
                }
                Err(fallback_error) => {
                    crate::error::trace_patch_build_failure(
                        "construct fallback job observation final",
                        &fallback_error,
                    );
                }
            }
        }
    }
}

fn encode_observe_final(
    state: &AppState,
    job: &Job,
    job_id: &str,
    cursor: u64,
    assistant_visible: bool,
    desk_href: &str,
) -> Result<hypergraft::StreamFrame, hypergraft::PatchBuildError> {
    let snapshot = job.snapshot();
    let more = snapshot.latest_seq > cursor || snapshot.status == JobStatus::Running;
    let mut patches = PatchSet::new();
    if !assistant_visible && !snapshot.output.is_empty() {
        let turn = assistant_turn(snapshot.assistant_index, &snapshot.output);
        patches.append("transcript", &TurnArticle { turn: &turn })?;
    }
    let run_id = snapshot.run_id.as_hex();
    let run_step = snapshot.step_label.as_str();
    let workflow_name = snapshot.workflow_name.as_str();
    if more {
        let status = if snapshot.cancel_requested {
            "Stopping"
        } else if snapshot.step_label.is_empty() {
            "Working"
        } else {
            snapshot.step_label.as_str()
        };
        patches.children(
            "job-observe",
            &JobObserveContents::observing(
                job_id,
                cursor,
                status,
                "",
                desk_href,
                &run_id,
                run_step,
                workflow_name,
            ),
        )?;
    } else {
        let error = snapshot.error.as_deref().unwrap_or("");
        let review_href = super::review_href_for(state, &snapshot);
        let quick_task_finished = super::quick_task_finished(state, &snapshot);
        patches.children(
            "job-observe",
            &JobObserveContents::idle(
                error,
                desk_href,
                &run_id,
                run_step,
                workflow_name,
                &review_href,
                quick_task_finished,
            ),
        )?;
    }
    let status = match snapshot.status {
        JobStatus::Failed => provider_status_from_message(snapshot.error.as_deref()),
        _ => PatchStatus::Ok,
    };
    patches.encode_final(status)
}

// Stream settlement cannot carry 429. Rate limits keep the recovery copy and 422.
fn provider_status_from_message(message: Option<&str>) -> PatchStatus {
    match message {
        Some(message) if message == ProviderError::Rejected.message() => PatchStatus::Unauthorized,
        _ => PatchStatus::UnprocessableEntity,
    }
}

pub(super) fn user_transcript_patch(
    turns: &[ChatTurn],
) -> Result<PatchSet, hypergraft::PatchBuildError> {
    let user_index = turns.len() - 1;
    let user = &turns[user_index];
    let turn = user_turn(user_index, &user.text);
    if user_index == 0 {
        let view = [turn];
        PatchSet::new().with_children("transcript", &TranscriptContents { turns: &view })
    } else {
        PatchSet::new().with_append("transcript", &TurnArticle { turn: &turn })
    }
}
