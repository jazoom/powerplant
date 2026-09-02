mod forms;
mod page;

#[cfg(test)]
mod tests;

use std::time::Duration;

use axum::{
    Form, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    response::Response,
    routing::{get, post},
};
use hypergraft::{GraftRequest, PageGraft, PatchGraft, PatchStatus};
use serde::Deserialize;

use crate::{
    environments::{EnvironmentError, EnvironmentId, EnvironmentRecord},
    error::AppResult,
    responses,
    sessions::RequiredSession,
    state::AppState,
};

use self::{
    forms::{EnvironmentFormState, FormError, FormErrors, parse_delete, parse_retry},
    page::{CatalogueView, EnvironmentFormView, EnvironmentStatusView},
};

const PREPARATION_HOLD: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(1)
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/environments", get(catalogue).post(create))
        .route("/environments/new", get(new_environment))
        .route(
            "/environments/{environment_id}/configuration",
            get(show_configuration).post(update_configuration),
        )
        .route(
            "/environments/{environment_id}/prepare",
            post(retry_preparation),
        )
        .route(
            "/environments/{environment_id}/delete",
            post(delete_environment),
        )
        .layer(DefaultBodyLimit::max(forms::MAXIMUM_FORM_BYTES))
}

#[derive(Default, Deserialize)]
struct RefreshQuery {
    #[serde(default)]
    cursor: String,
}

async fn catalogue(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
) -> AppResult<Response> {
    render_catalogue(&state, graft).await
}

async fn new_environment(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
) -> AppResult<Response> {
    render_form_page(
        &state,
        graft.into(),
        PatchStatus::Ok,
        page::NEW_TITLE,
        EnvironmentFormView::create(EnvironmentFormState::blank(), FormErrors::default()),
    )
}

async fn create(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    let form = match EnvironmentFormState::parse(pairs) {
        Ok(parsed) => parsed,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::NEW_TITLE,
                EnvironmentFormView::create(
                    EnvironmentFormState::blank(),
                    FormErrors::summary(error.message()),
                ),
            );
        }
    };
    match state.environments.create(form.to_draft()) {
        Ok((record, _)) => {
            state.environment_preparations.wake();
            Ok(responses::request_navigation(
                graft,
                &format!("/environments/{}/configuration", record.id.as_hex()),
            ))
        }
        Err(error) => render_form_command(
            &state,
            graft,
            status_for(error),
            page::NEW_TITLE,
            EnvironmentFormView::create(form, error.into()),
        ),
    }
}

async fn show_configuration(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
    Path(environment_id): Path<String>,
    Query(query): Query<RefreshQuery>,
) -> AppResult<Response> {
    let Some(record) = load_environment(&state, &environment_id) else {
        return Ok(responses::request_navigation(graft, "/environments"));
    };
    match graft {
        GraftRequest::Document => render_form_page(
            &state,
            graft,
            PatchStatus::Ok,
            page::CONFIG_TITLE,
            edit_view(&state, &record, FormErrors::default(), "").await?,
        ),
        GraftRequest::Navigation => render_form_page(
            &state,
            graft,
            PatchStatus::Ok,
            page::CONFIG_TITLE,
            edit_view(&state, &record, FormErrors::default(), "").await?,
        ),
        GraftRequest::Patch => refresh_preparation(&state, &record, query).await,
    }
}

async fn update_configuration(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(environment_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    let Some(record) = load_environment(&state, &environment_id) else {
        return Ok(responses::request_navigation(graft, "/environments"));
    };
    let form = match EnvironmentFormState::parse(pairs) {
        Ok(parsed) => parsed,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                edit_view(&state, &record, FormErrors::summary(error.message()), "").await?,
            );
        }
    };
    let Some(revision) = form.revision else {
        return render_form_command(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            page::CONFIG_TITLE,
            edit_view_with_state(
                &state,
                &record,
                form,
                FormErrors::summary(FormError::Revision.message()),
                "",
            )
            .await?,
        );
    };
    match state
        .environments
        .update(&record.id, revision, form.to_draft())
    {
        Ok(updated) => {
            if updated.preparation.is_some() {
                state.environment_preparations.wake();
            }
            Ok(responses::request_navigation(
                graft,
                &format!(
                    "/environments/{}/configuration",
                    updated.environment.id.as_hex()
                ),
            ))
        }
        Err(error) => render_form_command(
            &state,
            graft,
            status_for(error),
            page::CONFIG_TITLE,
            edit_view_with_state(&state, &record, form, error.into(), "").await?,
        ),
    }
}

