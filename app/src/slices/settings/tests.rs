use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    config::RuntimeConfig,
    preferences::Theme,
    providers::{ProviderConnection, ProviderKind},
    sessions,
    state::AppState,
};

fn test_state() -> AppState {
    crate::tests::test_state(RuntimeConfig::development())
}

fn app(state: &AppState) -> axum::Router {
    crate::slices::router()
        .layer(from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone())
}

fn connected(state: &AppState) -> String {
    let token = sessions::generate_session_token().expect("session token");
    state
        .vault
        .put(ProviderConnection::with_key(
            ProviderKind::Xai,
            "test-key",
            "grok-4.6",
        ))
        .expect("vault");
    state.sessions.insert(token.id());
    token.raw().as_str().to_owned()
}

fn cookie(token: &str) -> String {
    format!("powerplant_session={token}")
}

#[tokio::test]
async fn settings_supports_documents_and_navigation_only() {
    let state = test_state();
    let token = connected(&state);

    let document = app(&state)
        .oneshot(
            Request::builder()
                .uri("/settings")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("document");
    assert_eq!(document.status(), StatusCode::OK);
    let body = to_bytes(document.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(text.matches("id=\"chat-main\"").count(), 1);

    let navigation = app(&state)
        .oneshot(
            Request::builder()
                .uri("/settings")
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "navigation")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("navigation");
    assert_eq!(navigation.status(), StatusCode::OK);
    let body = to_bytes(navigation.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("operation=\"children\" target=\"chat-main\""));

    let patch = app(&state)
        .oneshot(
            Request::builder()
                .uri("/settings")
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("patch");
    assert_eq!(patch.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_initial_document_renders_the_saved_theme_and_selection() {
    let state = test_state();
    state
        .preferences
        .set_theme(Theme::SpringfieldDark)
        .expect("theme");
    let token = connected(&state);

    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/settings")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("settings");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(text.contains("<html lang=\"en-AU\" data-theme=\"springfield-dark\">"));
    assert!(text.contains("data-active-theme=\"springfield-dark\""));
    assert_eq!(text.matches("selected").count(), 1);
}

#[tokio::test]
async fn a_theme_patch_persists_and_returns_the_authoritative_selector() {
    let state = test_state();
    let token = connected(&state);

    let response = app(&state)
        .oneshot(theme_request(&token, "springfield-dark"))
        .await
        .expect("theme patch");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(state.preferences.theme(), Theme::SpringfieldDark);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("operation=\"children\" target=\"theme-setting\""));
    assert!(text.contains("data-active-theme=\"springfield-dark\""));
    assert_eq!(text.matches("selected").count(), 1);
}

#[tokio::test]
async fn an_unknown_theme_is_rejected_without_changing_the_preference() {
    let state = test_state();
    let token = connected(&state);

    let response = app(&state)
        .oneshot(theme_request(&token, "unknown"))
        .await
        .expect("theme patch");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.preferences.theme(), Theme::SpringfieldLight);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Choose a listed theme."));
    assert!(text.contains("data-active-theme=\"springfield-light\""));
}

fn theme_request(token: &str, theme: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/settings/theme")
        .header(header::COOKIE, cookie(token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from(format!("theme={theme}")))
        .unwrap()
}
