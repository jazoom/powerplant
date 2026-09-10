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
    agents::{AgentDraft, AgentError, DirectoryGrant, StarterAgent},
    error::{AppError, AppResult},
    execution::FolderPick,
    local_data::HOST_PATH_RESET_PENDING,
    projects::{ProjectError, ProjectId, ProjectRecord, submitted_host_path, submitted_name},
    responses,
    sessions::RequiredSession,
    state::AppState,
};

use self::{
    forms::{ProjectForm, REVISION_MESSAGE},
    page::{CatalogueView, DetailView, ProjectFormView},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(root))
        .route("/projects", get(catalogue).post(create))
        .route("/projects/new", get(new_project))
        .route("/projects/folder", post(choose_folder))
        .route("/projects/{project_id}", get(detail))
        .route(
            "/projects/{project_id}/configuration",
            get(show_configuration).post(update_configuration),
        )
        .route("/projects/{project_id}/agents/grant", post(grant_agent))
        .route(
            "/projects/{project_id}/agents/starter",
            post(create_starter),
        )
}

async fn root(
    State(_state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    Ok(responses::request_navigation(graft, "/conversations"))
}

async fn catalogue(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
) -> AppResult<Response> {
    render_catalogue(&state, graft)
}

async fn new_project(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
) -> AppResult<Response> {
    let view = ProjectFormView::create("", "", "");
    render_form_page(&state, graft.into(), PatchStatus::Ok, page::NEW_TITLE, view)
}

async fn choose_folder(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Form(form): Form<ProjectForm>,
) -> AppResult<Response> {
    match state.folder_picker.pick().await {
        FolderPick::Busy => render_form_command(
            &state,
            graft,
            PatchStatus::Conflict,
            page::NEW_TITLE,
            create_form(&form, forms::CHOOSER_BUSY),
        ),
        FolderPick::Cancelled => render_form_command(
            &state,
            graft,
            PatchStatus::Ok,
            page::NEW_TITLE,
            create_form(&form, ""),
        ),
        FolderPick::Selected(path) => match submitted_host_path(&path) {
            Ok(path) => {
                let name = if form.name.is_empty() {
                    derived_project_name(&path)
                } else {
                    form.name.clone()
                };
                let presented = path.to_string_lossy().into_owned();
                render_form_command(
                    &state,
                    graft,
                    PatchStatus::Ok,
                    page::NEW_TITLE,
                    ProjectFormView::create(&name, &presented, ""),
                )
            }
            Err(error) => render_form_command(
                &state,
                graft,
                status_for(error),
                page::NEW_TITLE,
                create_form(&form, error.message()),
            ),
        },
    }
}

async fn create(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Form(form): Form<ProjectForm>,
) -> AppResult<Response> {
    let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
        return render_form_command(
            &state,
            graft,
            PatchStatus::Conflict,
            page::NEW_TITLE,
            create_form(&form, HOST_PATH_RESET_PENDING),
        );
    };
    let name = match form.submitted_name() {
        Ok(name) => name,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                create_form(&form, error.message()),
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
                create_form(&form, error.message()),
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
            create_form(&form, error.message()),
        ),
    }
}

async fn detail(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Path(project_id): Path<String>,
) -> AppResult<Response> {
    let Some(record) = load_project(&state, &project_id) else {
        return Ok(responses::page_redirect(graft, "/projects"));
    };
    let view = DetailView::with_conversations(&record, &state.conversations.list());
    render_detail_page(&state, graft, PatchStatus::Ok, &view)
}

