#[cfg(test)]
mod tests;

use askama::Template;
use axum::extract::State;
use hypergraft::{
    PatchSet, PatchStatus,
    live::{LiveProjection, LiveReject, ProjectionError},
};

use crate::{error::AppResult, sessions::SessionId, state::AppState};

#[derive(Template)]
#[template(path = "conversations/templates/recent.html")]
pub(crate) struct RecentConversations {
    conversations: Vec<RecentConversation>,
}

struct RecentConversation {
    href: String,
    title: String,
    status: &'static str,
}

impl RecentConversations {
    pub(crate) fn new(state: &AppState) -> Self {
        let mut records = state.conversations.list();
        records.sort_by_key(|record| std::cmp::Reverse(record.updated_at_ms));
        let active = state.workflow_runs.active_runs();
        let conversations = records
            .into_iter()
            .take(12)
            .map(|record| {
                let latest = state
                    .workflow_runs
                    .for_conversation(&record.id)
                    .into_iter()
                    .next();
                let parent = state
                    .task_loops
                    .for_conversation(&record.id)
                    .into_iter()
                    .next();
                let status = if active.iter().any(|run| {
                    run.conversation_id == Some(record.id)
                        && run.gates.iter().any(|gate| {
                            gate.state == crate::workflows::gates::HumanGateState::AwaitingDecision
                        })
                }) {
                    "Needs your review"
                } else if record
                    .active_job
                    .is_some_and(|job| state.host_approvals.pending_for(record.id, job).is_some())
                {
                    "Needs command approval"
                } else if let Some(job) = record.active_job {
                    if state
                        .sessions
                        .conversation_job(record.id, job)
                        .is_some_and(|job| {
                            job.snapshot().status == crate::sessions::JobStatus::Failed
                        })
                    {
                        "Needs recovery"
                    } else {
                        "In progress"
                    }
                } else if let Some(parent) = parent.filter(|parent| {
                    latest
                        .as_ref()
                        .is_none_or(|run| parent.created_at_ms >= run.created_at_ms)
                }) {
                    super::page::loop_progress(&parent, false).state
                } else if let Some(run) = latest {
                    super::page::workflow_progress(&run).state
                } else {
                    match record.messages.last().map(|message| message.status) {
                        Some(crate::conversations::MessageStatus::Failed) => "Response failed",
                        Some(crate::conversations::MessageStatus::Interrupted) => "Interrupted",
                        Some(crate::conversations::MessageStatus::Pending) => "In progress",
                        _ => "Ready",
                    }
                };
                RecentConversation {
                    href: format!("/conversations/{}", record.id.as_hex()),
                    title: record.title,
                    status,
                }
            })
            .collect();
        Self { conversations }
    }
}

fn patches(state: &AppState) -> Result<PatchSet, hypergraft::PatchBuildError> {
    PatchSet::new().with_children("recent-conversations", &RecentConversations::new(state))
}

pub(super) fn response(state: &AppState) -> AppResult<axum::response::Response> {
    Ok(patches(state)?.respond(PatchStatus::Ok)?)
}

pub(super) async fn live(
    State(state): State<AppState>,
) -> Result<LiveProjection<SessionId>, LiveReject> {
    if !state.vault.has_providers() {
        return Err(LiveReject::Retire);
    }
    // Execution and title stores have separate clocks. The bounded projection reads both.
    let ticks = futures_util::stream::unfold(
        tokio::time::interval(std::time::Duration::from_secs(2)),
        |mut interval| async move {
            interval.tick().await;
            Some(((), interval))
        },
    );
    Ok(LiveProjection::new(ticks, move |_| {
        let state = state.clone();
        async move {
            if !state.vault.has_providers() {
                return Err(ProjectionError::Retire);
            }
            patches(&state).map_err(|_| ProjectionError::Retire)
        }
    }))
}
