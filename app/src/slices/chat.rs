mod forms;
mod job;
mod page;

#[cfg(test)]
mod tests;

use std::time::Duration;

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::Response,
    routing::{get, post},
};

use hypergraft::{CommandGraft, GraftRequest, PatchSet, PatchStatus};

use crate::{
    error::AppResult,
    responses,
    sessions::{self, BeginTurnError, JobIdError, JobStatus, OptionalSession, SessionSnapshot},
    state::AppState,
};

use self::{
    forms::{ChatForm, CursorError, ModelForm, ObserveQuery, SandboxAction, SandboxForm},
    job::{observe_response, run_job, user_transcript_patch},
    page::{ChatViewModel, JobObserveContents, TranscriptContents},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(show).post(send))
        .route("/model", get(refresh_model_options).post(update_model))
        .route("/sandbox", get(show_sandbox).post(update_sandbox))
        .route("/jobs/{job_id}/cancel", post(cancel))
}

async fn show(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
    Query(query): Query<ObserveQuery>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }

    match graft {
        GraftRequest::Document => render_document(
            &state,
            PatchStatus::Ok,
            view(&state, &session, "", "", "").await,
        ),
        GraftRequest::Navigation => {
            navigate_page(&state, &view(&state, &session, "", "", "").await)
        }
        GraftRequest::Patch => observe(&state, &session, query).await,
    }
}

async fn send(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(form): Form<ChatForm>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    let Some(connection) = state.vault.selected_connection() else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };

    if !form.is_bounded() {
        return reject_chat_input(&state, graft, &session, "Enter a message.").await;
    }

    let started = match state
        .sessions
        .begin_turn(&session.id, form.message.trim().to_owned())
    {
        Ok(started) => started,
        Err(BeginTurnError::MissingSession) => {
            return Ok(responses::graft_redirect(graft, "/connect"));
        }
        Err(BeginTurnError::Conflict) => {
            let Some(latest) = state.sessions.snapshot(&session.id) else {
                return Ok(responses::graft_redirect(graft, "/connect"));
            };
            return reject_parallel_command(&state, graft, &latest).await;
        }
        Err(BeginTurnError::JobId) => {
            return Err(crate::error::AppError::new(
                "create job identifier",
                JobIdError::RandomUnavailable,
            ));
        }
    };

    let job = started.job.clone();
    tokio::spawn(run_job(
        state.clone(),
        session.id,
        connection,
        started.turns.clone(),
        job,
    ));

    match graft {
        CommandGraft::Document => {
            let Some(latest) = state.sessions.snapshot(&session.id) else {
                return Ok(responses::graft_redirect(graft, "/connect"));
            };
            render_document(
                &state,
                PatchStatus::Ok,
                view(&state, &latest, "", "", "").await,
            )
        }
        CommandGraft::Patch => accept_job_patch(&started.turns, &started.job.id().as_hex()),
    }
}

async fn refresh_model_options(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let Some(current) = state.sessions.snapshot(&session.id) else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    match graft {
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "desk-model-catalogue",
            &view(&state, &current, "", "", "")
                .await
                .desk_model_catalogue(),
        )?),
        GraftRequest::Document | GraftRequest::Navigation => {
            Ok(responses::graft_redirect(graft, "/"))
        }
    }
}

async fn update_model(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(form): Form<ModelForm>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }

    let Some(current) = state.sessions.snapshot(&session.id) else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    if current
        .job
        .as_ref()
        .is_some_and(|job| job.status == JobStatus::Running)
    {
        return reject_model(&state, graft, &current, "Wait until this reply finishes.").await;
    }
    if form.wants_favourite_toggle() {
        return toggle_favourite(&state, graft, &current, &form).await;
    }

    match form.validate(|kind| state.vault.contains(kind)) {
        Ok((kind, model)) => {
            let model = submitted_model(&state, &form, kind, model);
            state
                .vault
                .select(kind, model)
                .map_err(|error| crate::error::AppError::new("store model", error))?;
        }
        Err(forms::ModelError::Provider) => {
            return reject_model(&state, graft, &current, "Choose a stored provider.").await;
        }
        Err(forms::ModelError::Model) => {
            return reject_model(&state, graft, &current, "That model name is too long.").await;
        }
    }

    let Some(current) = state.sessions.snapshot(&session.id) else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    match graft {
        CommandGraft::Document => render_document(
            &state,
            PatchStatus::Ok,
            view(&state, &current, "", "", "").await,
        ),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "desk-settings",
            &view(&state, &current, "", "", "").await.desk_settings(),
        )?),
    }
}

