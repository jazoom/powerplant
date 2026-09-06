use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use hypergraft::{PatchSet, PatchStatus};
use tokio::sync::mpsc;

use crate::{
    conversations::{
        ConversationId, ConversationRecord, MAXIMUM_REPLY_BYTES, MessageRole, MessageStatus,
    },
    providers::{ChatTurn, ModelEvent, ProviderConnection, ProviderError, Role},
    sessions::{Job, JobStatus, SessionId},
    state::AppState,
};

use super::page::{ConversationObserveContents, MessageBody, MessageView};

const OBSERVE_WAIT: Duration = Duration::from_secs(20);
const OBSERVE_SEGMENT_MAX: Duration = Duration::from_secs(25);

pub(super) async fn run(
    state: AppState,
    session: SessionId,
    conversation: ConversationId,
    record: ConversationRecord,
    connection: ProviderConnection,
    job: Arc<Job>,
) {
    let history = history(&record);
    let instructions = instructions(&state, &record);
    let mut reply = String::new();
    let mut event_count = 0usize;
    let result = tokio::select! {
        biased;
        _ = job.cancelled() => Err(Failure::Cancelled),
        _ = tokio::time::sleep(Duration::from_secs(600)) => Err(Failure::Provider(ProviderError::Unreachable)),
        result = async {
            let mut stream = state.chat.stream_turn(&connection, &history, &[], &[], &instructions).await.map_err(Failure::Provider)?;
            while let Some(event) = tokio::select! {
                biased;
                _ = job.cancelled() => return Err(Failure::Cancelled),
                event = stream.next() => event,
            } {
                event_count += 1;
                if event_count > 4096 {
                    return Err(Failure::Provider(ProviderError::ReplyTooLong));
                }
                match event.map_err(Failure::Provider)? {
                    ModelEvent::Text(text) => {
                        append_text(&mut reply, &text)?;
                        job.push_response(text);
                        state.conversations.append_output(&conversation, job.id(), reply.clone()).map_err(Failure::Store)?;
                    }
                    ModelEvent::Thinking(_) => {}
                    ModelEvent::Usage { input_tokens } => {
                        job.push_usage(crate::providers::ModelUsage {
                            provider: connection.kind,
                            model: connection.model.clone(),
                            input_tokens,
                        });
                    }
                    ModelEvent::ToolCall { .. } => return Err(Failure::Provider(ProviderError::Refused)),
                }
            }
            if reply.trim().is_empty() { return Err(Failure::Provider(ProviderError::EmptyReply)); }
            Ok(())
        } => result,
    };
    let (status, message_status, error) = match result {
        Ok(()) => (JobStatus::Completed, MessageStatus::Complete, None),
        Err(Failure::Cancelled)
        | Err(Failure::Store(crate::conversations::ConversationError::Conflict)) => {
            (JobStatus::Cancelled, MessageStatus::Interrupted, None)
        }
        Err(Failure::Provider(error)) => (
            JobStatus::Failed,
            MessageStatus::Failed,
            Some(error.message().to_owned()),
        ),
        Err(Failure::Store(error)) => (
            JobStatus::Failed,
            MessageStatus::Failed,
            Some(error.message().to_owned()),
        ),
    };
    let settlement =
        state
            .conversations
            .settle_message(&conversation, job.id(), reply, message_status);
    if settlement.is_ok() || settlement == Err(crate::conversations::ConversationError::Conflict) {
        job.finish(status, error.as_deref());
        state
            .sessions
            .finish_conversation_job(&session, conversation, job.id());
    } else {
        job.finish(
            JobStatus::Failed,
            Some("Power Plant could not store the reply. Try again."),
        );
    }
}

enum Failure {
    Provider(ProviderError),
    Store(crate::conversations::ConversationError),
    Cancelled,
}

fn append_text(reply: &mut String, text: &str) -> Result<(), Failure> {
    if text.contains('\0') || reply.len().saturating_add(text.len()) > MAXIMUM_REPLY_BYTES {
        return Err(Failure::Provider(ProviderError::ReplyTooLong));
    }
    reply.push_str(text);
    Ok(())
}

fn instructions(state: &AppState, record: &ConversationRecord) -> String {
    let mut text = record
        .model
        .as_ref()
        .map_or_else(String::new, |model| model.instructions.clone());
    if record.projects.is_empty() {
        return text;
    }
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str("Related project references:\n");
    for id in &record.projects {
        match state.projects.get(id) {
            Some(project) if project.host_path_is_available() => {
                text.push_str("- ");
                text.push_str(&project.name);
                text.push('\n');
            }
            Some(project) => {
                text.push_str("- ");
                text.push_str(&project.name);
                text.push_str(" (unavailable)\n");
            }
            None => text.push_str("- Project record unavailable\n"),
        }
    }
    text.push_str("These references grant no file access, tools or network access.");
    text
}

pub(super) fn history(record: &ConversationRecord) -> Vec<ChatTurn> {
    record
        .messages
        .iter()
        .filter_map(|message| match message.role {
            MessageRole::User => Some(ChatTurn::user(message.text.clone())),
            MessageRole::Assistant
                if message.status != MessageStatus::Pending && !message.text.is_empty() =>
            {
                Some(ChatTurn {
                    role: Role::Assistant,
                    text: message.text.clone(),
                    thinking: String::new(),
                    tools: Vec::new(),
                    activity: Vec::new(),
                    usage: None,
                })
            }
            MessageRole::Assistant => None,
        })
        .collect()
}

