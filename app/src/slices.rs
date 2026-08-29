use axum::Router;

use crate::state::AppState;

mod agents;
mod chat;
mod connect;
mod workflow_runs;
mod workflows;

pub(crate) use chat::{AgentOutcome, AgentRunSpec, run_agent_action};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .merge(connect::router())
        .merge(agents::router())
        .merge(chat::router())
        .merge(workflow_runs::router())
        .merge(workflows::router())
}
