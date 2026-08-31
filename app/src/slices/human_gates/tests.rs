use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use super::forms::{DecisionForm, FormError};
use crate::{config::RuntimeConfig, state::AppState};

fn test_state() -> AppState {
    crate::state::for_test(RuntimeConfig::development_for_test())
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

#[test]
fn decision_forms_reject_duplicate_and_blank_revision_fields() {
    let duplicate = vec![
        ("gate-revision".to_owned(), "1".to_owned()),
        ("gate-revision".to_owned(), "1".to_owned()),
        ("candidate".to_owned(), "sha256:00".to_owned()),
    ];
    assert_eq!(
        DecisionForm::parse(duplicate, false).err(),
        Some(FormError::Invalid)
    );

    let blank_note = vec![
        ("gate-revision".to_owned(), "1".to_owned()),
        ("candidate".to_owned(), "sha256:00".to_owned()),
        ("note".to_owned(), "  ".to_owned()),
    ];
    assert_eq!(
        DecisionForm::parse(blank_note, true).err(),
        Some(FormError::Note)
    );
}

#[tokio::test]
async fn anonymous_gate_requests_redirect_to_connect() {
    let state = test_state();
    let id = "0".repeat(32);
    let detail = format!("/runs/{id}/gates/{id}");
    let approve = format!("/runs/{id}/gates/{id}/approve");
    let cases = [
        ("GET", detail.as_str(), None, false),
        ("GET", detail.as_str(), Some("navigation"), true),
        ("POST", approve.as_str(), None, false),
        ("POST", approve.as_str(), Some("patch"), true),
    ];
    for (method, uri, graft, enhanced) in cases {
        assert_connect_redirect(&state, method, uri, graft, enhanced).await;
    }
}

async fn assert_connect_redirect(
    state: &AppState,
    method: &str,
    uri: &str,
    graft: Option<&str>,
    enhanced: bool,
) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(graft) = graft {
        builder = builder
            .header(hypergraft::GRAFT_REQUEST, graft)
            .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
    }
    let response = app(state)
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("anonymous");
    if enhanced {
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            hypergraft::MEDIA_TYPE
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            text.contains("navigate=\"/connect\""),
            "{method} {uri} {graft:?}: {text}"
        );
    } else {
        assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/connect"
        );
    }
}
