use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::sync::mpsc;

use hypergraft::{PatchSet, PatchStatus};

use crate::{
    providers::{ChatTurn, ProviderConnection, ProviderError},
    sessions::{Job, JobEventKind, JobStatus, SessionId},
    state::AppState,
};

use super::page::{
    ComposerContents, JobCursorContents, TranscriptContents, TurnArticle, TurnBody, TurnView,
    assistant_turn, user_turn,
};

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

// Stay below the 1 MiB envelope after Markdown HTML and the composer patch.
pub(super) const MAXIMUM_REPLY_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Eq, PartialEq)]
enum ProgressOffer {
    Sent,
    Skipped,
    Exhausted,
    Closed,
}

pub(super) fn observe_response(job: Arc<Job>, cursor: u64) -> axum::response::Response {
    let (tx, rx) = mpsc::channel::<hypergraft::StreamFrame>(4);
    tokio::spawn(observe_segment(tx, job, cursor));
    let frames = futures_util::stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|item| (item, rx))
    });
    hypergraft::outcome::stream_response(frames)
}

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

pub(super) fn bound_reply(text: &str) -> &str {
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

fn persist_success(state: &AppState, id: &SessionId, job: &Job, reply: &str) {
    let _ = state
        .sessions
        .finish_turn(id, &job.id(), bound_reply(reply).to_owned());
}

fn persist_failure(state: &AppState, id: &SessionId, job: &Job, reply: &str) {
    let _ = state
        .sessions
        .fail_turn(id, &job.id(), bound_reply(reply).to_owned());
}

fn finish_cancelled(state: &AppState, id: &SessionId, job: &Job, reply: &str) {
    publish_remaining(job, reply, job.snapshot().output.len().min(reply.len()));
    persist_failure(state, id, job, reply);
    job.finish(JobStatus::Cancelled, None);
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

async fn observe_segment(tx: mpsc::Sender<hypergraft::StreamFrame>, job: Arc<Job>, cursor: u64) {
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
                        &job_id,
                        event.seq,
                        job.assistant_index(),
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

    send_observe_final(&tx, &job, &job_id, sent, assistant_visible).await;
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
    job_id: &str,
    cursor: u64,
    assistant_index: usize,
    text: &str,
    already_visible: bool,
) -> ProgressOffer {
    let turn = assistant_turn(assistant_index, text);
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
    job: &Job,
    job_id: &str,
    cursor: u64,
    assistant_visible: bool,
) {
    match encode_observe_final(job, job_id, cursor, assistant_visible) {
        Ok(frame) => {
            let _ = tx.send(frame).await;
        }
        Err(build_error) => {
            crate::error::trace_patch_build_failure(
                "construct job observation final",
                &build_error,
            );
            match encode_observe_final(job, job_id, cursor, false) {
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
    job: &Job,
    job_id: &str,
    cursor: u64,
    assistant_visible: bool,
) -> Result<hypergraft::StreamFrame, hypergraft::PatchBuildError> {
    let snapshot = job.snapshot();
    let more = snapshot.latest_seq > cursor || snapshot.status == JobStatus::Running;
    let mut patches = PatchSet::new();
    if !assistant_visible && !snapshot.output.is_empty() {
        let turn = assistant_turn(snapshot.assistant_index, &snapshot.output);
        patches.append("transcript", &TurnArticle { turn: &turn })?;
    }
    if more {
        let status = if snapshot.cancel_requested {
            "Stopping"
        } else {
            "Writing"
        };
        patches.children(
            "composer",
            &ComposerContents::observing(job_id, cursor, status, ""),
        )?;
    } else {
        let error = snapshot.error.unwrap_or("");
        patches.children("composer", &ComposerContents::idle(error))?;
    }
    let status = match snapshot.status {
        JobStatus::Failed => provider_status_from_message(snapshot.error),
        _ => PatchStatus::Ok,
    };
    patches.encode_final(status)
}

// Stream settlement cannot carry 429. Rate limits keep the recovery copy and 422.
fn provider_status_from_message(message: Option<&'static str>) -> PatchStatus {
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
