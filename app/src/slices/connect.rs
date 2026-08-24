mod forms;
mod page;

use axum::{
    Form, Router,
    extract::State,
    response::Response,
    routing::{get, post},
};

use crate::{
    error::AppResult,
    providers::{ProviderConnection, ProviderError, SecretString},
    responses::{self, CommandGraft, GraftRequest, PageGraft, PatchStatus},
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

    let Some(kind) = form.provider_kind() else {
        return render(
            &state,
            graft.into(),
            PatchStatus::UnprocessableEntity,
            ConnectViewModel::invalid(form, "Choose a provider."),
        );
    };
    if !form.api_key_is_bounded() {
        return render(
            &state,
            graft.into(),
            PatchStatus::UnprocessableEntity,
            ConnectViewModel::invalid(form, "Enter an API key."),
        );
    }
    if !form.model_is_bounded() {
        return render(
            &state,
            graft.into(),
            PatchStatus::UnprocessableEntity,
            ConnectViewModel::invalid(form, "That model name is too long."),
        );
    }

    let model = form.resolved_model(kind);
    let connection = ProviderConnection {
        kind,
        api_key: SecretString::new(form.api_key),
        model,
    };
    match state.chat.verify(&connection).await {
        Ok(()) => {}
        Err(ProviderError::Rejected) | Err(ProviderError::Unreachable) => {
            return render(
                &state,
                graft.into(),
                PatchStatus::Unauthorized,
                ConnectViewModel::rejected(kind, connection.model),
            );
        }
        Err(ProviderError::EmptyReply) => {
            return render(
                &state,
                graft.into(),
                PatchStatus::UnprocessableEntity,
                ConnectViewModel::rejected(kind, connection.model),
            );
        }
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
    responses::connect_graft_page(
        graft,
        status,
        page::DOCUMENT_TITLE,
        &state.assets,
        &view,
        &view.card_contents(),
    )
}
