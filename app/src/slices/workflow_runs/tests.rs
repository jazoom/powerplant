use askama::Template;
use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    config::RuntimeConfig,
    providers::{ProviderConnection, ProviderKind},
    sessions,
    state::AppState,
    workflows::{
        RunId, WorkflowRun, definition::PinnedWorkflowDefinition, seeds::one_agent_definition,
    },
};

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

fn cookie(token: &str) -> String {
    format!("powerplant_session={token}")
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

fn stored_run(state: &AppState) -> RunId {
    let definition = one_agent_definition(crate::workflows::definition::test_environment_id());
    let environments = crate::workflows::test_environment_set(&definition);
    let run = WorkflowRun::create(
        RunId::generate().expect("run"),
        1,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    let id = run.id;
    state.workflow_runs.create(run).expect("store");
    id
}

#[tokio::test]
async fn a_runs_document_uses_chat_main() {
    let state = test_state();
    let token = connected(&state);
    stored_run(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/runs")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("index");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert_eq!(text.matches("id=\"chat-main\"").count(), 1);
    assert!(text.contains("href=\"/runs/"));
    assert!(text.contains("data-graft"));
}

#[tokio::test]
async fn a_runs_navigation_patches_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/runs")
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "navigation")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("navigation");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("<!doctype html>"));
    assert!(text.contains("operation=\"children\" target=\"chat-main\""));
}

#[tokio::test]
async fn a_runs_patch_is_rejected() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/runs")
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("patch");
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_detail_document_uses_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let id = stored_run(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}", id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert_eq!(text.matches("id=\"run-detail\"").count(), 1);
    assert!(text.contains("Refresh"));
}

#[tokio::test]
async fn a_detail_navigation_patches_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let id = stored_run(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}", id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "navigation")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("navigation");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("operation=\"children\" target=\"chat-main\""));
}

#[tokio::test]
async fn a_detail_patch_targets_run_detail() {
    let state = test_state();
    let token = connected(&state);
    let id = stored_run(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}", id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("patch");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("operation=\"children\" target=\"run-detail\""));
    assert!(!text.contains("id=\"run-detail\""));
}

#[test]
fn run_timeline_renders_status_handoffs_and_the_commit_identifier() {
    let view = super::page::RunDetailView {
        run_id: "run".to_owned(),
        name: "Sequential team".to_owned(),
        name_href: String::new(),
        catalogue_note: String::new(),
        version: "version".to_owned(),
        state: "Completed",
        created: "now".to_owned(),
        current_step: "Commit".to_owned(),
        steps: vec![super::page::StepView {
            name: "Commit".to_owned(),
            action: "System command",
            candidate_access: "",
            environment: "Alpine Git".to_owned(),
            status: "Completed",
            result: "Completed".to_owned(),
            artefacts: vec![super::page::StepArtefactView {
                href: "/runs/run/artefacts/candidate".to_owned(),
                key: "committed-candidate".to_owned(),
                kind: "candidate-revision",
                candidate_hash: String::new(),
                status: "",
                note: "",
            }],
            commit: "01234567".to_owned(),
        }],
        environments: Vec::new(),
        attempts: Vec::new(),
        artefacts: Vec::new(),
    };

    let rendered = view.render().expect("render timeline");

    assert!(rendered.contains("Completed"));
    assert!(rendered.contains("Commit 01234567"));
    assert!(rendered.contains("href=\"/runs/run/artefacts/candidate\" data-graft"));
}

#[tokio::test]
async fn an_unknown_run_redirects_to_the_index() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}", "a".repeat(32)))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("missing");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/runs");
}

#[tokio::test]
async fn an_unknown_artefact_redirects_to_the_run() {
    let state = test_state();
    let token = connected(&state);
    let id = stored_run(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/runs/{}/artefacts/{}",
                    id.as_hex(),
                    "a".repeat(32)
                ))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("missing artefact");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        format!("/runs/{}", id.as_hex()).as_str()
    );
}

#[tokio::test]
async fn anonymous_run_pages_redirect_to_connect() {
    let state = test_state();
    let response = app(&state)
        .oneshot(Request::builder().uri("/runs").body(Body::empty()).unwrap())
        .await
        .expect("index");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/connect"
    );
}
