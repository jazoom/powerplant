use axum::{
    extract::{Path, State},
    response::Response,
};
use hypergraft::{GraftRequest, PatchStatus};

use crate::{error::AppResult, responses, sessions::RequiredSession, state::AppState};

use super::{detail_view, load_conversation, render_detail};

pub(super) async fn show(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: GraftRequest,
    Path(conversation_id): Path<String>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::request_navigation(graft, "/conversations"));
    };
    let view = attach_activity(
        detail_view(&state, session.0, &record, &record.title, ""),
        &state,
    )?;
    render_detail(&state, session.0, graft, PatchStatus::Ok, view)
}

fn attach_activity(
    view: super::page::ConversationDetailView,
    state: &AppState,
) -> AppResult<super::page::ConversationDetailView> {
    let html = view
        .render_activity(state)
        .map_err(|error| crate::error::AppError::new("render activity companion", error))?;
    Ok(view.with_companion_titled(html, "activity", "Activity and evidence"))
}

#[cfg(test)]
mod tests;
