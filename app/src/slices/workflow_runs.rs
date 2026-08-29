mod page;

#[cfg(test)]
mod tests;

use axum::{
    Router,
    extract::{Path, State},
    response::Response,
    routing::get,
};
use hypergraft::{GraftRequest, PageGraft, PatchStatus};

use crate::{
    error::AppResult,
    responses,
    sessions::{OptionalSession, SessionId},
    state::AppState,
    workflows::RunId,
};

use self::page::{RunDetailView, RunIndexView};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/runs", get(index))
        .route("/runs/{run_id}", get(detail))
}

async fn require_session(
    state: &AppState,
    session: Option<SessionId>,
    graft: impl Into<hypergraft::GraftRequest>,
) -> Result<SessionId, Response> {
    let graft = graft.into();
    let Some(session) = session else {
        return Err(responses::graft_redirect(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Err(responses::graft_redirect(graft, "/connect"));
    }
    Ok(session)
}

async fn index(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: PageGraft,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let view = RunIndexView::from_summaries(&state.workflow_runs.summaries());
    match graft {
        PageGraft::Document => {
            let mut response =
                responses::chat_page_response(page::INDEX_TITLE, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, PatchStatus::Ok);
            Ok(response)
        }
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            page::INDEX_TITLE,
            "chat-main",
            &view,
        )?),
    }
}

async fn detail(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
    Path(run_id): Path<String>,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let Some(id) = RunId::parse(&run_id) else {
        return Ok(responses::graft_redirect(graft, "/runs"));
    };
    let Some(run) = state.workflow_runs.get(&id) else {
        return Ok(responses::graft_redirect(graft, "/runs"));
    };
    let view = RunDetailView::from_run(&run, &state.workflows);
    match graft {
        GraftRequest::Document => {
            let mut response =
                responses::chat_page_response(page::DETAIL_TITLE, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, PatchStatus::Ok);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            page::DETAIL_TITLE,
            "chat-main",
            &view,
        )?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "run-detail",
            &view.contents(),
        )?),
    }
}
