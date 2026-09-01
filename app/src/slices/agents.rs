mod forms;
mod page;

#[cfg(test)]
mod tests;

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::Response,
    routing::{get, post},
};
use hypergraft::{CommandGraft, GraftRequest, PatchStatus};
use serde::Deserialize;

use crate::{
    agents::{AgentError, AgentId, AgentRecord},
    error::{AppError, AppResult},
    projects::{ProjectId, ProjectRecord, desk_path, unique_desk_path},
    responses,
    sessions::RequiredSession,
    state::AppState,
};

use self::{
    forms::{AgentFormState, DeleteForm, FormIntent, OrphanForm, REVISION_MESSAGE},
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

#[derive(Default, Deserialize)]
struct AgentQuery {
    #[serde(default)]
    project: String,
}

async fn root(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    let agents = state.agents.list();
    let projects = state.projects.list();
    let destination = match agents.as_slice() {
        [agent] => unique_desk_path(agent, &projects).unwrap_or_else(|| "/agents".to_owned()),
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
    Query(query): Query<AgentQuery>,
) -> AppResult<Response> {
    let starter = load_starter_project(&state, &query.project);
    if starter_project_is_missing(&query.project, starter.as_ref()) {
        return Ok(responses::graft_redirect(graft, "/projects"));
    }
    render_form_page(
        &state,
        graft,
        PatchStatus::Ok,
        page::NEW_TITLE,
        create_form_view(starter.as_ref(), starter_form(starter.as_ref()), ""),
    )
}

async fn create(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: CommandGraft,
    Query(query): Query<AgentQuery>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    let starter = load_starter_project(&state, &query.project);
    if starter_project_is_missing(&query.project, starter.as_ref()) {
        return Ok(responses::graft_redirect(graft, "/projects"));
    }
    let (mut form, intent) = match AgentFormState::parse(pairs) {
        Ok(parsed) => parsed,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                create_form_view(
                    starter.as_ref(),
                    starter_form(starter.as_ref()),
                    error.message(),
                ),
            );
        }
    };
    if intent != FormIntent::Save {
        if let Err(error) = form.apply(intent) {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                create_form_view(starter.as_ref(), form, error.message()),
            );
        }
        return render_form_command(
            &state,
            graft,
            PatchStatus::Ok,
            page::NEW_TITLE,
            create_form_view(starter.as_ref(), form, ""),
        );
    }
    if let Some(project) = &starter {
        // The project record owns this path. Submitted path_0 values cannot replace it.
        form.assign_project_path(&project.host_path);
    }
    let draft = match form.draft() {
        Ok(draft) => draft,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                create_form_view(starter.as_ref(), form, error.message()),
            );
        }
    };
    match state.agents.create(draft) {
        Ok(record) => {
            let destination = match &starter {
                Some(project) => desk_path(&project.id, &record.id),
                None => unique_desk_path(&record, &state.projects.list())
                    .unwrap_or_else(|| format!("/agents/{}/configuration", record.id.as_hex())),
            };
            Ok(responses::graft_redirect(graft, &destination))
        }
        Err(error @ (AgentError::Random | AgentError::Persist | AgentError::Corrupt)) => {
            Err(AppError::new("store agent", error))
        }
        Err(error) => render_form_command(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            page::NEW_TITLE,
            create_form_view(starter.as_ref(), form, error.message()),
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
    render_form_page(
        &state,
        graft,
        PatchStatus::Ok,
        page::CONFIG_TITLE,
        AgentFormView::edit(&record, AgentFormState::from_record(&record), ""),
    )
}

async fn update_configuration(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: CommandGraft,
    Path(agent_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    let Some(record) = load_agent(&state, &agent_id) else {
        return Ok(responses::graft_redirect(graft, "/agents"));
    };
    let (mut form, intent) = match AgentFormState::parse(pairs) {
        Ok(parsed) => parsed,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                AgentFormView::edit(
                    &record,
                    AgentFormState::from_record(&record),
                    error.message(),
                ),
            );
        }
    };
    if intent != FormIntent::Save {
        if let Err(error) = form.apply(intent) {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                AgentFormView::edit(&record, form, error.message()),
            );
        }
        return render_form_command(
            &state,
            graft,
            PatchStatus::Ok,
            page::CONFIG_TITLE,
            AgentFormView::edit(&record, form, ""),
        );
    }
    let Ok(_operation) = state.agent_leases.acquire(record.id) else {
        return render_form_command(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            page::CONFIG_TITLE,
            AgentFormView::edit(&record, form, "Wait until this reply finishes."),
        );
    };
    let revision = match form.revision() {
        Ok(Some(revision)) => revision,
        Ok(None) | Err(_) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                AgentFormView::edit(&record, form, REVISION_MESSAGE),
            );
        }
    };
    let draft = match form.draft() {
        Ok(draft) => draft,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                AgentFormView::edit(&record, form, error.message()),
            );
        }
    };
    match state.agents.update(&record.id, revision, draft) {
        Ok(updated) => render_form_command(
            &state,
            graft,
            PatchStatus::Ok,
            page::CONFIG_TITLE,
            AgentFormView::edit(&updated, AgentFormState::from_record(&updated), ""),
        ),
        Err(error) => render_configuration_error(&state, graft, record, form, error),
    }
}

