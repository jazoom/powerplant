mod forms;
mod page;

#[cfg(test)]
mod tests;

use std::time::Duration;

use axum::{
    Form, Router,
    extract::State,
    http::StatusCode,
    response::Response,
    routing::{get, post},
};
use hypergraft::{CommandGraft, GraftRequest, PatchStatus};

use crate::{
    error::AppResult,
    models,
    plan_login::PendingPlan,
    providers::{ProviderConnection, ProviderError},
    responses,
    sessions::{self, OptionalSession},
    state::AppState,
};

use self::{
    forms::{ConnectForm, ForgetForm, PlanForm},
    page::ConnectViewModel,
};

// A long device-code wait cannot occupy one HTTP response. Each hold ends first.
const PLAN_HOLD: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(15)
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/connect", get(show).post(submit))
        .route("/connect/plan", post(start_plan))
        .route("/connect/forget", post(forget))
}

async fn show(State(state): State<AppState>, graft: GraftRequest) -> AppResult<Response> {
    if graft == GraftRequest::Patch {
        let previous = state.plan_login.snapshot();
        if previous
            .as_ref()
            .is_some_and(|pending| pending.error.is_none())
        {
            state
                .plan_login
                .wait_until_changed(previous, PLAN_HOLD)
                .await;
        }
    }
    render(
        &state,
        graft,
        PatchStatus::Ok,
        ConnectViewModel::initial(&state.vault, state.plan_login.snapshot()),
    )
}

async fn submit(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(form): Form<ConnectForm>,
) -> AppResult<Response> {
    let kind = match form.validate() {
        Ok(kind) => kind,
        Err(error) => {
            return render(
                &state,
                graft.into(),
                PatchStatus::UnprocessableEntity,
                ConnectViewModel::invalid(&state.vault, state.plan_login.snapshot(), form, error),
            );
        }
    };

    let connection = ProviderConnection::with_key(kind, form.api_key, kind.default_model());
    if let Err(error) = state.chat.verify(&connection).await {
        return render(
            &state,
            graft.into(),
            error.patch_status(),
            ConnectViewModel::failed(&state.vault, state.plan_login.snapshot(), kind, error),
        );
    }

    state
        .vault
        .put(connection.clone())
        .map_err(|error| crate::error::AppError::new("store provider", error))?;
    models::refresh(state.clone(), connection);

    connected_redirect(&state, graft, session.is_none())
}

async fn start_plan(
    State(state): State<AppState>,
    graft: CommandGraft,
    Form(form): Form<PlanForm>,
) -> AppResult<Response> {
    let kind = match form.validate() {
        Ok(kind) => kind,
        Err(error) => {
            return render(
                &state,
                graft.into(),
                PatchStatus::UnprocessableEntity,
                ConnectViewModel::plan_invalid(&state.vault, state.plan_login.snapshot(), error),
            );
        }
    };
    let Some(plan_file) = state.vault.plan_file(kind) else {
        return render(
            &state,
            graft.into(),
            PatchStatus::UnprocessableEntity,
            ConnectViewModel::failed(
                &state.vault,
                state.plan_login.snapshot(),
                kind,
                ProviderError::Unreachable,
            ),
        );
    };

    let generation = state.plan_login.begin();
    let started = match crate::providers::plan::start(kind, plan_file).await {
        Ok(started) => started,
        Err(error) => {
            return render(
                &state,
                graft.into(),
                error.patch_status(),
                ConnectViewModel::failed(&state.vault, state.plan_login.snapshot(), kind, error),
            );
        }
    };

    let pending = PendingPlan {
        kind,
        verification_uri: started.prompt.verification_uri,
        user_code: started.prompt.user_code,
        error: None,
    };
    state.plan_login.set_pending(generation, pending);
    let task_state = state.clone();
    let handle = tokio::spawn(async move {
        let result = started
            .done
            .await
            .unwrap_or(Err(ProviderError::Unreachable));
        match result {
            Ok(()) => {
                let connection = ProviderConnection::with_plan(
                    kind,
                    kind.default_model(),
                    task_state.vault.plan_file(kind),
                );
                if task_state.vault.put(connection.clone()).is_ok() {
                    models::refresh(task_state.clone(), connection);
                    task_state.plan_login.finish(generation);
                } else {
                    task_state.plan_login.set_error(
                        generation,
                        "Power Plant could not store that provider. Try again.".to_owned(),
                    );
                }
            }
            Err(ProviderError::Reauthenticate) => {
                task_state.plan_login.set_error(
                    generation,
                    ProviderError::Reauthenticate.message().to_owned(),
                );
            }
            Err(_) => {
                task_state
                    .plan_login
                    .set_error(generation, "Sign-in did not finish. Try again.".to_owned());
            }
        }
    });
    state.plan_login.attach_task(generation, handle);

    render(
        &state,
        graft.into(),
        PatchStatus::Ok,
        ConnectViewModel::initial(&state.vault, state.plan_login.snapshot()),
    )
}

async fn forget(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(form): Form<ForgetForm>,
) -> AppResult<Response> {
    if let Some(kind) = form.provider_kind() {
        state
            .vault
            .forget(kind)
            .map_err(|error| crate::error::AppError::new("forget provider", error))?;
        state.models.remove(kind);
    }

    if !state.vault.has_providers() {
        if let Some(session) = session {
            state.sessions.remove(&session.id);
        }
        let mut response = responses::graft_redirect(graft, "/connect");
        sessions::clear_session_cookie(&mut response, &state);
        return Ok(response);
    }

    render(
        &state,
        graft.into(),
        PatchStatus::Ok,
        ConnectViewModel::initial(&state.vault, state.plan_login.snapshot()),
    )
}

fn connected_redirect(
    state: &AppState,
    graft: CommandGraft,
    needs_session: bool,
) -> AppResult<Response> {
    let mut response = responses::graft_redirect(graft, "/");
    if needs_session {
        let token = sessions::generate_session_token()
            .map_err(|error| crate::error::AppError::new("create session token", error))?;
        state.sessions.insert(token.id());
        sessions::set_session_cookie(&mut response, state, token.raw())?;
    }
    Ok(response)
}

fn render(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    view: ConnectViewModel,
) -> AppResult<Response> {
    match graft {
        GraftRequest::Document => {
            let mut response =
                responses::connect_page_response(page::DOCUMENT_TITLE, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation if status == PatchStatus::Ok => Ok(
            hypergraft::outcome::page_patch(page::DOCUMENT_TITLE, "connect-main", &view)?,
        ),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "connect-card",
            &view.card_contents(),
        )?),
        _ => Ok(responses::no_store_status_response(
            StatusCode::BAD_REQUEST,
            "Bad request",
        )),
    }
}
