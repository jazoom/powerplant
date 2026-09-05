use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::sync::mpsc;

use hypergraft::{PatchSet, PatchStatus};

use rig_core::completion::{AssistantContent, Message};

use crate::{
    agents::{AgentId, DirectoryPolicy, ToolId},
    providers::{
        AssistantActivity, AssistantReply, ChatTurn, ModelEvent, ModelUsage, ProviderConnection,
        ProviderError, ToolOutput,
    },
    sandbox::GuestSandbox,
    sessions::{Job, JobEventKind, JobStatus, SessionId},
    state::AppState,
    tools,
};

use super::page::{
    DeskStatusContents, JobCursorContents, JobObserveContents, ModelContextContents,
    ModelContextView, TranscriptContents, TurnArticle, TurnBody, TurnView, assistant_reply_turn,
    user_turn,
};

#[cfg(test)]
mod tests;

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
const THINKING_INITIAL_DELAY: Duration = Duration::from_millis(75);
const THINKING_PROGRESS_INTERVAL: Duration = Duration::from_millis(75);
const MAXIMUM_THINKING_PROGRESS_BYTES: usize = 192;

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
pub(super) const MAXIMUM_MODEL_REPLY_BYTES: usize = 64 * 1024;
pub(super) const MAXIMUM_THINKING_BYTES: usize = 64 * 1024;
const MAXIMUM_VISIBLE_TOOL_BYTES: usize = 64 * 1024;

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
    pub(crate) reply: AssistantReply,
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
    let mut reply = AssistantReply::default();
    let mut model_reply_bytes = 0usize;
    let mut thinking_bytes = 0usize;
    let mut visible_tool_bytes = 0usize;
    let mut published_response = 0usize;
    let mut thinking_progress = ThinkingProgress::default();
    let mut last_emit = Instant::now();
    let mut output_visible = false;

    for _ in 0..MAXIMUM_TOOL_ROUNDS {
        thinking_progress.begin_phase();
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
                    thinking_progress.flush(&job, &reply.thinking);
                    publish_reply_remaining(
                        &job,
                        &reply,
                        published_response,
                        thinking_progress.published,
                    );
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
            let thinking_deadline = thinking_progress.deadline(&reply.thinking);
            let wait_for_thinking = async {
                match thinking_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                    None => std::future::pending().await,
                }
            };
            let chunk = tokio::select! {
                biased;
                _ = job.cancelled() => {
                    return cancel_action(state, &session_id, &agent_id, &job, &reply);
                }
                _ = wait_for_thinking => {
                    thinking_progress.publish_due(&job, &reply.thinking, Instant::now());
                    continue;
                }
                chunk = events.next() => chunk,
            };
            let Some(chunk) = chunk else {
                break;
            };
            match chunk {
                Ok(ModelEvent::Text(piece)) => {
                    thinking_progress.flush(&job, &reply.thinking);
                    text.push_str(&piece);
                    let truncated =
                        append_model_piece(&mut reply.text, &piece, &mut model_reply_bytes);
                    publish_progress(
                        &job,
                        &reply.text,
                        &mut published_response,
                        OutputChannel::Response,
                        &mut last_emit,
                        &mut output_visible,
                    );
                    if truncated {
                        publish_reply_remaining(
                            &job,
                            &reply,
                            published_response,
                            thinking_progress.published,
                        );
                        persist_failure(state, &session_id, &agent_id, &job, &reply);
                        return AgentActionEnd {
                            outcome: AgentOutcome::ProviderFailure,
                            error: Some(ProviderError::ReplyTooLong.message().to_owned()),
                            reply: reply.clone(),
                        };
                    }
                }
                Ok(ModelEvent::Thinking(piece)) => {
                    append_thinking_piece(&mut reply, &piece, &mut thinking_bytes);
                    thinking_progress.note_pending(&reply.thinking, Instant::now());
                }
                Ok(ModelEvent::ToolCall {
                    id,
                    name,
                    arguments,
                }) => calls.push((id, name, arguments)),
                Ok(ModelEvent::Usage { input_tokens }) => {
                    let usage = ModelUsage {
                        provider: spec.connection.kind,
                        model: spec.connection.model.clone(),
                        input_tokens,
                    };
                    reply.usage = Some(usage.clone());
                    job.push_usage(usage);
                }
                Err(error) => {
                    thinking_progress.flush(&job, &reply.thinking);
                    publish_reply_remaining(
                        &job,
                        &reply,
                        published_response,
                        thinking_progress.published,
                    );
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
            thinking_progress.flush(&job, &reply.thinking);
            publish_reply_remaining(
                &job,
                &reply,
                published_response,
                thinking_progress.published,
            );
            if reply.text.trim().is_empty() {
                persist_failure(
                    state,
                    &session_id,
                    &agent_id,
                    &job,
                    &AssistantReply::default(),
                );
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

        publish_reply_before_tools(
            &job,
            &reply,
            &mut published_response,
            &mut thinking_progress,
        );
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
            if let Some(visible) =
                visible_tool_output(trace.label, &output, &mut visible_tool_bytes)
            {
                job.push_tool(visible.clone());
                reply.push_tool(visible);
                output_visible = true;
            }
            extra.push(Message::tool_result(id, name, output));
        }
    }

    thinking_progress.flush(&job, &reply.thinking);
    publish_reply_remaining(
        &job,
        &reply,
        published_response,
        thinking_progress.published,
    );
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

fn append_model_piece(reply: &mut String, piece: &str, model_reply_bytes: &mut usize) -> bool {
    let remaining = MAXIMUM_MODEL_REPLY_BYTES.saturating_sub(*model_reply_bytes);
    if piece.len() <= remaining {
        reply.push_str(piece);
        *model_reply_bytes += piece.len();
        return false;
    }
    let mut end = remaining;
    while end > 0 && !piece.is_char_boundary(end) {
        end -= 1;
    }
    reply.push_str(&piece[..end]);
    *model_reply_bytes += end;
    true
}

fn append_thinking_piece(reply: &mut AssistantReply, piece: &str, thinking_bytes: &mut usize) {
    let remaining = MAXIMUM_THINKING_BYTES.saturating_sub(*thinking_bytes);
    let mut end = piece.len().min(remaining);
    while end > 0 && !piece.is_char_boundary(end) {
        end -= 1;
    }
    reply.push_thinking(&piece[..end]);
    *thinking_bytes += end;
}

fn visible_tool_output(
    label: String,
    output: &str,
    visible_tool_bytes: &mut usize,
) -> Option<ToolOutput> {
    const MARKER: &str = "\n[output truncated]";
    let remaining = MAXIMUM_VISIBLE_TOOL_BYTES.saturating_sub(*visible_tool_bytes);
    let output_limit = remaining.checked_sub(label.len())?;
    if output_limit == 0 {
        return None;
    }
    let visible = if output.len() <= output_limit {
        output.to_owned()
    } else if output_limit > MARKER.len() {
        let mut end = output_limit - MARKER.len();
        while end > 0 && !output.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}{}", &output[..end], MARKER)
    } else {
        let mut end = output_limit;
        while end > 0 && !output.is_char_boundary(end) {
            end -= 1;
        }
        output[..end].to_owned()
    };
    *visible_tool_bytes += label.len().saturating_add(visible.len());
    Some(ToolOutput {
        label,
        output: visible,
    })
}

