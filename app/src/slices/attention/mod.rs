pub(crate) mod page;

#[cfg(test)]
mod tests;

use crate::{error::AppResult, responses, sessions::RequiredSession, state::AppState};
use axum::{
    Router,
    extract::{Query, State},
    response::Response,
    routing::get,
};
use hypergraft::PageGraft;

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/attention", get(show))
}

#[derive(Default, serde::Deserialize)]
struct Selection {
    #[serde(default)]
    page: usize,
}

async fn show(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Query(query): Query<Selection>,
) -> AppResult<Response> {
    let view = page::AttentionPage::new(&state, query.page);
    match graft {
        PageGraft::Document => responses::chat_page_response("Needs your attention", &state, &view),
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            "Needs your attention",
            "chat-main",
            &view,
        )?),
    }
}