pub(super) fn observe_response(
    state: AppState,
    conversation: ConversationId,
    session: SessionId,
    job: Arc<Job>,
    cursor: u64,
) -> axum::response::Response {
    let (tx, rx) = mpsc::channel(4);
    tokio::spawn(observe_segment(
        tx,
        state,
        conversation,
        session,
        job,
        cursor,
    ));
    hypergraft::outcome::stream_response(futures_util::stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|frame| (frame, rx))
    }))
}

async fn observe_segment(
    tx: mpsc::Sender<hypergraft::StreamFrame>,
    state: AppState,
    conversation: ConversationId,
    session: SessionId,
    job: Arc<Job>,
    cursor: u64,
) {
    let mut cursor = cursor;
    let mut budget = hypergraft::StreamBudget::new();
    job.wait_after(cursor, OBSERVE_WAIT).await;
    let started = std::time::Instant::now();
    while job.latest_seq() > cursor {
        let snapshot = job.snapshot();
        let output = job.output_up_to(snapshot.latest_seq);
        if !output.text.is_empty()
            && let Some(frame) = progress_frame(
                &conversation,
                &job,
                snapshot.latest_seq,
                &output.text,
                &mut budget,
            )
            && tx.send(frame).await.is_err()
        {
            return;
        }
        cursor = snapshot.latest_seq;
        if !job.is_running() || started.elapsed() >= OBSERVE_SEGMENT_MAX {
            break;
        }
        job.wait_after(cursor, OBSERVE_WAIT).await;
    }
    let _ = tx
        .send(final_frame(&state, &conversation, session, &job, cursor))
        .await;
}

fn progress_frame(
    conversation: &ConversationId,
    job: &Job,
    cursor: u64,
    text: &str,
    budget: &mut hypergraft::StreamBudget,
) -> Option<hypergraft::StreamFrame> {
    let message = MessageView {
        index: job.assistant_index(),
        user: false,
        html: super::page::reply_html(text),
        status: "Replying",
        streaming: true,
        saveable_plan: false,
        plan_title: String::new(),
        plan_action: String::new(),
        conversation_revision: String::new(),
    };
    let mut patches = PatchSet::new();
    let target = format!("conversation-message-{}", message.index);
    patches
        .children(&target, &MessageBody { message: &message })
        .ok()?;
    patches
        .children(
            "conversation-observe",
            &ConversationObserveContents {
                id: &conversation.as_hex(),
                job_id: &job.id().as_hex(),
                cursor,
                active: true,
            },
        )
        .ok()?;
    let frame = patches.encode_progress().ok()?;
    budget.try_progress(&frame).ok()?;
    Some(frame)
}

fn final_frame(
    state: &AppState,
    conversation: &ConversationId,
    session: SessionId,
    job: &Job,
    cursor: u64,
) -> hypergraft::StreamFrame {
    let snapshot = job.snapshot();
    let record = state.conversations.get(conversation);
    let mut patches = PatchSet::new();
    if snapshot.status != JobStatus::Running
        && let Some(record) = &record
    {
        let view = super::detail_view(state, session, record, &record.title, "");
        if patches
            .children("conversation-detail", &view.contents())
            .is_ok()
            && let Ok(frame) = patches.encode_final(PatchStatus::Ok)
        {
            return frame;
        }
        patches = PatchSet::new();
    }
    if let Some(record) = record
        && let Some(message) = record.messages.get(job.assistant_index())
        && message.request == Some(job.id())
    {
        let index = job.assistant_index();
        let message = MessageView {
            index,
            user: false,
            html: super::page::reply_html(&message.text),
            status: match message.status {
                MessageStatus::Complete => "",
                MessageStatus::Interrupted => "Interrupted",
                MessageStatus::Failed => "Failed",
                MessageStatus::Pending => "Replying",
            },
            streaming: message.status == MessageStatus::Pending,
            saveable_plan: false,
            plan_title: String::new(),
            plan_action: String::new(),
            conversation_revision: String::new(),
        };
        let target = format!("conversation-message-{index}");
        let _ = patches.children(&target, &MessageBody { message: &message });
    }
    let active = snapshot.status == JobStatus::Running;
    let id = conversation.as_hex();
    let job_id = job.id().as_hex();
    let _ = patches.children(
        "conversation-observe",
        &ConversationObserveContents {
            id: &id,
            job_id: &job_id,
            cursor,
            active,
        },
    );
    patches
        .encode_final(if snapshot.status == JobStatus::Failed {
            PatchStatus::UnprocessableEntity
        } else {
            PatchStatus::Ok
        })
        .unwrap_or_else(|_| {
            let mut fallback = PatchSet::new();
            fallback
                .children(
                    "conversation-observe",
                    &ConversationObserveContents {
                        id: &id,
                        job_id: &job_id,
                        cursor,
                        active: false,
                    },
                )
                .expect("bounded observation controls");
            fallback
                .encode_final(PatchStatus::UnprocessableEntity)
                .expect("bounded final frame")
        })
}

#[cfg(test)]
mod tests;
