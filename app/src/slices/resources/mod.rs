mod page;

#[cfg(test)]
mod tests;

use crate::{error::AppResult, responses, sessions::OptionalSession, state::AppState};
use axum::{Router, extract::State, response::Response, routing::get};
use hypergraft::PageGraft;

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/resources", get(show))
}

async fn show(
    State(state): State<AppState>,
    _session: OptionalSession,
    graft: PageGraft,
) -> AppResult<Response> {
    match graft {
        PageGraft::Document => {
            responses::chat_page_response("Workflows and presets", &state, &page::ResourcesPage)
        }
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            "Workflows and presets",
            "chat-main",
            &page::ResourcesPage,
        )?),
    }
}
