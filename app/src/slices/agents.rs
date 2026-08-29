mod forms;
mod page;

#[cfg(test)]
mod tests;

use axum::{
    Form, Router,
    extract::{Path, State},
    response::Response,
    routing::{get, post},
};
use hypergraft::{CommandGraft, GraftRequest, PatchStatus};

use crate::{
    agents::{AgentId, AgentRecord},
    error::AppResult,
    responses,
    sessions::{OptionalSession, SessionId},
    state::AppState,
};

use self::{
    forms::{AgentForm, OrphanForm},
    page::{AgentFormView, CatalogueView},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(root))
        .route("/agents", get(catalogue).post(create))
        .route("/agents/new", get(new_agent))
        .route("/agents/orphans/remove", post(remove_orphan))
        .route(
            "/agents/{agent_id}/configuration",
            get(show_configuration).post(update_configuration),
        )
        .route("/agents/{agent_id}/delete", post(delete_agent))
}

async fn root(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    if session.is_none() || !state.vault.has_providers() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let agents = state.agents.list();
    let destination = match agents.as_slice() {
        [agent] => format!("/agents/{}", agent.id.as_hex()),
        _ => "/agents".to_owned(),
    };
    Ok(responses::graft_redirect(graft, &destination))
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

async fn catalogue(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    render_catalogue(&state, graft, PatchStatus::Ok, "")
}

async fn new_agent(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    render_form(
        &state,
        graft,
        PatchStatus::Ok,
        page::NEW_TITLE,
        AgentFormView::create(""),
    )
}

async fn create(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(form): Form<AgentForm>,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let draft = match form.draft() {
        Ok(draft) => draft,
        Err(error) => {
            return render_form(
                &state,
                graft.into(),
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                AgentFormView::create(error.message()),
            );
        }
    };
    match state.agents.create(draft) {
        Ok(record) => Ok(responses::graft_redirect(
            graft,
            &format!("/agents/{}", record.id.as_hex()),
        )),
        Err(error) => render_form(
            &state,
            graft.into(),
            PatchStatus::UnprocessableEntity,
            page::NEW_TITLE,
            AgentFormView::create(error.message()),
        ),
    }
}

async fn show_configuration(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
    Path(agent_id): Path<String>,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let Some(record) = load_agent(&state, &agent_id) else {
        return Ok(responses::graft_redirect(graft, "/agents"));
    };
    render_form(
        &state,
        graft,
        PatchStatus::Ok,
        page::CONFIG_TITLE,
        AgentFormView::edit(&record, ""),
    )
}

async fn update_configuration(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Path(agent_id): Path<String>,
    Form(form): Form<AgentForm>,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let Some(record) = load_agent(&state, &agent_id) else {
        return Ok(responses::graft_redirect(graft, "/agents"));
    };
    let Ok(_operation) = state.agent_leases.acquire(record.id) else {
        return render_form(
            &state,
            graft.into(),
            PatchStatus::UnprocessableEntity,
            page::CONFIG_TITLE,
            AgentFormView::edit(&record, "Wait until this reply finishes."),
        );
    };
    let draft = match form.draft() {
        Ok(draft) => draft,
        Err(error) => {
            return render_form(
                &state,
                graft.into(),
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                AgentFormView::edit(&record, error.message()),
            );
        }
    };
    match state.agents.update(&record.id, draft) {
        Ok(updated) => render_form(
            &state,
            graft.into(),
            PatchStatus::Ok,
            page::CONFIG_TITLE,
            AgentFormView::edit(&updated, ""),
        ),
        Err(error) => render_form(
            &state,
            graft.into(),
            PatchStatus::UnprocessableEntity,
            page::CONFIG_TITLE,
            AgentFormView::edit(&record, error.message()),
        ),
    }
}

async fn delete_agent(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Path(agent_id): Path<String>,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let Some(record) = load_agent(&state, &agent_id) else {
        return Ok(responses::graft_redirect(graft, "/agents"));
    };
    let Ok(_operation) = state.agent_leases.acquire(record.id) else {
        return render_form(
            &state,
            graft.into(),
            PatchStatus::UnprocessableEntity,
            page::CONFIG_TITLE,
            AgentFormView::edit(&record, "Wait until this reply finishes."),
        );
    };
    if let Err(error) = state.agents.delete(&record.id) {
        return render_catalogue(
            &state,
            graft.into(),
            PatchStatus::UnprocessableEntity,
            error.message(),
        );
    }
    Ok(responses::graft_redirect(graft, "/agents"))
}

async fn remove_orphan(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(form): Form<OrphanForm>,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let error = match state.sandboxes.remove_orphan(form.name.trim()).await {
        Ok(()) => "",
        Err(error) => error.message(),
    };
    let status = if error.is_empty() {
        PatchStatus::Ok
    } else {
        PatchStatus::UnprocessableEntity
    };
    render_catalogue(&state, graft.into(), status, error)
}

fn load_agent(state: &AppState, raw: &str) -> Option<AgentRecord> {
    AgentId::parse(raw).and_then(|id| state.agents.get(&id))
}

fn render_catalogue(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    error: &'static str,
) -> AppResult<Response> {
    let view = CatalogueView::from_parts(&state.agents.list(), state.sandboxes.orphans(), error);
    render_desk(state, graft, status, page::CATALOGUE_TITLE, &view)
}

fn render_form(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    title: &str,
    view: AgentFormView,
) -> AppResult<Response> {
    render_desk(state, graft, status, title, &view)
}

fn render_desk<T: askama::Template>(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    title: &str,
    view: &T,
) -> AppResult<Response> {
    match graft {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(title, &state.assets, view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(title, "chat-main", view)?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "chat-main",
            view,
        )?),
    }
}
