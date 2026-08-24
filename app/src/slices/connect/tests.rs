use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    config::RuntimeConfig,
    providers::{
        ChatBackend, ProviderConnection, ProviderError, ProviderKind, SecretString,
        scripted::ScriptedBackend,
    },
    sessions::{self, SESSION_LIFETIME, SESSION_LIFETIME_HOURS, SessionId, ValidatedToken},
    state::AppState,
};

const SECRET_KEY: &str = "sk-test-secret-key-do-not-echo";

fn test_state() -> AppState {
    crate::state::for_test(RuntimeConfig::development_for_test())
}

fn app(state: AppState) -> axum::Router {
    crate::slices::router()
        .layer(from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state)
}

fn state_with_backend(backend: ScriptedBackend) -> AppState {
    let mut state = test_state();
    state.chat = Arc::new(ChatBackend::Scripted(backend));
    state
}

fn connected(state: &AppState) -> String {
    let token = sessions::generate_session_token().expect("session token");
    state.sessions.insert(
        token.id(),
        ProviderConnection {
            kind: ProviderKind::Xai,
            api_key: SecretString::new(SECRET_KEY.to_owned()),
            model: "grok-4.6".to_owned(),
        },
    );
    token.raw().as_str().to_owned()
}

fn cookie(token: &str) -> String {
    format!("circus_session={token}")
}

fn session_id(token: &str) -> SessionId {
    SessionId::from_validated(&ValidatedToken::parse(token).expect("token"))
}

fn set_cookies(response: &axum::http::Response<Body>) -> Vec<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().expect("cookie utf8").to_owned())
        .collect()
}

async fn wait_flag(flag: &AtomicBool, message: &str) {
    for _ in 0..2_000 {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("{message}");
}

#[tokio::test]
async fn connect_page_states_the_session_lifetime() {
    let response = app(test_state())
        .oneshot(
            Request::builder()
                .uri("/connect")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("connect");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains(&format!("after {SESSION_LIFETIME_HOURS} hours")));
}

#[tokio::test]
async fn an_expired_cookie_cannot_resolve_a_session() {
    let state = test_state();
    let token = connected(&state);
    let id = session_id(&token);
    state
        .sessions
        .advance_clock(SESSION_LIFETIME + Duration::from_secs(1));

    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("chat");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/connect"
    );
    let cookies = set_cookies(&response);
    assert!(
        cookies
            .iter()
            .any(|value| value.contains("circus_session="))
    );
    assert!(cookies.iter().all(|value| !value.contains(&token)));
    assert!(cookies.iter().all(|value| !value.contains(SECRET_KEY)));
    assert!(!state.sessions.contains(&id));
}

#[tokio::test]
async fn disconnect_stops_an_active_provider_stream() {
    let started = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let state = state_with_backend(ScriptedBackend::hang_watched(
        started.clone(),
        dropped.clone(),
    ));
    let token = connected(&state);
    let id = session_id(&token);

    let send = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("message=Hello"))
                .unwrap(),
        )
        .await
        .expect("chat send");
    assert_eq!(send.status(), StatusCode::OK);
    wait_flag(&started, "provider stream did not start").await;

    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/disconnect")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("disconnect");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/connect"
    );
    let cookies = set_cookies(&response);
    assert!(
        cookies
            .iter()
            .any(|value| value.contains("circus_session="))
    );
    assert!(cookies.iter().all(|value| !value.contains(&token)));
    assert!(cookies.iter().all(|value| !value.contains(SECRET_KEY)));
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains(SECRET_KEY));
    assert!(!state.sessions.contains(&id));
    wait_flag(&dropped, "provider stream was not dropped").await;
}

fn connect_form() -> &'static str {
    "provider=xai&api_key=sk-test-key&model=grok-4.6"
}

async fn connect_patch(state: AppState) -> axum::http::Response<Body> {
    app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/connect")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(connect_form()))
                .unwrap(),
        )
        .await
        .expect("connect")
}

#[tokio::test]
async fn a_rejected_key_returns_an_unauthorised_result() {
    let mut backend = ScriptedBackend::accept();
    backend.verify_result = Err(ProviderError::Rejected);
    let response = connect_patch(state_with_backend(backend)).await;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(text.contains("That key was rejected"));
    assert_field_target(&text, "connect-key", "sk-test-key");
}

