mod forms;
mod page;

#[cfg(test)]
mod tests;

use axum::{
    Form, Router,
    extract::{Path, State},
    response::Response,
    routing::get,
};
use hypergraft::{CommandGraft, GraftRequest, PageGraft, PatchStatus};

use crate::{
    error::{AppError, AppResult},
    projects::{ProjectError, ProjectId, ProjectRecord},
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
        .route("/projects", get(catalogue).post(create))
        .route("/projects/new", get(new_project))
        .route("/projects/{project_id}", get(detail))
        .route(
            "/projects/{project_id}/configuration",
            get(show_configuration).post(update_configuration),
        )
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
    graft: CommandGraft,
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
        Ok(record) => Ok(responses::graft_redirect(
            graft,
            &format!("/projects/{}", record.id.as_hex()),
        )),
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
    _session: RequiredSession,
    graft: PageGraft,
    Path(project_id): Path<String>,
) -> AppResult<Response> {
    let Some(record) = load_project(&state, &project_id) else {
        return Ok(responses::graft_redirect(graft, "/projects"));
    };
    let view = DetailView::from_record(&record);
    match graft {
        PageGraft::Document => {
            let mut response =
                responses::chat_page_response(&view.document_title, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, PatchStatus::Ok);
            Ok(response)
        }
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            &view.document_title,
            "chat-main",
            &view,
        )?),
    }
}

async fn show_configuration(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Path(project_id): Path<String>,
) -> AppResult<Response> {
    let Some(record) = load_project(&state, &project_id) else {
        return Ok(responses::graft_redirect(graft, "/projects"));
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
    graft: CommandGraft,
    Path(project_id): Path<String>,
    Form(form): Form<ProjectForm>,
) -> AppResult<Response> {
    let Some(record) = load_project(&state, &project_id) else {
        return Ok(responses::graft_redirect(graft, "/projects"));
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

fn status_for(error: ProjectError) -> PatchStatus {
    match error {
        ProjectError::Conflict | ProjectError::Missing => PatchStatus::Conflict,
        _ => PatchStatus::UnprocessableEntity,
    }
}

fn render_configuration_error(
    state: &AppState,
    graft: CommandGraft,
    record: ProjectRecord,
    submitted_name: String,
    error: ProjectError,
) -> AppResult<Response> {
    if matches!(error, ProjectError::Missing) {
        return Ok(responses::graft_redirect(graft, "/projects"));
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
    let view = CatalogueView::from_records(&state.projects.list());
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

fn render_form_page(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    title: &str,
    view: ProjectFormView,
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
            "project-form",
            &view.contents(),
        )?),
    }
}

fn render_form_command(
    state: &AppState,
    graft: CommandGraft,
    status: PatchStatus,
    title: &str,
    view: ProjectFormView,
) -> AppResult<Response> {
    match graft {
        CommandGraft::Document => {
            let mut response = responses::chat_page_response(title, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "project-form",
            &view.contents(),
        )?),
    }
}
