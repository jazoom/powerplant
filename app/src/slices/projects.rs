mod desk;
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
use hypergraft::{GraftRequest, PageGraft, PatchGraft, PatchStatus};

use crate::{
    agents::{AgentDraft, AgentError, DirectoryGrant},
    error::{AppError, AppResult},
    projects::{ProjectError, ProjectId, ProjectRecord, desk_path, eligible_agents},
    responses,
    sessions::RequiredSession,
    state::AppState,
};

use self::{
    forms::{GrantForm, ProjectForm, REVISION_MESSAGE},
    page::{CatalogueView, DetailView, ProjectFormView},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(root))
        .route("/projects", get(catalogue).post(create))
        .route("/projects/new", get(new_project))
        .route("/projects/{project_id}", get(detail))
        .route(
            "/projects/{project_id}/configuration",
            get(show_configuration).post(update_configuration),
        )
        .route("/projects/{project_id}/agents/grant", post(grant_agent))
        .route(
            "/projects/{project_id}/agents/{agent_id}",
            get(desk::show).post(desk::send),
        )
        .route(
            "/projects/{project_id}/agents/{agent_id}/jobs/{job_id}/cancel",
            post(desk::cancel),
        )
}

async fn root(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    let destination = match state.projects.list().as_slice() {
        [] => "/projects/new".to_owned(),
        [project] => format!("/projects/{}", project.id.as_hex()),
        _ => "/projects".to_owned(),
    };
    Ok(responses::request_navigation(graft, &destination))
}

async fn catalogue(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PageGraft,
) -> AppResult<Response> {
    render_catalogue(&state, session.0, graft)
}

async fn new_project(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
) -> AppResult<Response> {
    render_form_page(
        &state,
        graft.into(),
        PatchStatus::Ok,
        page::NEW_TITLE,
        ProjectFormView::create("", "", ""),
    )
}

async fn create(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Form(form): Form<ProjectForm>,
) -> AppResult<Response> {
    let name = match form.submitted_name() {
        Ok(name) => name,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                ProjectFormView::create(&form.name, &form.path, error.message()),
            );
        }
    };
    let host_path = match form.submitted_path() {
        Ok(path) => path,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                ProjectFormView::create(&form.name, &form.path, error.message()),
            );
        }
    };
    match state.projects.create(name, host_path) {
        Ok(record) => Ok(responses::command_navigation(&format!(
            "/projects/{}",
            record.id.as_hex()
        ))),
        Err(error @ (ProjectError::Random | ProjectError::Persist | ProjectError::Corrupt)) => {
            Err(AppError::new("store project", error))
        }
        Err(error) => render_form_command(
            &state,
            graft,
            status_for(error),
            page::NEW_TITLE,
            ProjectFormView::create(&form.name, &form.path, error.message()),
        ),
    }
}

async fn detail(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PageGraft,
    Path(project_id): Path<String>,
) -> AppResult<Response> {
    let Some(record) = load_project(&state, &project_id) else {
        return Ok(responses::page_redirect(graft, "/projects"));
    };
    let eligible = eligible_agents(&state.agents.list(), &record);
    let remembered = state.sessions.last_agent(&session.0, &record.id);
    let remembered_eligible =
        remembered.filter(|agent_id| eligible.iter().any(|agent| agent.id == *agent_id));
    if remembered.is_some() && remembered_eligible.is_none() {
        state.sessions.forget_last_agent(&session.0, &record.id);
    }
    let destination = match eligible.as_slice() {
        [agent] => Some(desk_path(&record.id, &agent.id)),
        _ => remembered_eligible.map(|agent_id| desk_path(&record.id, &agent_id)),
    };
    if let Some(destination) = destination {
        return Ok(responses::page_redirect(graft, &destination));
    }
    let view = DetailView::from_record(&record, &eligible, &state.agents.list());
    render_detail_page(&state, graft, PatchStatus::Ok, &view)
}