#[tokio::test]
async fn an_outage_does_not_tell_the_user_to_replace_a_valid_key() {
    let mut backend = ScriptedBackend::accept();
    backend.verify_result = Err(ProviderError::Unreachable);
    let response = connect_patch(state_with_backend(backend)).await;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains("The provider could not be reached"));
    assert!(!text.contains("key was rejected"));
    assert_field_target(&text, "connect-provider", "sk-test-key");
}

#[tokio::test]
async fn a_rate_limit_returns_a_typed_retry_interval() {
    let mut backend = ScriptedBackend::accept();
    backend.verify_result = Err(ProviderError::RateLimited {
        retry_after: hypergraft::RetryAfter::seconds(7),
    });
    let response = connect_patch(state_with_backend(backend)).await;
    let status = response.status();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(retry_after.as_deref(), Some("7"));
    assert!(text.contains("rate-limited"));
    assert_field_target(&text, "connect-provider", "sk-test-key");
}

async fn connect_submit(
    state: AppState,
    body: String,
    enhanced: bool,
) -> axum::http::Response<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/connect")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if enhanced {
        request = request
            .header(hypergraft::GRAFT_REQUEST, "patch")
            .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
    }
    app(state)
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .expect("connect")
}

fn assert_field_target(text: &str, target: &str, secret: &str) {
    assert!(text.contains(&format!("href=\"#{target}\"")));
    assert_eq!(text.matches("aria-invalid").count(), 1);
    assert_eq!(
        text.matches(r#"aria-describedby="connect-errors""#).count(),
        1
    );
    assert_eq!(aria_invalid_control(text), target);
    assert!(!text.contains(secret));
}

fn has_checked_control(text: &str) -> bool {
    text.replace("has-checked", "").contains("checked")
}

fn aria_invalid_control(text: &str) -> &str {
    let aria = text.find("aria-invalid").expect("aria-invalid");
    let start = text[..aria].rfind('<').expect("tag open");
    let tag = &text[start..aria];
    for id in ["connect-provider", "connect-key", "connect-model"] {
        if tag.contains(id) {
            return id;
        }
    }
    panic!("aria-invalid on unknown control: {tag}");
}

#[tokio::test]
async fn a_provider_error_targets_the_provider_fieldset() {
    let response = connect_submit(
        test_state(),
        "provider=openai&api_key=sk-secret-key&model=custom-model".to_owned(),
        true,
    )
    .await;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains(r#"operation="children" target="connect-card""#));
    assert!(text.contains("Choose a provider."));
    assert_field_target(&text, "connect-provider", "sk-secret-key");
    assert!(!has_checked_control(&text));
    assert!(text.contains(r#"value="custom-model""#));
}

#[tokio::test]
async fn an_api_key_error_targets_the_key_control() {
    let response = connect_submit(
        test_state(),
        "provider=xai&api_key=&model=custom-model".to_owned(),
        true,
    )
    .await;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains("Enter an API key."));
    assert_field_target(&text, "connect-key", "sk-secret-key");
    assert!(text.contains(r#"value="xai""#));
    assert!(has_checked_control(&text));
    assert!(text.contains(r#"value="custom-model""#));
}

#[tokio::test]
async fn a_model_error_targets_the_model_control() {
    let long_model = "a".repeat(super::forms::MAXIMUM_MODEL_BYTES + 1);
    let response = connect_submit(
        test_state(),
        format!("provider=xai&api_key=sk-secret-key&model={long_model}"),
        true,
    )
    .await;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains("That model name is too long."));
    assert_field_target(&text, "connect-model", "sk-secret-key");
    assert!(text.contains(r#"value="xai""#));
    assert!(has_checked_control(&text));
    assert!(!text.contains(&long_model));
}

#[tokio::test]
async fn a_native_rejection_uses_the_same_field_relation() {
    let response = connect_submit(
        test_state(),
        "provider=openai&api_key=sk-secret-key&model=custom-model".to_owned(),
        false,
    )
    .await;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains("<!doctype html>"));
    assert!(text.contains("Choose a provider."));
    assert_field_target(&text, "connect-provider", "sk-secret-key");
    assert!(text.contains(r#"value="custom-model""#));
}
