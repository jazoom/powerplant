use axum::Router;

use crate::state::AppState;

mod agents;
pub(crate) mod chat;
mod connect;
mod environments;
mod human_gates;
mod projects;
mod settings;
mod workflow_runs;
mod workflows;

#[cfg(test)]
mod tests;

pub(crate) use chat::{AgentOutcome, AgentRunSpec, bound_reply, run_agent_action};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .merge(connect::router())
        .merge(projects::router())
        .merge(agents::router())
        .merge(chat::router())
        .merge(settings::router())
        .merge(workflow_runs::router())
        .merge(human_gates::router())
        .merge(workflows::router())
        .merge(environments::router())
}

pub(crate) fn live_router() -> hypergraft::live::LiveRouter<AppState> {
    hypergraft::live::LiveRouter::new()
        .merge(chat::live_router())
        .expect("live projection paths are unique")
}
