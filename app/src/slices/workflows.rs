mod forms;
mod page;

#[cfg(test)]
mod tests;

use axum::{
    Form, Router,
    extract::{DefaultBodyLimit, Path, State},
    response::Response,
    routing::{get, post},
};
use hypergraft::{CommandGraft, PageGraft, PatchStatus};

use crate::{
    error::AppResult,
    responses,
    sessions::{OptionalSession, SessionId},
    state::AppState,
    workflows::{CatalogueError, WorkflowId, WorkflowRecord},
};

use crate::environments::SnapshotAvailability;

use self::{
    forms::{FormError, FormErrors, FormIntent, WorkflowFormState, parse_delete},
    page::{CatalogueView, WorkflowFormView},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/workflows", get(catalogue).post(create))
        .route("/workflows/new", get(new_workflow))
        .route(
            "/workflows/{workflow_id}/configuration",
            get(show_configuration).post(update_configuration),
        )
        .route("/workflows/{workflow_id}/delete", post(delete_workflow))
        .layer(DefaultBodyLimit::max(forms::MAXIMUM_FORM_BYTES))
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
    graft: PageGraft,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    render_catalogue(&state, graft)
}

async fn new_workflow(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: PageGraft,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    render_form_page(
        &state,
        graft.into(),
        PatchStatus::Ok,
        page::NEW_TITLE,
        WorkflowFormView::create(WorkflowFormState::blank(), FormErrors::default()),
    )
    .await
}

async fn create(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let (mut form, intent) = match WorkflowFormState::parse(pairs) {
        Ok(parsed) => parsed,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                WorkflowFormView::create(
                    WorkflowFormState::blank(),
                    FormErrors::summary(error.message()),
                ),
            )
            .await;
        }
    };
    if intent != FormIntent::Save {
        if let Err(error) = form.apply(intent) {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                WorkflowFormView::create(form, FormErrors::summary(error.message())),
            )
            .await;
        }
        return render_form_command(
            &state,
            graft,
            PatchStatus::Ok,
            page::NEW_TITLE,
            WorkflowFormView::create(form, FormErrors::default()),
        )
        .await;
    }
    let definition = match form.to_definition() {
        Ok(definition) => definition,
        Err(errors) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                WorkflowFormView::create(form, errors),
            )
            .await;
        }
    };
    if let Some(errors) = reject_unready_environments(&state, &form, &definition).await {
        return render_form_command(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            page::NEW_TITLE,
            WorkflowFormView::create(form, errors),
        )
        .await;
    }
    match state.workflows.create(definition) {
        Ok(record) => Ok(responses::graft_redirect(
            graft,
            &format!("/workflows/{}/configuration", record.id.as_hex()),
        )),
        Err(error) => {
            render_form_command(
                &state,
                graft,
                status_for(error),
                page::NEW_TITLE,
                WorkflowFormView::create(form, error.into()),
            )
            .await
        }
    }
}

async fn show_configuration(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: PageGraft,
    Path(workflow_id): Path<String>,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let Some(record) = load_workflow(&state, &workflow_id) else {
        return Ok(responses::graft_redirect(graft, "/workflows"));
    };
    render_form_page(
        &state,
        graft.into(),
        PatchStatus::Ok,
        page::CONFIG_TITLE,
        WorkflowFormView::edit_state(&record, FormErrors::default(), ""),
    )
    .await
}

async fn update_configuration(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Path(workflow_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let Some(record) = load_workflow(&state, &workflow_id) else {
        return Ok(responses::graft_redirect(graft, "/workflows"));
    };
    let (mut form, intent) = match WorkflowFormState::parse(pairs) {
        Ok(parsed) => parsed,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                WorkflowFormView::edit_state(&record, FormErrors::summary(error.message()), ""),
            )
            .await;
        }
    };
    form.maintain_candidate_outputs_from(&WorkflowFormState::from_record(&record));
    if intent != FormIntent::Save {
        if let Err(error) = form.apply(intent) {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                WorkflowFormView::edit(&record, form, FormErrors::summary(error.message()), ""),
            )
            .await;
        }
        return render_form_command(
            &state,
            graft,
            PatchStatus::Ok,
            page::CONFIG_TITLE,
            WorkflowFormView::edit(&record, form, FormErrors::default(), ""),
        )
        .await;
    }
    let Some(revision) = form.revision else {
        return render_form_command(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            page::CONFIG_TITLE,
            WorkflowFormView::edit(
                &record,
                form,
                FormErrors::summary(FormError::Revision.message()),
                "",
            ),
        )
        .await;
    };
    let definition = match form.to_definition() {
        Ok(definition) => definition,
        Err(errors) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                WorkflowFormView::edit(&record, form, errors, ""),
            )
            .await;
        }
    };
    if let Some(errors) = reject_unready_environments(&state, &form, &definition).await {
        return render_form_command(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            page::CONFIG_TITLE,
            WorkflowFormView::edit(&record, form, errors, ""),
        )
        .await;
    }
    match state.workflows.update(&record.id, revision, definition) {
        Ok(updated) => {
            render_form_command(
                &state,
                graft,
                PatchStatus::Ok,
                page::CONFIG_TITLE,
                WorkflowFormView::edit_state(&updated, FormErrors::default(), ""),
            )
            .await
        }
        Err(error) => {
            render_form_command(
                &state,
                graft,
                status_for(error),
                page::CONFIG_TITLE,
                WorkflowFormView::edit(&record, form, error.into(), ""),
            )
            .await
        }
    }
}

