mod page;

#[cfg(test)]
mod tests;

use crate::{error::AppResult, responses, sessions::OptionalSession, state::AppState};
use axum::{
    Router,
    extract::{Query, State},
    response::Response,
    routing::get,
};
use hypergraft::PageGraft;
use serde::Deserialize;

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/resources", get(show))
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Selection {
    conversation: String,
    workflow: String,
    preset: String,
}

async fn show(
    State(state): State<AppState>,
    _session: OptionalSession,
    graft: PageGraft,
    Query(query): Query<Selection>,
) -> AppResult<Response> {
    // Return paths derive from validated records only. Arbitrary
    // identifiers fall back safely, and this GET creates no record,
    // starts no workflow and grants no access.
    let conversation = crate::conversations::ConversationId::parse(query.conversation.trim())
        .filter(|id| state.conversations.get(id).is_some());
    let view = page::ResourcesPage::new(&state, conversation, &query.workflow, &query.preset);
    match graft {
        PageGraft::Document => {
            responses::chat_page_response("Workflows and presets", &state, &view)
        }
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            "Workflows and presets",
            "chat-main",
            &view,
        )?),
    }
}