async fn delete_agent(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: CommandGraft,
    Path(agent_id): Path<String>,
    Form(form): Form<DeleteForm>,
) -> AppResult<Response> {
    let Some(record) = load_agent(&state, &agent_id) else {
        return Ok(responses::graft_redirect(graft, "/agents"));
    };
    let Ok(_operation) = state.agent_leases.acquire(record.id) else {
        return render_form_command(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            page::CONFIG_TITLE,
            AgentFormView::edit(
                &record,
                AgentFormState::from_record(&record),
                "Wait until this reply finishes.",
            ),
        );
    };
    let revision = match form.revision() {
        Ok(Some(revision)) => revision,
        Ok(None) | Err(_) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                AgentFormView::edit(
                    &record,
                    AgentFormState::from_record(&record),
                    REVISION_MESSAGE,
                ),
            );
        }
    };
    match state.agents.delete(&record.id, revision) {
        Ok(()) => Ok(responses::graft_redirect(graft, "/agents")),
        Err(error) => {
            let form = AgentFormState::from_record(&record);
            render_configuration_error(&state, graft, record, form, error)
        }
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

fn load_starter_project(state: &AppState, raw: &str) -> Option<ProjectRecord> {
    ProjectId::parse(raw).and_then(|id| state.projects.get(&id))
}

fn starter_project_is_missing(raw: &str, loaded: Option<&ProjectRecord>) -> bool {
    !raw.is_empty() && loaded.is_none()
}

fn starter_form(project: Option<&ProjectRecord>) -> AgentFormState {
    match project {
        Some(record) => AgentFormState::for_project(&record.name),
        None => AgentFormState::blank(),
    }
}

fn create_form_view(
    project: Option<&ProjectRecord>,
    form: AgentFormState,
    error: &'static str,
) -> AgentFormView {
    match project {
        Some(record) => AgentFormView::create_for_project(form, error, record),
        None => AgentFormView::create(form, error, ""),
    }
}

fn render_configuration_error(
    state: &AppState,
    graft: CommandGraft,
    record: AgentRecord,
    form: AgentFormState,
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
    let view = match error {
        AgentError::Conflict => {
            let latest = state.agents.get(&record.id).unwrap_or(record);
            AgentFormView::edit(
                &latest,
                AgentFormState::from_record(&latest),
                error.message(),
            )
        }
        _ => AgentFormView::edit(&record, form, error.message()),
    };
    render_form_command(state, graft, status, page::CONFIG_TITLE, view)
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

fn render_form_page(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    title: &str,
    view: AgentFormView,
) -> AppResult<Response> {
    match graft {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(title, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(title, "chat-main", &view)?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "agent-form",
            &view.contents(),
        )?),
    }
}

fn render_form_command(
    state: &AppState,
    graft: CommandGraft,
    status: PatchStatus,
    title: &str,
    view: AgentFormView,
) -> AppResult<Response> {
    match graft {
        CommandGraft::Document => {
            let mut response = responses::chat_page_response(title, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "agent-form",
            &view.contents(),
        )?),
    }
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
