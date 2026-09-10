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
    #[serde(default)]
    conversation: Option<String>,
}

async fn show(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Query(query): Query<Selection>,
) -> AppResult<Response> {
    // Return paths derive from validated records only. Arbitrary return
    // URLs are never honoured, so forged context falls back safely.
    let conversation = query
        .conversation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(crate::conversations::ConversationId::parse)
        .filter(|id| state.conversations.get(id).is_some());
    let view = page::AttentionPage::new(&state, query.page, conversation);
    match graft {
        PageGraft::Document => responses::chat_page_response("Needs your attention", &state, &view),
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            "Needs your attention",
            "chat-main",
            &view,
        )?),
    }
}
