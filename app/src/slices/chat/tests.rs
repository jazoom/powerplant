use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    config::RuntimeConfig,
    providers::{ProviderConnection, ProviderKind, SecretString},
    sessions,
    state::AppState,
};

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

fn connected(state: &AppState) -> String {
    let token = sessions::generate_session_token().expect("session token");
    state.sessions.insert(
        token.id(),
        ProviderConnection {
            kind: ProviderKind::Xai,
            api_key: SecretString::new("test-key".to_owned()),
            model: "grok-4.6".to_owned(),
        },
    );
    token.raw().as_str().to_owned()
}

fn patch_headers(token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/")
        .header(header::COOKIE, format!("circus_session={token}"))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from("message=Hello"))
        .unwrap()
}

#[tokio::test]
async fn a_valid_patch_send_streams_progress_then_a_final_frame() {
    let state = test_state();
    let token = connected(&state);
    let response = app(state)
        .oneshot(patch_headers(&token))
        .await
        .expect("chat send");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response.headers().get(hypergraft::GRAFT_TRANSFER).unwrap(),
        "stream"
    );
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        hypergraft::MEDIA_TYPE
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("phase=\"progress\""));
    assert!(text.contains("phase=\"final\""));
    assert!(text.contains("turn-1"));
    assert!(text.contains("Hello from Circus."));
}

#[tokio::test]
async fn an_empty_patch_send_stays_a_complete_unprocessable_response() {
    let state = test_state();
    let token = connected(&state);
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header(header::COOKIE, format!("circus_session={token}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("message=   "))
                .unwrap(),
        )
        .await
        .expect("chat send");

    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(response.headers().get(hypergraft::GRAFT_TRANSFER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Enter a message."));
    assert!(!text.contains("phase=\""));
}

#[tokio::test]
async fn a_document_send_returns_the_full_page() {
    let state = test_state();
    let token = connected(&state);
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header(header::COOKIE, format!("circus_session={token}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("message=Hello"))
                .unwrap(),
        )
        .await
        .expect("chat send");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(response.headers().get(hypergraft::GRAFT_TRANSFER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert!(text.contains("Hello from Circus."));
}