#[derive(Clone, Copy)]
enum OutputChannel {
    Response,
    Thinking,
}

#[derive(Default)]
struct ThinkingProgress {
    published: usize,
    phase_started: Option<Instant>,
    last_emit: Option<Instant>,
}

impl ThinkingProgress {
    fn begin_phase(&mut self) {
        self.phase_started = None;
        self.last_emit = None;
    }

    fn note_pending(&mut self, text: &str, now: Instant) {
        if self.published < text.len() && self.phase_started.is_none() {
            self.phase_started = Some(now);
        }
    }

    fn deadline(&self, text: &str) -> Option<Instant> {
        if self.published >= text.len() {
            return None;
        }
        self.last_emit
            .map(|last_emit| last_emit + THINKING_PROGRESS_INTERVAL)
            .or_else(|| {
                self.phase_started
                    .map(|started| started + THINKING_INITIAL_DELAY)
            })
    }

    fn publish_due(&mut self, job: &Job, text: &str, now: Instant) -> bool {
        if self.deadline(text).is_none_or(|deadline| now < deadline) {
            return false;
        }
        self.publish_next(job, text, now)
    }

    fn flush(&mut self, job: &Job, text: &str) {
        while self.published < text.len() {
            self.publish_next(job, text, Instant::now());
        }
    }

