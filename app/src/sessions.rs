mod cookies;
mod store;
mod tokens;

pub(crate) use cookies::CookieRead;
pub(crate) use store::{SessionSnapshot, SessionStore};
pub(crate) use tokens::{SessionId, ValidatedToken, generate as generate_session_token};

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{header, request::Parts},
    middleware::Next,
    response::Response,
};

use crate::{
    error::{AppResult, AppResultExt, trace_operation_failure},
    state::AppState,
};

/// Resolve the provider session before handlers. Invalid cookies are expired.
pub(crate) async fn resolve_session(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let read = cookies::read_session(request.headers());
    let resolved = match read {
        CookieRead::Missing => ResolvedSession::Anonymous,
        CookieRead::Invalid => ResolvedSession::Invalid,
        CookieRead::Valid(token) => {
            match state.sessions.snapshot(&SessionId::from_validated(&token)) {
                Some(snapshot) => ResolvedSession::Present(snapshot),
                None => ResolvedSession::Invalid,
            }
        }
    };
    let invalid = matches!(resolved, ResolvedSession::Invalid);
    request.extensions_mut().insert(resolved);
    let mut response = next.run(request).await;
    if invalid {
        expire_unless_replaced(&mut response, &state);
    }
    response
}

#[derive(Clone)]
pub(crate) enum ResolvedSession {
    Anonymous,
    Invalid,
    Present(SessionSnapshot),
}

impl<S: Send + Sync> FromRequestParts<S> for ResolvedSession {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<Self>()
            .cloned()
            .unwrap_or(Self::Anonymous))
    }
}

pub(crate) struct OptionalSession(pub(crate) Option<SessionSnapshot>);

impl<S: Send + Sync> FromRequestParts<S> for OptionalSession {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            match parts.extensions.get::<ResolvedSession>().cloned() {
                Some(ResolvedSession::Present(snapshot)) => Some(snapshot),
                _ => None,
            },
        ))
    }
}

pub(crate) fn set_session_cookie(
    response: &mut Response,
    state: &AppState,
    raw: &ValidatedToken,
) -> AppResult<()> {
    let header =
        cookies::session_set_header(&state.config, raw).with_operation("build session cookie")?;
    response.headers_mut().append(header::SET_COOKIE, header);
    Ok(())
}

pub(crate) fn clear_session_cookie(response: &mut Response, state: &AppState) {
    expire_unless_replaced(response, state);
}

fn expire_unless_replaced(response: &mut Response, state: &AppState) {
    if cookies::response_has_replacement(response.headers()) {
        return;
    }
    match cookies::session_deletion_header(&state.config) {
        Ok(header) => {
            response.headers_mut().append(header::SET_COOKIE, header);
        }
        Err(error) => trace_operation_failure("expire session cookie", &error),
    }
}