async fn grant_agent(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(project_id): Path<String>,
    Form(form): Form<GrantForm>,
) -> AppResult<Response> {
    let Some(project) = load_project(&state, &project_id) else {
        return Ok(responses::command_navigation("/projects"));
    };
    let agent_id = match form.agent_id() {
        Ok(agent_id) => agent_id,
        Err(error) => {
            return render_grant_error(
                &state,
                graft,
                &project,
                &form,
                error,
                PatchStatus::UnprocessableEntity,
            );
        }
    };
    let revision = match form.revision() {
        Ok(revision) => revision,
        Err(error) => {
            return render_grant_error(
                &state,
                graft,
                &project,
                &form,
                error,
                PatchStatus::UnprocessableEntity,
            );
        }
    };
    let access = match form.access() {
        Ok(access) => access,
        Err(error) => {
            return render_grant_error(
                &state,
                graft,
                &project,
                &form,
                error,
                PatchStatus::UnprocessableEntity,
            );
        }
    };
    let Ok(_lease) = state.agent_leases.acquire(agent_id) else {
        return render_grant_error(
            &state,
            graft,
            &project,
            &form,
            "Wait until this reply finishes.",
            PatchStatus::UnprocessableEntity,
        );
    };
    let Some(project) = state.projects.get(&project.id) else {
        return Ok(responses::command_navigation("/projects"));
    };
    let Some(agent) = state.agents.get(&agent_id) else {
        return render_grant_error(
            &state,
            graft,
            &project,
            &form,
            AgentError::Missing.message(),
            PatchStatus::UnprocessableEntity,
        );
    };
    if agent.revision != revision {
        return render_grant_error(
            &state,
            graft,
            &project,
            &form,
            AgentError::Conflict.message(),
            PatchStatus::Conflict,
        );
    }
    let mut directories = agent.directories.clone();
    directories.push(DirectoryGrant {
        alias: form.alias(),
        host_path: project.host_path.clone(),
        access,
    });
    let draft = AgentDraft {
        name: agent.name.clone(),
        instructions: agent.instructions.clone(),
        tools: agent.tools.clone(),
        directories,
        primary_directory: agent.primary_directory.clone(),
    };
    match state.agents.update(&agent.id, revision, draft) {
        Ok(updated) => Ok(responses::command_navigation(&desk_path(
            &project.id,
            &updated.id,
        ))),
        Err(error @ (AgentError::Random | AgentError::Persist | AgentError::Corrupt)) => {
            Err(AppError::new("store agent", error))
        }
        Err(AgentError::Missing) => Ok(responses::command_navigation(&format!(
            "/projects/{}",
            project.id.as_hex()
        ))),
        Err(error) => {
            let status = if error == AgentError::Conflict {
                PatchStatus::Conflict
            } else {
                PatchStatus::UnprocessableEntity
            };
            render_grant_error(&state, graft, &project, &form, error.message(), status)
        }
    }
}

async fn show_configuration(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Path(project_id): Path<String>,
) -> AppResult<Response> {
    let Some(record) = load_project(&state, &project_id) else {
        return Ok(responses::page_redirect(graft, "/projects"));
    };
    render_form_page(
        &state,
        graft.into(),
        PatchStatus::Ok,
        page::CONFIG_TITLE,
        ProjectFormView::edit(&record, &record.name, ""),
    )
}

async fn update_configuration(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(project_id): Path<String>,
    Form(form): Form<ProjectForm>,
) -> AppResult<Response> {
    let Some(record) = load_project(&state, &project_id) else {
        return Ok(responses::command_navigation("/projects"));
    };
    let revision = match form.revision() {
        Ok(revision) => revision,
        Err(_) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                ProjectFormView::edit(&record, &form.name, REVISION_MESSAGE),
            );
        }
    };
    let name = match form.submitted_name() {
        Ok(name) => name,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                ProjectFormView::edit(&record, &form.name, error.message()),
            );
        }
    };
    match state.projects.update_name(&record.id, revision, name) {
        Ok(updated) => render_form_command(
            &state,
            graft,
            PatchStatus::Ok,
            page::CONFIG_TITLE,
            ProjectFormView::edit(&updated, &updated.name, ""),
        ),
        Err(error) => render_configuration_error(&state, graft, record, form.name, error),
    }
}

