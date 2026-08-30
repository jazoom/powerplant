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
    workflows::seeds::one_agent_definition,
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

async fn seed_ready_environment(state: &AppState) -> crate::environments::EnvironmentId {
    let (environment, preparation) = state
        .environments
        .create(crate::environments::EnvironmentDraft {
            name: "Alpine Git".to_owned(),
            oci_image: "alpine/git".to_owned(),
            setup_script: String::new(),
        })
        .expect("environment");
    state.environments.claim_oldest_queued().expect("claim");
    let snapshot = crate::environments::snapshot::tests_support::sample_snapshot(preparation.id);
    state.environment_snapshots.mark(
        snapshot.artifact_key.clone(),
        crate::environments::SnapshotAvailability::Available,
    );
    state
        .environments
        .finish_ready(&preparation.id, snapshot, preparation.log)
        .expect("ready");
    environment.id
}

fn create_body(environment_id: crate::environments::EnvironmentId) -> String {
    let environment = format!("default-environment={}", environment_id.as_hex());
    [
        "intent=save",
        "name=One+step",
        environment.as_str(),
        "role_0_key=coding-agent",
        "role_0_name=Coding+agent",
        "role_0_expertise=",
        "role_0_prompt=",
        "step_0_key=work-on-task",
        "step_0_name=Work+on+task",
        "step_0_action=agent",
        "step_0_role=coding-agent",
        "step_0_candidate-access=edit-candidate",
        "step_0_tool_list=on",
        "step_0_input_0_key=candidate",
        "step_0_input_0_kind=candidate-revision",
        "step_0_input_0_source=run-initial-candidate",
        "step_0_output_0_key=assistant-reply",
        "step_0_output_0_kind=assistant-reply",
        "step_0_output_1_key=candidate",
        "step_0_output_1_kind=candidate-revision",
    ]
    .join("&")
}

#[tokio::test]
async fn a_catalogue_document_uses_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/workflows")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("catalogue");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert_eq!(text.matches("id=\"chat-main\"").count(), 1);
    assert!(text.contains("href=\"/workflows/new\""));
    assert!(text.contains("data-graft"));
}

#[tokio::test]
async fn a_catalogue_navigation_patches_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/workflows")
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
async fn a_catalogue_patch_is_rejected() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/workflows")
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
async fn create_redirects_to_configuration() {
    let state = test_state();
    let token = connected(&state);
    let environment = seed_ready_environment(&state).await;
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/workflows")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(create_body(environment)))
                .unwrap(),
        )
        .await
        .expect("create");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.starts_with("/workflows/"));
    assert!(location.ends_with("/configuration"));
    assert_eq!(state.workflows.list()[0].definition.name(), "One step");
}

#[tokio::test]
async fn create_rejects_an_unready_environment() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/workflows")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(create_body(
                    crate::workflows::definition::test_environment_id(),
                )))
                .unwrap(),
        )
        .await
        .expect("unready");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"workflow-form\""));
    assert!(state.workflows.list().is_empty());
}

#[tokio::test]
async fn create_validation_returns_unprocessable() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/workflows")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("intent=save&name="))
                .unwrap(),
        )
        .await
        .expect("invalid");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"workflow-form\""));
}

#[tokio::test]
async fn stale_updates_return_conflict() {
    let state = test_state();
    let token = connected(&state);
    let environment = seed_ready_environment(&state).await;
    let record = state
        .workflows
        .create(one_agent_definition(environment))
        .expect("create");
    state
        .workflows
        .update(
            &record.id,
            record.revision,
            crate::workflows::definition::WorkflowDefinition::from_parts(
                "Edited".to_owned(),
                environment,
                record.definition.roles().to_vec(),
                record.definition.first_step().clone(),
                record.definition.steps().to_vec(),
            )
            .expect("edited"),
        )
        .expect("update");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/workflows/{}/configuration", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "{}&revision={}",
                    create_body(environment),
                    record.revision
                )))
                .unwrap(),
        )
        .await
        .expect("stale");
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
}

#[tokio::test]
async fn delete_redirects_to_the_catalogue() {
    let state = test_state();
    let token = connected(&state);
    let record = state
        .workflows
        .create(one_agent_definition(
            crate::workflows::definition::test_environment_id(),
        ))
        .expect("create");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/workflows/{}/delete", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "revision={}&confirm=on",
                    record.revision
                )))
                .unwrap(),
        )
        .await
        .expect("delete");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/workflows"
    );
    assert!(state.workflows.list().is_empty());
}

#[tokio::test]
async fn oversized_forms_are_rejected_before_row_allocation() {
    let state = test_state();
    let token = connected(&state);
    let oversized = format!("intent=save&name={}&pad={}", "a", "b".repeat(800_000));
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/workflows")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(oversized))
                .unwrap(),
        )
        .await
        .expect("oversize");
    assert_eq!(response.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn anonymous_workflow_pages_redirect_to_connect() {
    let state = test_state();
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/workflows")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("index");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/connect"
    );
}
