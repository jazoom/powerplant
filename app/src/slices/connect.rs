mod forms;
mod page;

#[cfg(test)]
mod tests;

use axum::{
    Form, Router,
    extract::State,
    http::StatusCode,
    response::Response,
    routing::{get, post},
};
use hypergraft::{CommandGraft, GraftRequest, PageGraft, PatchStatus};

use crate::{
    error::AppResult,
    providers::{ProviderConnection, SecretString},
    responses,
    sessions::{self, OptionalSession},
    state::AppState,
};

use self::{forms::ConnectForm, page::ConnectViewModel};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/connect", get(show).post(submit))
        .route("/disconnect", post(disconnect))
}

async fn show(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: PageGraft,
) -> AppResult<Response> {
    if session.is_some() {
        return Ok(responses::graft_redirect(graft, "/"));
    }
    render(
        &state,
        graft.into(),
        PatchStatus::Ok,
        ConnectViewModel::initial(),
    )
}

async fn submit(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(form): Form<ConnectForm>,
) -> AppResult<Response> {
    if session.is_some() {
        return Ok(responses::graft_redirect(graft, "/"));
    }

    let kind = match form.validate() {
        Ok(kind) => kind,
        Err(error) => {
            return render(
                &state,
                graft.into(),
                PatchStatus::UnprocessableEntity,
                ConnectViewModel::invalid(form, error),
            );
        }
    };

    let model = form.resolved_model(kind);
    let connection = ProviderConnection {
        kind,
        api_key: SecretString::new(form.api_key),
        model,
    };
    if let Err(error) = state.chat.verify(&connection).await {
        return render(
            &state,
            graft.into(),
            error.patch_status(),
            ConnectViewModel::failed(kind, connection.model, error),
        );
    }

    let token = sessions::generate_session_token()
        .map_err(|error| crate::error::AppError::new("create session token", error))?;
    state.sessions.insert(token.id(), connection);
    let mut response = responses::graft_redirect(graft, "/");
    sessions::set_session_cookie(&mut response, &state, token.raw())?;
    Ok(response)
}

async fn disconnect(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
) -> AppResult<Response> {
    if let Some(session) = session {
        state.sessions.remove(&session.id);
    }
    let mut response = responses::graft_redirect(graft, "/connect");
    sessions::clear_session_cookie(&mut response, &state);
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