async fn delete_workflow(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Path(workflow_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    if require_session(&state, session, graft).await.is_err() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let Some(record) = load_workflow(&state, &workflow_id) else {
        return Ok(responses::graft_redirect(graft, "/workflows"));
    };
    let (revision, confirmed) = match parse_delete(&pairs) {
        Ok(parsed) => parsed,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                WorkflowFormView::edit_state(&record, FormErrors::default(), error.message()),
            )
            .await;
        }
    };
    if !confirmed {
        return render_form_command(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            page::CONFIG_TITLE,
            WorkflowFormView::edit_state(
                &record,
                FormErrors::default(),
                "Tick the box to delete this workflow.",
            ),
        )
        .await;
    }
    match state.workflows.delete(&record.id, revision) {
        Ok(()) => Ok(responses::graft_redirect(graft, "/workflows")),
        Err(error) => {
            render_form_command(
                &state,
                graft,
                status_for(error),
                page::CONFIG_TITLE,
                WorkflowFormView::edit_state(&record, FormErrors::default(), error.message()),
            )
            .await
        }
    }
}

async fn reject_unready_environments(
    state: &AppState,
    form: &WorkflowFormState,
    definition: &crate::workflows::definition::WorkflowDefinition,
) -> Option<FormErrors> {
    let mut errors = FormErrors::for_state(form);
    errors.summary = "Choose a ready environment.";
    let mut invalid = false;
    let mut ready = Vec::new();
    for record in state.environments.list() {
        let Some(id) = record.ready_preparation else {
            continue;
        };
        let Some(preparation) = state.environments.preparation(&id) else {
            continue;
        };
        let Some(snapshot) = preparation.snapshot else {
            continue;
        };
        if state.environment_snapshots.inspect(&snapshot).await != SnapshotAvailability::Available {
            continue;
        }
        ready.push(record.id);
    }
    let default = definition.default_environment();
    if !ready.contains(&default) {
        errors.default_environment = "Choose a ready environment.";
        invalid = true;
    }
    for (index, step) in definition.steps().iter().enumerate() {
        if let Some(crate::workflows::definition::StepEnvironment::Override { environment_id }) =
            step.environment()
            && !ready.contains(&environment_id)
        {
            errors.steps[index].environment = "Choose a ready environment.";
            invalid = true;
        }
    }
    invalid.then_some(errors)
}

fn load_workflow(state: &AppState, raw: &str) -> Option<WorkflowRecord> {
    WorkflowId::parse(raw).and_then(|id| state.workflows.get(&id))
}

fn status_for(error: CatalogueError) -> PatchStatus {
    match error {
        CatalogueError::Conflict | CatalogueError::Missing => PatchStatus::Conflict,
        _ => PatchStatus::UnprocessableEntity,
    }
}

fn render_catalogue(state: &AppState, graft: PageGraft) -> AppResult<Response> {
    let view = CatalogueView::from_records(&state.workflows.list());
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

async fn attach_environments(state: &AppState, view: WorkflowFormView) -> WorkflowFormView {
    let mut options = Vec::new();
    for record in state.environments.list() {
        let Some(ready) = record.ready_preparation else {
            continue;
        };
        let Some(preparation) = state.environments.preparation(&ready) else {
            continue;
        };
        let Some(snapshot) = preparation.snapshot else {
            continue;
        };
        if state.environment_snapshots.inspect(&snapshot).await != SnapshotAvailability::Available {
            continue;
        }
        let selected = view.default_environment == record.id.as_hex();
        options.push(page::EnvironmentOption {
            id: record.id.as_hex(),
            name: record.name.clone(),
            context: format!(
                "{} · {}",
                record.recipe.oci_image.as_str(),
                snapshot.snapshot_digest.short_hex()
            ),
            selected,
        });
    }
    let no_ready = options.is_empty();
    view.with_environments(options, no_ready)
}

async fn render_form_page(
    state: &AppState,
    graft: hypergraft::GraftRequest,
    status: PatchStatus,
    title: &str,
    view: WorkflowFormView,
) -> AppResult<Response> {
    let view = attach_environments(state, view).await;
    match graft {
        hypergraft::GraftRequest::Document => {
            let mut response = responses::chat_page_response(title, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        hypergraft::GraftRequest::Navigation => {
            Ok(hypergraft::outcome::page_patch(title, "chat-main", &view)?)
        }
        hypergraft::GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "workflow-form",
            &view.contents(),
        )?),
    }
}

async fn render_form_command(
    state: &AppState,
    graft: CommandGraft,
    status: PatchStatus,
    title: &str,
    view: WorkflowFormView,
) -> AppResult<Response> {
    let view = attach_environments(state, view).await;
    match graft {
        CommandGraft::Document => {
            let mut response = responses::chat_page_response(title, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "workflow-form",
            &view.contents(),
        )?),
    }
}