async fn cancel(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Path(job_id): Path<String>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    if let Some(id) = sessions::JobId::parse(&job_id)
        && let Some(job) = state.sessions.job(&session.id, &id)
    {
        job.request_cancel();
    }
    let Some(latest) = state.sessions.snapshot(&session.id) else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    match graft {
        CommandGraft::Document => render_document(
            &state,
            PatchStatus::Ok,
            view(&state, &latest, "", "", "").await,
        ),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "job-observe",
            &view(&state, &latest, "", "", "").await.job_observe(),
        )?),
    }
}

const SANDBOX_HOLD: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(1)
};

async fn show_sandbox(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let Some(current) = state.sessions.snapshot(&session.id) else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    match graft {
        GraftRequest::Document | GraftRequest::Navigation => {
            Ok(responses::graft_redirect(graft, "/"))
        }
        GraftRequest::Patch => {
            let previous = state.sandbox.view().await;
            if previous.status.is_starting() {
                state
                    .sandbox
                    .wait_until_changed(previous, SANDBOX_HOLD)
                    .await;
            }
            Ok(hypergraft::outcome::children_patch(
                PatchStatus::Ok,
                "sandbox-status",
                &view(&state, &current, "", "", "").await.sandbox_status(),
            )?)
        }
    }
}

async fn update_sandbox(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(form): Form<SandboxForm>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    let Some(current) = state.sessions.snapshot(&session.id) else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    let action = match form.validate() {
        Ok(action) => action,
        Err(_) => {
            return reject_sandbox(&state, graft, &current, "Choose start or stop.").await;
        }
    };
    let result = match action {
        SandboxAction::Start => state.sandbox.start().await,
        SandboxAction::Stop => state.sandbox.stop().await,
    };
    let error = match result {
        Ok(()) => "",
        Err(error) => error.message(),
    };
    let status = if error.is_empty() {
        PatchStatus::Ok
    } else {
        PatchStatus::UnprocessableEntity
    };
    match graft {
        CommandGraft::Document => {
            render_document(&state, status, view(&state, &current, "", "", error).await)
        }
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "sandbox-status",
            &view(&state, &current, "", "", error).await.sandbox_status(),
        )?),
    }
}

async fn toggle_favourite(
    state: &AppState,
    graft: CommandGraft,
    session: &SessionSnapshot,
    form: &ModelForm,
) -> AppResult<Response> {
    match form.validate_favourite(|kind| state.vault.contains(kind)) {
        Ok((kind, model)) => {
            let model = submitted_model(state, form, kind, model);
            match state.vault.toggle_favourite(kind, &model) {
                Ok(_) => {}
                Err(crate::vault::FavouriteError::Provider) => {
                    return reject_model(state, graft, session, "Choose a stored provider.").await;
                }
                Err(crate::vault::FavouriteError::Full) => {
                    return reject_model(state, graft, session, "The favourites list is full.")
                        .await;
                }
                Err(crate::vault::FavouriteError::Persist(error)) => {
                    return Err(crate::error::AppError::new("store favourite", error));
                }
            }
        }
        Err(forms::ModelError::Provider) => {
            return reject_model(state, graft, session, "Choose a stored provider.").await;
        }
        Err(forms::ModelError::Model) => {
            return reject_model(state, graft, session, "Choose a model.").await;
        }
    }
    match graft {
        CommandGraft::Document => render_document(
            state,
            PatchStatus::Ok,
            view(state, session, "", "", "").await,
        ),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "desk-model-catalogue",
            &view(state, session, "", "", "")
                .await
                .desk_model_catalogue(),
        )?),
    }
}

fn submitted_model(
    state: &AppState,
    form: &ModelForm,
    kind: crate::providers::ProviderKind,
    model: String,
) -> String {
    if form.provider_model_synced {
        return model;
    }
    let providers = state.vault.desk_providers();
    let Some(selected) = providers.iter().find(|provider| provider.selected) else {
        return model;
    };
    if selected.kind == kind {
        return model;
    }
    providers
        .iter()
        .find(|provider| provider.kind == kind)
        .map(|provider| provider.model.clone())
        .unwrap_or(model)
}