async fn retry_preparation(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(environment_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    let Some(record) = load_environment(&state, &environment_id) else {
        return Ok(responses::request_navigation(graft, "/environments"));
    };
    let (revision, recipe) = match parse_retry(&pairs) {
        Ok(parsed) => parsed,
        Err(_) => {
            return render_status(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                &record,
                false,
            )
            .await;
        }
    };
    match state
        .environments
        .retry_preparation(&record.id, revision, &recipe)
    {
        Ok(_) => {
            state.environment_preparations.wake();
            let latest = state.environments.get(&record.id).unwrap_or(record);
            render_status(&state, graft, PatchStatus::Ok, &latest, true).await
        }
        Err(error) => render_status(&state, graft, status_for(error), &record, false).await,
    }
}

async fn delete_environment(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(environment_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    let Some(record) = load_environment(&state, &environment_id) else {
        return Ok(responses::request_navigation(graft, "/environments"));
    };
    let (revision, confirmed) = match parse_delete(&pairs) {
        Ok(parsed) => parsed,
        Err(error) => {
            return render_form_command(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                page::CONFIG_TITLE,
                edit_view(&state, &record, FormErrors::default(), error.message()).await?,
            );
        }
    };
    if !confirmed {
        return render_form_command(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            page::CONFIG_TITLE,
            edit_view(
                &state,
                &record,
                FormErrors::default(),
                "Tick the box to delete this environment.",
            )
            .await?,
        );
    }
    match state.environments.delete(&record.id, revision) {
        Ok(_) => {
            state.environment_preparations.wake();
            Ok(responses::request_navigation(graft, "/environments"))
        }
        Err(error) => render_form_command(
            &state,
            graft,
            status_for(error),
            page::CONFIG_TITLE,
            edit_view(&state, &record, FormErrors::default(), error.message()).await?,
        ),
    }
}

async fn refresh_preparation(
    state: &AppState,
    record: &EnvironmentRecord,
    query: RefreshQuery,
) -> AppResult<Response> {
    let cursor = if query.cursor.trim().is_empty() {
        None
    } else {
        match crate::environments::EnvironmentCatalogue::parse_refresh_cursor(&query.cursor) {
            Some(cursor) => Some(cursor),
            None => {
                return Ok(hypergraft::outcome::children_patch(
                    PatchStatus::UnprocessableEntity,
                    "environment-preparation",
                    &status_view(state, record).await,
                )?);
            }
        }
    };
    if !state.environments.cursor_is_stale(cursor) {
        let cursor = cursor.expect("current cursor");
        state
            .environments
            .wait_while_current(cursor, PREPARATION_HOLD)
            .await;
    }
    let latest = state
        .environments
        .get(&record.id)
        .unwrap_or_else(|| record.clone());
    Ok(hypergraft::outcome::children_patch(
        PatchStatus::Ok,
        "environment-preparation",
        &status_view(state, &latest).await,
    )?)
}

fn load_environment(state: &AppState, raw: &str) -> Option<EnvironmentRecord> {
    EnvironmentId::parse(raw).and_then(|id| state.environments.get(&id))
}

fn status_for(error: EnvironmentError) -> PatchStatus {
    match error {
        EnvironmentError::Conflict | EnvironmentError::Missing => PatchStatus::Conflict,
        _ => PatchStatus::UnprocessableEntity,
    }
}

async fn edit_view(
    state: &AppState,
    record: &EnvironmentRecord,
    errors: FormErrors,
    delete_error: &'static str,
) -> AppResult<EnvironmentFormView> {
    edit_view_with_state(
        state,
        record,
        EnvironmentFormState::from_record(record),
        errors,
        delete_error,
    )
    .await
}

async fn edit_view_with_state(
    state: &AppState,
    record: &EnvironmentRecord,
    form: EnvironmentFormState,
    errors: FormErrors,
    delete_error: &'static str,
) -> AppResult<EnvironmentFormView> {
    let status = status_view(state, record).await;
    let affected = state
        .workflows
        .referencing(&record.id)
        .into_iter()
        .map(|workflow| page::AffectedWorkflow {
            name: workflow.definition.name().to_owned(),
            href: format!("/workflows/{}/configuration", workflow.id.as_hex()),
        })
        .collect();
    EnvironmentFormView::edit(record, form, errors, delete_error, &status)
        .map(|view| view.with_affected(affected))
        .map_err(|error| crate::error::AppError::new("render environment form", error))
}

async fn status_view(state: &AppState, record: &EnvironmentRecord) -> EnvironmentStatusView {
    EnvironmentStatusView::from_record(
        record,
        &state.environments,
        &state.environment_snapshots,
        state.environments.refresh_cursor(),
    )
    .await
}

async fn render_catalogue(state: &AppState, graft: PageGraft) -> AppResult<Response> {
    let view = CatalogueView::from_records(
        &state.environments.list(),
        &state.environments,
        &state.environment_snapshots,
    )
    .await;
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
    graft: hypergraft::GraftRequest,
    status: PatchStatus,
    title: &str,
    view: EnvironmentFormView,
) -> AppResult<Response> {
    match graft {
        hypergraft::GraftRequest::Document => {
            let mut response = responses::chat_page_response(title, state, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        hypergraft::GraftRequest::Navigation => {
            Ok(hypergraft::outcome::page_patch(title, "chat-main", &view)?)
        }
        hypergraft::GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "environment-form",
            &view.contents(),
        )?),
    }
}

fn render_form_command(
    _state: &AppState,
    _graft: PatchGraft,
    status: PatchStatus,
    _title: &str,
    view: EnvironmentFormView,
) -> AppResult<Response> {
    Ok(hypergraft::outcome::children_patch(
        status,
        "environment-form",
        &view.contents(),
    )?)
}

async fn render_status(
    state: &AppState,
    _graft: PatchGraft,
    status: PatchStatus,
    record: &EnvironmentRecord,
    refresh_form: bool,
) -> AppResult<Response> {
    let view = status_view(state, record).await;
    if refresh_form {
        let form = EnvironmentFormView::edit(
            record,
            EnvironmentFormState::from_record(record),
            FormErrors::default(),
            "",
            &view,
        )
        .map_err(|error| crate::error::AppError::new("render environment form", error))?;
        let mut patches = hypergraft::PatchSet::new();
        patches.children("environment-form", &form.contents())?;
        patches.children("environment-preparation", &view)?;
        return Ok(patches.respond(status)?);
    }
    Ok(hypergraft::outcome::children_patch(
        status,
        "environment-preparation",
        &view,
    )?)
}
