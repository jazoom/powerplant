mod page;

#[cfg(test)]
mod tests;

use axum::{Router, extract::State, response::Response, routing::get};
use hypergraft::PageGraft;

use crate::{error::AppResult, responses, sessions::RequiredSession, state::AppState};

use self::page::SettingsPage;

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/settings", get(show))
}

async fn show(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
) -> AppResult<Response> {
    let page = SettingsPage;
    match graft {
        PageGraft::Document => responses::chat_page_response(page::TITLE, &state.assets, &page),
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            page::TITLE,
            "chat-main",
            &page,
        )?),
    }
}