async fn grant_agent(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(project_id): Path<String>,
    Form(form): Form<forms::GrantForm>,
) -> AppResult<Response> {
    let Some(project) = load_project(&state, &project_id) else {
        return Ok(responses::command_navigation("/projects"));
    };
    let agent_id = match form.agent_id() {
        Ok(agent_id) => agent_id,
        Err(error) => {
            return render_project_command_error(
                &state,
                graft,
                &project,
                error,
                PatchStatus::UnprocessableEntity,
            );
        }
    };
    let revision = match form.revision() {
        Ok(revision) => revision,
        Err(error) => {
            return render_project_command_error(
                &state,
                graft,
                &project,
                error,
                PatchStatus::UnprocessableEntity,
            );
        }
    };
    let access = match form.access() {
        Ok(access) => access,
        Err(error) => {
            return render_project_command_error(
                &state,
                graft,
                &project,
                error,
                PatchStatus::UnprocessableEntity,
            );
        }
    };
    let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
        return render_project_command_error(
            &state,
            graft,
            &project,
            HOST_PATH_RESET_PENDING,
            PatchStatus::Conflict,
        );
    };
    let Ok(_lease) = state.agent_leases.acquire(agent_id) else {
        return render_project_command_error(
            &state,
            graft,
            &project,
            "Wait until this reply finishes.",
            PatchStatus::UnprocessableEntity,
        );
    };
    let Some(project) = state.projects.get(&project.id) else {
        return Ok(responses::command_navigation("/projects"));
    };
    let Some(agent) = state.agents.get(&agent_id) else {
        return render_project_command_error(
            &state,
            graft,
            &project,
            AgentError::Missing.message(),
            PatchStatus::UnprocessableEntity,
        );
    };
    if agent.revision != revision {
        return render_project_command_error(
            &state,
            graft,
            &project,
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
        selection: agent.selection.clone(),
        tools: agent.tools.clone(),
        network: agent.network.clone(),
        directories,
        primary_directory: agent.primary_directory.clone(),
    };
    match state.agents.update(&agent.id, revision, draft) {
        Ok(_) => Ok(responses::command_navigation(&format!(
            "/projects/{}",
            project.id.as_hex()
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
            render_project_command_error(&state, graft, &project, error.message(), status)
        }
    }
}

async fn create_starter(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(project_id): Path<String>,
) -> AppResult<Response> {
    let Some(project) = load_project(&state, &project_id) else {
        return Ok(responses::command_navigation("/projects"));
    };
    let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
        return render_project_command_error(
            &state,
            graft,
            &project,
            HOST_PATH_RESET_PENDING,
            PatchStatus::Conflict,
        );
    };
    match state.agents.ensure_starter(&project) {
        Ok(StarterAgent::One(_) | StarterAgent::Created(_)) => Ok(responses::command_navigation(
            &format!("/projects/{}", project.id.as_hex()),
        )),
        Ok(StarterAgent::Several) => Ok(responses::command_navigation(&format!(
            "/projects/{}",
            project.id.as_hex()
        ))),
        Err(error @ (AgentError::Random | AgentError::Persist | AgentError::Corrupt)) => {
            Err(AppError::new("store agent", error))
        }
        Err(error) => render_project_command_error(
            &state,
            graft,
            &project,
            error.message(),
            PatchStatus::UnprocessableEntity,
        ),
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
        Ok(_) => Ok(responses::command_navigation("/projects")),
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

fn render_project_command_error(
    state: &AppState,
    _graft: PatchGraft,
    project: &ProjectRecord,
    error: &'static str,
    status: PatchStatus,
) -> AppResult<Response> {
    let latest = state
        .projects
        .get(&project.id)
        .unwrap_or_else(|| project.clone());
    let view = DetailView::with_error(&latest, &state.conversations.list(), error);
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

fn render_catalogue(state: &AppState, graft: PageGraft) -> AppResult<Response> {
    let projects = ordered_projects(state);
    let view = CatalogueView::from_records(&projects);
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

fn create_form(form: &ProjectForm, error: &'static str) -> ProjectFormView {
    ProjectFormView::create(&form.name, &form.path, error)
}

fn derived_project_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(submitted_name)
        .and_then(Result::ok)
        .unwrap_or_default()
}

fn ordered_projects(state: &AppState) -> Vec<ProjectRecord> {
    state.projects.list()
}