async fn observe(
    state: &AppState,
    session: &SessionSnapshot,
    query: ObserveQuery,
) -> AppResult<Response> {
    let cursor = match query.cursor() {
        Ok(cursor) => cursor,
        Err(CursorError::Malformed | CursorError::Excessive) => {
            return Ok(hypergraft::outcome::children_patch(
                PatchStatus::UnprocessableEntity,
                "job-observe",
                &view(state, session, "", "", "")
                    .await
                    .job_observe_with("That cursor is not valid."),
            )?);
        }
    };
    let Some(job_id) = query.job_id() else {
        return refresh_composer(state, session).await;
    };
    let Some(job) = state.sessions.job(&session.id, &job_id) else {
        return refresh_composer(state, session).await;
    };
    Ok(observe_response(job, cursor))
}

async fn refresh_composer(state: &AppState, session: &SessionSnapshot) -> AppResult<Response> {
    Ok(hypergraft::outcome::children_patch(
        PatchStatus::Ok,
        "job-observe",
        &view(state, session, "", "", "").await.job_observe(),
    )?)
}

fn accept_job_patch(turns: &[crate::providers::ChatTurn], job_id: &str) -> AppResult<Response> {
    let mut patches = user_transcript_patch(turns)?;
    patches.children(
        "job-observe",
        &JobObserveContents::observing(job_id, 0, "Writing", ""),
    )?;
    Ok(patches.respond(PatchStatus::Ok)?)
}

async fn reject_parallel_command(
    state: &AppState,
    graft: CommandGraft,
    session: &SessionSnapshot,
) -> AppResult<Response> {
    const MESSAGE: &str = "Wait until this reply finishes.";
    match graft {
        CommandGraft::Document => render_document(
            state,
            PatchStatus::Conflict,
            view(state, session, MESSAGE, "", "").await,
        ),
        CommandGraft::Patch => {
            let view = view(state, session, "", "", "").await;
            let mut patches = PatchSet::new();
            patches.children("transcript", &TranscriptContents { turns: &view.turns })?;
            patches.children("job-observe", &view.job_observe_with(MESSAGE))?;
            Ok(patches.respond(PatchStatus::Conflict)?)
        }
    }
}

async fn reject_chat_input(
    state: &AppState,
    graft: CommandGraft,
    session: &SessionSnapshot,
    message: &'static str,
) -> AppResult<Response> {
    match graft {
        CommandGraft::Document => render_document(
            state,
            PatchStatus::UnprocessableEntity,
            view(state, session, message, "", "").await,
        ),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::UnprocessableEntity,
            "composer",
            &view(state, session, message, "", "").await.composer(),
        )?),
    }
}

async fn reject_model(
    state: &AppState,
    graft: CommandGraft,
    session: &SessionSnapshot,
    message: &'static str,
) -> AppResult<Response> {
    match graft {
        CommandGraft::Document => render_document(
            state,
            PatchStatus::UnprocessableEntity,
            view(state, session, "", message, "").await,
        ),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::UnprocessableEntity,
            "desk-settings",
            &view(state, session, "", message, "").await.desk_settings(),
        )?),
    }
}

async fn reject_sandbox(
    state: &AppState,
    graft: CommandGraft,
    session: &SessionSnapshot,
    message: &'static str,
) -> AppResult<Response> {
    match graft {
        CommandGraft::Document => render_document(
            state,
            PatchStatus::UnprocessableEntity,
            view(state, session, "", "", message).await,
        ),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::UnprocessableEntity,
            "sandbox-status",
            &view(state, session, "", "", message).await.sandbox_status(),
        )?),
    }
}

async fn view(
    state: &AppState,
    session: &SessionSnapshot,
    error: &'static str,
    desk_error: &'static str,
    sandbox_error: &'static str,
) -> ChatViewModel {
    ChatViewModel::from_session(
        session,
        &state.vault,
        &state.models,
        state.sandbox.view().await,
        error,
        desk_error,
        sandbox_error,
    )
}

fn navigate_page(state: &AppState, view: &ChatViewModel) -> AppResult<Response> {
    match hypergraft::outcome::page_patch(page::DOCUMENT_TITLE, "chat-main", view) {
        Ok(response) => Ok(response),
        Err(error) if error.kind() == hypergraft::PatchBuildErrorKind::ResponseLimit => {
            crate::error::trace_patch_build_failure("construct chat page navigation patch", &error);
            responses::chat_page_response(page::DOCUMENT_TITLE, &state.assets, view)
        }
        Err(error) => Err(error.into()),
    }
}

fn render_document(
    state: &AppState,
    status: PatchStatus,
    view: ChatViewModel,
) -> AppResult<Response> {
    let mut response = responses::chat_page_response(page::DOCUMENT_TITLE, &state.assets, &view)?;
    responses::apply_patch_status(&mut response, status);
    Ok(response)
}