    fn publish_next(&mut self, job: &Job, text: &str, now: Instant) -> bool {
        let end = bounded_progress_end(text, self.published, MAXIMUM_THINKING_PROGRESS_BYTES);
        if end <= self.published {
            return false;
        }
        publish_range(job, text, self.published, end, OutputChannel::Thinking);
        self.published = end;
        self.last_emit = Some(now);
        true
    }
}

fn bounded_progress_end(text: &str, published: usize, maximum: usize) -> usize {
    let mut end = published.saturating_add(maximum).min(text.len());
    while end > published && !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn publish_progress(
    job: &Job,
    text: &str,
    published: &mut usize,
    channel: OutputChannel,
    last_emit: &mut Instant,
    output_visible: &mut bool,
) {
    if progress_due(*output_visible, *last_emit) {
        publish_remaining(job, text, *published, channel);
        *published = text.len();
        *output_visible = *published > 0;
        *last_emit = Instant::now();
    }
}

pub(crate) fn bound_reply(reply: &AssistantReply) -> AssistantReply {
    let mut bounded = reply.clone();
    truncate_utf8(&mut bounded.text, MAXIMUM_MODEL_REPLY_BYTES);
    if bounded.activity.is_empty() {
        truncate_utf8(&mut bounded.thinking, MAXIMUM_THINKING_BYTES);
        let mut tool_bytes = 0usize;
        bounded.tools.retain_mut(|tool| {
            let Some(visible) =
                visible_tool_output(tool.label.clone(), &tool.output, &mut tool_bytes)
            else {
                return false;
            };
            *tool = visible;
            true
        });
        return bounded;
    }

    bounded.thinking.clear();
    bounded.tools.clear();
    let mut activities = Vec::new();
    let mut thinking_bytes = 0usize;
    let mut tool_bytes = 0usize;
    for activity in std::mem::take(&mut bounded.activity) {
        match activity {
            AssistantActivity::Thinking(mut thinking) => {
                let remaining = MAXIMUM_THINKING_BYTES.saturating_sub(thinking_bytes);
                truncate_utf8(&mut thinking, remaining);
                if !thinking.is_empty() {
                    thinking_bytes += thinking.len();
                    bounded.thinking.push_str(&thinking);
                    activities.push(AssistantActivity::Thinking(thinking));
                }
            }
            AssistantActivity::Tool(tool) => {
                let ToolOutput { label, output } = tool;
                if let Some(tool) = visible_tool_output(label, &output, &mut tool_bytes) {
                    bounded.tools.push(tool.clone());
                    activities.push(AssistantActivity::Tool(tool));
                }
            }
        }
    }
    bounded.activity = activities;
    bounded
}

fn truncate_utf8(text: &mut String, maximum: usize) {
    if text.len() <= maximum {
        return;
    }
    let mut end = maximum;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

fn persist_success(
    _state: &AppState,
    _id: &SessionId,
    _agent: &AgentId,
    _job: &Job,
    _reply: &AssistantReply,
) {
    // Workflow execution settles the active turn after all durable outputs are visible.
}

fn persist_failure(
    _state: &AppState,
    _id: &SessionId,
    _agent: &AgentId,
    _job: &Job,
    _reply: &AssistantReply,
) {
    // Workflow execution owns the single terminal settlement.
}

fn cancel_action(
    state: &AppState,
    id: &SessionId,
    agent: &AgentId,
    job: &Job,
    reply: &AssistantReply,
) -> AgentActionEnd {
    let published = job.snapshot().output;
    let mut thinking_progress = ThinkingProgress {
        published: published.thinking.len().min(reply.thinking.len()),
        ..ThinkingProgress::default()
    };
    thinking_progress.flush(job, &reply.thinking);
    publish_reply_remaining(
        job,
        reply,
        published.text.len().min(reply.text.len()),
        thinking_progress.published,
    );
    persist_failure(state, id, agent, job, reply);
    AgentActionEnd {
        outcome: AgentOutcome::Cancelled,
        error: None,
        reply: reply.clone(),
    }
}

fn publish_reply_before_tools(
    job: &Job,
    reply: &AssistantReply,
    published_response: &mut usize,
    thinking_progress: &mut ThinkingProgress,
) {
    thinking_progress.flush(job, &reply.thinking);
    publish_reply_remaining(job, reply, *published_response, thinking_progress.published);
    *published_response = reply.text.len();
}

fn publish_reply_remaining(
    job: &Job,
    reply: &AssistantReply,
    published_response: usize,
    published_thinking: usize,
) {
    publish_remaining(
        job,
        &reply.text,
        published_response,
        OutputChannel::Response,
    );
    publish_remaining(
        job,
        &reply.thinking,
        published_thinking,
        OutputChannel::Thinking,
    );
}

fn publish_remaining(job: &Job, text: &str, published: usize, channel: OutputChannel) {
    publish_range(job, text, published, text.len(), channel);
}

fn publish_range(job: &Job, text: &str, published: usize, end: usize, channel: OutputChannel) {
    if published >= end || end > text.len() {
        return;
    }
    let delta = text[published..end].to_owned();
    match channel {
        OutputChannel::Response => {
            let _ = job.push_response(delta);
        }
        OutputChannel::Thinking => {
            let _ = job.push_thinking(delta);
        }
    }
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
        let mut output = job.output_up_to(sent);
        let mut exhausted = false;
        for event in events {
            let output_changed = match event.kind {
                JobEventKind::Response { delta } => {
                    output.text.push_str(&delta);
                    true
                }
                JobEventKind::Thinking { delta } => {
                    output.push_thinking(&delta);
                    true
                }
                JobEventKind::Tool { output: tool } => {
                    output.push_tool(tool);
                    true
                }
                JobEventKind::Usage { usage } => {
                    output.usage = Some(usage);
                    false
                }
                JobEventKind::Completed | JobEventKind::Failed | JobEventKind::Cancelled => false,
            };
            if !output_changed {
                sent = event.seq;
                continue;
            }
            match offer_output_progress(
                &tx,
                &mut budget,
                &job,
                &job_id,
                event.seq,
                &output,
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
    output: &AssistantReply,
    already_visible: bool,
) -> ProgressOffer {
    let turn = assistant_reply_turn(job.assistant_index(), output, true);
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
    if !snapshot.output.is_empty() {
        let turn = assistant_reply_turn(snapshot.assistant_index, &snapshot.output, more);
        if assistant_visible {
            patches.children(&turn.id, &TurnBody { turn: &turn })?;
        } else {
            patches.append("transcript", &TurnArticle { turn: &turn })?;
        }
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
        patches.children("desk-status", &DeskStatusContents::active(status))?;
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
        patches.children(
            "desk-status",
            &DeskStatusContents::idle(&review_href, quick_task_finished),
        )?;
    }
    if let Some(usage) = snapshot.output.usage.as_ref() {
        let context = ModelContextView::from_model(
            &state.models_dev,
            usage.provider,
            &usage.model,
            Some(usage),
        );
        patches.children(
            "desk-model-context",
            &ModelContextContents {
                model_context: &context,
            },
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
