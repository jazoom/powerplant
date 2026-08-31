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
    error::AppResult, responses, sessions::RequiredSession, state::AppState, workflows::RunId,
};

use self::page::{ArtefactView, RunDetailView, RunIndexView};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/runs", get(index))
        .route("/runs/{run_id}", get(detail))
        .route("/runs/{run_id}/artefacts/{artefact_id}", get(artefact))
}

async fn index(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
) -> AppResult<Response> {
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
    _session: RequiredSession,
    graft: GraftRequest,
    Path(run_id): Path<String>,
) -> AppResult<Response> {
    let Some(id) = RunId::parse(&run_id) else {
        return Ok(responses::graft_redirect(graft, "/runs"));
    };
    let Some(run) = state.workflow_runs.get(&id) else {
        return Ok(responses::graft_redirect(graft, "/runs"));
    };
    let view = RunDetailView::from_run(&run, &state.workflows, &state.environments);
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

async fn artefact(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Path((run_id, artefact_id)): Path<(String, String)>,
) -> AppResult<Response> {
    let Some(run_id) = RunId::parse(&run_id) else {
        return Ok(responses::graft_redirect(graft, "/runs"));
    };
    let Some(artefact_id) = crate::workflows::ArtefactId::parse(&artefact_id) else {
        return Ok(responses::graft_redirect(graft, "/runs"));
    };
    let Some(run) = state.workflow_runs.get(&run_id) else {
        return Ok(responses::graft_redirect(graft, "/runs"));
    };
    let Some(record) = run.artefact(&artefact_id) else {
        return Ok(responses::graft_redirect(
            graft,
            &format!("/runs/{}", run.id.as_hex()),
        ));
    };
    let view = ArtefactView::from_record(&run, record, &state);
    match graft {
        PageGraft::Document => {
            let mut response =
                responses::chat_page_response(page::ARTEFACT_TITLE, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, PatchStatus::Ok);
            Ok(response)
        }
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            page::ARTEFACT_TITLE,
            "chat-main",
            &view,
        )?),
    }
}
