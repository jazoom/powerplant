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
    agents::{AgentError, AgentId, AgentRecord},
    error::{AppError, AppResult},
    responses,
    sessions::RequiredSession,
    state::AppState,
};

use self::{
    forms::{AgentForm, OrphanForm, REVISION_MESSAGE},
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
    _session: RequiredSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    let agents = state.agents.list();
    let projects = state.projects.list();
    let destination = match agents.as_slice() {
        [agent] => crate::projects::unique_desk_path(agent, &projects)
            .unwrap_or_else(|| "/agents".to_owned()),
        _ => "/agents".to_owned(),
    };
    Ok(responses::graft_redirect(graft, &destination))
}

async fn catalogue(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    render_catalogue(&state, graft, PatchStatus::Ok, "")
}

async fn new_agent(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
) -> AppResult<Response> {
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
    _session: RequiredSession,
    graft: CommandGraft,
    Form(form): Form<AgentForm>,
) -> AppResult<Response> {
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
        Ok(record) => {
            let destination = crate::projects::unique_desk_path(&record, &state.projects.list())
                .unwrap_or_else(|| format!("/agents/{}/configuration", record.id.as_hex()));
            Ok(responses::graft_redirect(graft, &destination))
        }
        Err(error @ (AgentError::Random | AgentError::Persist | AgentError::Corrupt)) => {
            Err(AppError::new("store agent", error))
        }
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
    _session: RequiredSession,
    graft: GraftRequest,
    Path(agent_id): Path<String>,
) -> AppResult<Response> {
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
    _session: RequiredSession,
    graft: CommandGraft,
    Path(agent_id): Path<String>,
    Form(form): Form<AgentForm>,
) -> AppResult<Response> {
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
    let revision = match form.revision() {
        Ok(Some(revision)) => revision,
        Ok(None) | Err(_) => {
            return render_form(
                &state,
                graft.into(),
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                AgentFormView::edit(&record, REVISION_MESSAGE),
            );
        }
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
    match state.agents.update(&record.id, revision, draft) {
        Ok(updated) => render_form(
            &state,
            graft.into(),
            PatchStatus::Ok,
            page::CONFIG_TITLE,
            AgentFormView::edit(&updated, ""),
        ),
        Err(error) => render_configuration_error(&state, graft, record, error),
    }
}

async fn delete_agent(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: CommandGraft,
    Path(agent_id): Path<String>,
    Form(form): Form<AgentForm>,
) -> AppResult<Response> {
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
    let revision = match form.revision() {
        Ok(Some(revision)) => revision,
        Ok(None) | Err(_) => {
            return render_form(
                &state,
                graft.into(),
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                AgentFormView::edit(&record, REVISION_MESSAGE),
            );
        }
    };
    match state.agents.delete(&record.id, revision) {
        Ok(()) => Ok(responses::graft_redirect(graft, "/agents")),
        Err(error) => render_configuration_error(&state, graft, record, error),
    }
}

async fn remove_orphan(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: CommandGraft,
    Form(form): Form<OrphanForm>,
) -> AppResult<Response> {
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

fn render_configuration_error(
    state: &AppState,
    graft: CommandGraft,
    record: AgentRecord,
    error: AgentError,
) -> AppResult<Response> {
    if matches!(error, AgentError::Missing) {
        return Ok(responses::graft_redirect(graft, "/agents"));
    }
    let status = match error {
        AgentError::Persist | AgentError::Random | AgentError::Corrupt => {
            return Err(AppError::new("store agent", error));
        }
        AgentError::Conflict => PatchStatus::Conflict,
        _ => PatchStatus::UnprocessableEntity,
    };
    let latest = match error {
        AgentError::Conflict => state.agents.get(&record.id).unwrap_or(record),
        _ => record,
    };
    render_form(
        state,
        graft.into(),
        status,
        page::CONFIG_TITLE,
        AgentFormView::edit(&latest, error.message()),
    )
}

fn render_catalogue(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    error: &'static str,
) -> AppResult<Response> {
    let view = CatalogueView::from_parts(
        &state.agents.list(),
        &state.projects.list(),
        state.sandboxes.orphans(),
        error,
    );
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