fn load_project(state: &AppState, raw: &str) -> Option<ProjectRecord> {
    ProjectId::parse(raw).and_then(|id| state.projects.get(&id))
}

fn render_detail_page(
    state: &AppState,
    graft: PageGraft,
    status: PatchStatus,
    view: &DetailView,
) -> AppResult<Response> {
    match graft {
        PageGraft::Document => {
            let mut response = responses::chat_page_response(&view.document_title, state, view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            &view.document_title,
            "chat-main",
            view,
        )?),
    }
}

fn render_grant_error(
    state: &AppState,
    _graft: PatchGraft,
    project: &ProjectRecord,
    form: &GrantForm,
    error: &'static str,
    status: PatchStatus,
) -> AppResult<Response> {
    let latest = state
        .projects
        .get(&project.id)
        .unwrap_or_else(|| project.clone());
    let agents = state.agents.list();
    let eligible = eligible_agents(&agents, &latest);
    let view = DetailView::with_grant(
        &latest,
        &eligible,
        &agents,
        &form.alias(),
        &form.access,
        error,
    );
    Ok(hypergraft::outcome::children_patch(
        status,
        "chat-main",
        &view,
    )?)
}

fn status_for(error: ProjectError) -> PatchStatus {
    match error {
        ProjectError::Conflict | ProjectError::Missing => PatchStatus::Conflict,
        _ => PatchStatus::UnprocessableEntity,
    }
}

fn render_configuration_error(
    state: &AppState,
    graft: PatchGraft,
    record: ProjectRecord,
    submitted_name: String,
    error: ProjectError,
) -> AppResult<Response> {
    if matches!(error, ProjectError::Missing) {
        return Ok(responses::command_navigation("/projects"));
    }
    if matches!(
        error,
        ProjectError::Persist | ProjectError::Random | ProjectError::Corrupt
    ) {
        return Err(AppError::new("store project", error));
    }
    let status = status_for(error);
    let (latest, name) = match error {
        ProjectError::Conflict => {
            let latest = state.projects.get(&record.id).unwrap_or(record);
            let name = latest.name.clone();
            (latest, name)
        }
        _ => (record, submitted_name),
    };
    render_form_command(
        state,
        graft,
        status,
        page::CONFIG_TITLE,
        ProjectFormView::edit(&latest, &name, error.message()),
    )
}

fn render_catalogue(
    state: &AppState,
    session: crate::sessions::SessionId,
    graft: PageGraft,
) -> AppResult<Response> {
    let view = CatalogueView::from_records(&ordered_projects(state, session));
    match graft {
        PageGraft::Document => {
            let mut response = responses::chat_page_response(page::INDEX_TITLE, state, &view)?;
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

fn render_form_page(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    title: &str,
    view: ProjectFormView,
) -> AppResult<Response> {
    match graft {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(title, state, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(title, "chat-main", &view)?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "project-form",
            &view.contents(),
        )?),
    }
}

fn render_form_command(
    _state: &AppState,
    _graft: PatchGraft,
    status: PatchStatus,
    _title: &str,
    view: ProjectFormView,
) -> AppResult<Response> {
    Ok(hypergraft::outcome::children_patch(
        status,
        "project-form",
        &view.contents(),
    )?)
}

fn ordered_projects(state: &AppState, session: crate::sessions::SessionId) -> Vec<ProjectRecord> {
    let recent = state.sessions.recent_projects(&session);
    let mut listed = state.projects.list();
    listed.sort_by(|left, right| {
        match (
            recent.iter().position(|id| *id == left.id),
            recent.iter().position(|id| *id == right.id),
        ) {
            (Some(left_rank), Some(right_rank)) => left_rank.cmp(&right_rank),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
    listed
}
