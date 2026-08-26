use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    agents::{AccessMode, AgentDraft, DirectoryGrant, ToolId},
    config::RuntimeConfig,
    providers::{ProviderConnection, ProviderKind},
    sessions,
    state::AppState,
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

#[tokio::test]
async fn root_redirects_to_the_catalogue_when_no_agent_exists() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("root");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/agents");
}

#[tokio::test]
async fn root_redirects_to_the_sole_agent() {
    let state = test_state();
    let token = connected(&state);
    let dir = tempfile::tempdir().expect("dir");
    let record = state
        .agents
        .create(AgentDraft {
            name: "Only".to_owned(),
            instructions: String::new(),
            tools: vec![ToolId::List],
            directories: vec![DirectoryGrant {
                alias: "project".to_owned(),
                host_path: dir.path().to_path_buf(),
                access: AccessMode::ReadWrite,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("agent");
    state
        .scratch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("root");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        format!("/agents/{}", record.id.as_hex()).as_str()
    );
}

#[tokio::test]
async fn a_catalogue_document_uses_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/agents")
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
    assert!(text.contains("href=\"/agents/new\""));
    assert!(text.contains("data-graft"));
}

#[tokio::test]
async fn create_redirects_to_the_new_agent() {
    let state = test_state();
    let token = connected(&state);
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().canonicalize().expect("canonical");
    let encoded = path.to_string_lossy().replace(' ', "%20");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "name=Reader&instructions=&primary=project&tool_list=on&alias_0=project&path_0={encoded}&access_0=read-write"
                )))
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
    assert!(location.starts_with("/agents/"));
    assert_eq!(state.agents.list().len(), 1);
    assert_eq!(state.agents.list()[0].name, "Reader");
    state
        .scratch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(dir);
}

#[tokio::test]
async fn configuration_patch_returns_a_hypergraft_patch() {
    let state = test_state();
    let token = connected(&state);
    let dir = tempfile::tempdir().expect("dir");
    let record = state
        .agents
        .create(AgentDraft {
            name: "Before".to_owned(),
            instructions: String::new(),
            tools: vec![ToolId::List],
            directories: vec![DirectoryGrant {
                alias: "project".to_owned(),
                host_path: dir.path().to_path_buf(),
                access: AccessMode::ReadWrite,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("agent");
    let path = dir.path().canonicalize().expect("canonical");
    let encoded = path.to_string_lossy().replace(' ', "%20");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/agents/{}/configuration",
                    record.id.as_hex()
                ))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "name=After&instructions=&primary=project&tool_list=on&alias_0=project&path_0={encoded}&access_0=read-write"
                )))
                .unwrap(),
        )
        .await
        .expect("configuration");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        hypergraft::MEDIA_TYPE
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"chat-main\""));
    assert_eq!(state.agents.get(&record.id).expect("updated").name, "After");
    state
        .scratch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(dir);
}

#[tokio::test]
async fn delete_redirects_to_the_catalogue() {
    let state = test_state();
    let token = connected(&state);
    let dir = tempfile::tempdir().expect("dir");
    let record = state
        .agents
        .create(AgentDraft {
            name: "Gone".to_owned(),
            instructions: String::new(),
            tools: vec![ToolId::List],
            directories: vec![DirectoryGrant {
                alias: "project".to_owned(),
                host_path: dir.path().to_path_buf(),
                access: AccessMode::ReadWrite,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("agent");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{}/delete", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("delete");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/agents");
    assert!(state.agents.list().is_empty());
    state
        .scratch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(dir);
}
