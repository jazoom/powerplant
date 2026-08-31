use axum::Router;

use crate::state::AppState;

mod agents;
mod chat;
mod connect;
mod environments;
mod human_gates;
mod projects;
mod workflow_runs;
mod workflows;

pub(crate) use chat::{AgentOutcome, AgentRunSpec, bound_reply, run_agent_action};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .merge(connect::router())
        .merge(projects::router())
        .merge(agents::router())
        .merge(chat::router())
        .merge(workflow_runs::router())
        .merge(human_gates::router())
        .merge(workflows::router())
        .merge(environments::router())
}
