use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    agents::{
        AccessMode, AgentDraft, AgentError, AgentRecord, AgentStore, DirectoryGrant, NetworkAccess,
        ToolId,
    },
    config::RuntimeConfig,
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
async fn the_catalogue_links_each_exact_grant_match() {
    let state = test_state();
    let token = connected(&state);
    let harbour_dir = git_worktree();
    let quay_dir = git_worktree();
    let other_dir = git_worktree();
    let harbour = state
        .projects
        .create("Harbour".to_owned(), harbour_dir.path().to_path_buf())
        .expect("harbour");
    let quay = state
        .projects
        .create("Quay".to_owned(), quay_dir.path().to_path_buf())
        .expect("quay");
    let both = state
        .agents
        .create(AgentDraft {
            name: "Both".to_owned(),
            instructions: String::new(),
            tools: vec![ToolId::List],
            network: NetworkAccess::None,
            directories: vec![
                DirectoryGrant {
                    alias: "harbour".to_owned(),
                    host_path: harbour.host_path.clone(),
                    access: AccessMode::ReadWrite,
                },
                DirectoryGrant {
                    alias: "quay".to_owned(),
                    host_path: quay.host_path.clone(),
                    access: AccessMode::ReadOnly,
                },
            ],
            primary_directory: "harbour".to_owned(),
        })
        .expect("both");
    let none = state
        .agents
        .create(AgentDraft {
            name: "None".to_owned(),
            instructions: String::new(),
            tools: vec![ToolId::List],
            network: NetworkAccess::None,
            directories: vec![DirectoryGrant {
                alias: "project".to_owned(),
                host_path: other_dir.path().to_path_buf(),
                access: AccessMode::ReadWrite,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("none");
    keep_dir(&state, harbour_dir);
    keep_dir(&state, quay_dir);
    keep_dir(&state, other_dir);
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
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Both"));
    assert!(text.contains("None"));
    assert!(text.contains(&crate::projects::desk_path(&harbour.id, &both.id)));
    assert!(text.contains(&crate::projects::desk_path(&quay.id, &both.id)));
    assert!(!text.contains(&crate::projects::desk_path(&harbour.id, &none.id)));
    assert!(!text.contains(&crate::projects::desk_path(&quay.id, &none.id)));
    assert!(text.contains("No matching project"));
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
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "intent=save&name=Reader&instructions=&primary=project&tool_list=on&alias_0=project&path_0={encoded}&access_0=read-write"
                )))
                .unwrap(),
        )
        .await
        .expect("create");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("navigate=\"/agents/"));
    assert_eq!(state.agents.list().len(), 1);
    assert_eq!(state.agents.list()[0].name, "Reader");
    state.keep_temp_dir(dir);
}

#[tokio::test]
async fn create_persistence_failure_returns_internal_error() {
    let mut state = test_state();
    let data = tempfile::tempdir().expect("data");
    let agents_dir = data.path().join("agents");
    state.agents = Arc::new(
        AgentStore::open(agents_dir.clone(), &data.path().join("project.json")).expect("store"),
    );
    std::fs::remove_dir(&agents_dir).expect("remove agents directory");
    std::fs::write(&agents_dir, b"not a directory").expect("block agents directory");
    let token = connected(&state);
    let project = tempfile::tempdir().expect("project");
    let encoded = project
        .path()
        .canonicalize()
        .expect("canonical")
        .to_string_lossy()
        .replace(' ', "%20");

    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "intent=save&name=Reader&instructions=&primary=project&tool_list=on&alias_0=project&path_0={encoded}&access_0=read-write"
                )))
                .unwrap(),
        )
        .await
        .expect("create");

    assert_eq!(
        response.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], b"Internal error");
    assert!(state.agents.list().is_empty());
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
            network: NetworkAccess::None,
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
                    "intent=save&name=After&instructions=&primary=project&tool_list=on&alias_0=project&path_0={encoded}&access_0=read-write&revision={}",
                    record.revision
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
    assert!(text.contains("target=\"agent-form\""));
    assert_eq!(state.agents.get(&record.id).expect("updated").name, "After");
    state.keep_temp_dir(dir);
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
            network: NetworkAccess::None,
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
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!("revision={}", record.revision)))
                .unwrap(),
        )
        .await
        .expect("delete");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("navigate=\"/agents\""));
    assert!(state.agents.list().is_empty());
    state.keep_temp_dir(dir);
}

fn seed_agent(state: &AppState, name: &str) -> (tempfile::TempDir, AgentRecord) {
    let dir = tempfile::tempdir().expect("dir");
    let record = state
        .agents
        .create(AgentDraft {
            name: name.to_owned(),
            instructions: String::new(),
            tools: vec![ToolId::List],
            network: NetworkAccess::None,
            directories: vec![DirectoryGrant {
                alias: "project".to_owned(),
                host_path: dir.path().to_path_buf(),
                access: AccessMode::ReadWrite,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("agent");
    (dir, record)
}

fn configuration_body(path: &std::path::Path, name: &str, revision: u32) -> String {
    let encoded = path
        .canonicalize()
        .expect("canonical")
        .to_string_lossy()
        .replace(' ', "%20");
    format!(
        "intent=save&name={name}&instructions=&primary=project&tool_list=on&alias_0=project&path_0={encoded}&access_0=read-write&revision={revision}"
    )
}

#[tokio::test]
async fn stale_update_returns_conflict() {
    let state = test_state();
    let token = connected(&state);
    let (dir, record) = seed_agent(&state, "Before");
    let updated = state
        .agents
        .update(
            &record.id,
            record.revision,
            AgentDraft {
                name: "After".to_owned(),
                instructions: String::new(),
                tools: vec![ToolId::List],
                network: NetworkAccess::None,
                directories: vec![DirectoryGrant {
                    alias: "project".to_owned(),
                    host_path: dir.path().to_path_buf(),
                    access: AccessMode::ReadWrite,
                }],
                primary_directory: "project".to_owned(),
            },
        )
        .expect("update");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{}/configuration", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(configuration_body(
                    dir.path(),
                    "Stale",
                    record.revision,
                )))
                .unwrap(),
        )
        .await
        .expect("stale");
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        hypergraft::MEDIA_TYPE
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"agent-form\""));
    assert!(text.contains(AgentError::Conflict.message()));
    assert!(text.contains("After"));
    assert!(text.contains(&format!("name=\"revision\" value=\"{}\"", updated.revision)));
    let current = state.agents.get(&record.id).expect("current");
    assert_eq!(current.name, "After");
    assert_eq!(current.revision, updated.revision);
    state.keep_temp_dir(dir);
}

#[tokio::test]
async fn stale_delete_returns_conflict() {
    let state = test_state();
    let token = connected(&state);
    let (dir, record) = seed_agent(&state, "Kept");
    let updated = state
        .agents
        .update(
            &record.id,
            record.revision,
            AgentDraft {
                name: "Kept".to_owned(),
                instructions: "Newer".to_owned(),
                tools: vec![ToolId::List],
                network: NetworkAccess::None,
                directories: vec![DirectoryGrant {
                    alias: "project".to_owned(),
                    host_path: dir.path().to_path_buf(),
                    access: AccessMode::ReadWrite,
                }],
                primary_directory: "project".to_owned(),
            },
        )
        .expect("update");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{}/delete", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!("revision={}", record.revision)))
                .unwrap(),
        )
        .await
        .expect("stale");
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        hypergraft::MEDIA_TYPE
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"agent-form\""));
    assert!(text.contains(AgentError::Conflict.message()));
    assert!(text.contains(&format!("name=\"revision\" value=\"{}\"", updated.revision)));
    let current = state.agents.get(&record.id).expect("current");
    assert_eq!(current.revision, updated.revision);
    assert_eq!(current.instructions, "Newer");
    state.keep_temp_dir(dir);
}

fn encoded_path(path: &std::path::Path) -> String {
    path.canonicalize()
        .expect("canonical")
        .to_string_lossy()
        .replace(' ', "%20")
}

#[tokio::test]
async fn new_agent_document_renders_one_directory_row() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/agents/new")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("new agent");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert!(text.contains("id=\"agent-form\""));
    assert!(text.contains("name=\"alias_0\""));
    assert!(!text.contains("name=\"alias_1\""));
    assert!(text.contains("value=\"add-directory\""));
    assert!(text.contains("value=\"save\""));
    assert!(!text.contains("value=\"remove-directory:0\""));
}

#[tokio::test]
async fn configuration_document_renders_stored_directory_rows() {
    let state = test_state();
    let token = connected(&state);
    let (dir, record) = seed_agent(&state, "Reader");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/configuration", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("configuration");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("id=\"agent-form\""));
    assert!(text.contains("name=\"alias_0\""));
    assert!(!text.contains("name=\"alias_1\""));
    assert!(text.contains("value=\"add-directory\""));
    state.keep_temp_dir(dir);
}

#[tokio::test]
async fn native_add_directory_is_rejected_without_saving() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(
                    "intent=add-directory&name=Reader&instructions=Be+careful&primary=project&tool_list=on&alias_0=project&path_0=%2Ftmp%2Fapp&access_0=read-write",
                ))
                .unwrap(),
        )
        .await
        .expect("add");
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert!(state.agents.list().is_empty());
}

#[tokio::test]
async fn add_directory_returns_an_agent_form_patch() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(
                    "intent=add-directory&name=Reader&instructions=&primary=project&tool_list=on&alias_0=project&path_0=%2Ftmp%2Fapp&access_0=read-write",
                ))
                .unwrap(),
        )
        .await
        .expect("add patch");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        hypergraft::MEDIA_TYPE
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"agent-form\""));
    assert!(text.contains("name=\"alias_1\""));
    assert!(state.agents.list().is_empty());
}

fn git_worktree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("dir");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .expect("git")
            .success()
    );
    dir
}

fn keep_dir(state: &AppState, dir: tempfile::TempDir) {
    state.keep_temp_dir(dir);
}

#[tokio::test]
async fn add_directory_preserves_project_query_context() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let project_id = project.id.as_hex();
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents?project={project_id}"))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(
                    "intent=add-directory&name=Reader&instructions=&primary=project&tool_list=on&alias_0=project&access_0=read-write",
                ))
                .unwrap(),
        )
        .await
        .expect("add with project");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains(&format!("action=\"/agents?project={project_id}\"")));
    assert!(text.contains("name=\"alias_1\""));
    assert!(text.contains(&project.host_path.to_string_lossy().into_owned()));
    assert!(!text.contains("name=\"path_0\""));
    assert!(text.contains("name=\"path_1\""));
    keep_dir(&state, dir);
}

#[tokio::test]
async fn starter_form_omits_the_project_path_field() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/new?project={}", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("starter form");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains(&format!(
        "action=\"/agents?project={}\"",
        project.id.as_hex()
    )));
    assert!(text.contains("value=\"Desk\""));
    assert!(text.contains(&project.host_path.to_string_lossy().into_owned()));
    assert!(!text.contains("name=\"path_0\""));
    assert!(text.contains("name=\"alias_0\""));
    assert!(text.contains("name=\"tool_write\""));
    keep_dir(&state, dir);
}

#[tokio::test]
async fn invalid_and_missing_starter_projects_redirect_to_the_catalogue() {
    let state = test_state();
    let token = connected(&state);
    for project in ["not-an-id", "0123456789abcdef0123456789abcdef"] {
        let response = app(&state)
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/new?project={project}"))
                    .header(header::COOKIE, cookie(&token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("invalid starter");
        assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/projects"
        );
    }
}

#[tokio::test]
async fn starter_create_ignores_a_submitted_host_path() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let other = tempfile::tempdir().expect("other");
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let forged = encoded_path(other.path());
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents?project={}", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "intent=save&name=Worker&instructions=&primary=project&tool_list=on&alias_0=project&path_0={forged}&access_0=read-write"
                )))
                .unwrap(),
        )
        .await
        .expect("starter create");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    let agents = state.agents.list();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "Worker");
    assert_eq!(agents[0].directories.len(), 1);
    assert_eq!(agents[0].directories[0].host_path, project.host_path);
    assert!(text.contains(&format!(
        "navigate=\"{}\"",
        crate::projects::desk_path(&project.id, &agents[0].id)
    )));
    keep_dir(&state, dir);
    keep_dir(&state, other);
}

#[tokio::test]
async fn starter_create_keeps_an_extra_context_grant() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let extra = tempfile::tempdir().expect("extra");
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let extra_path = encoded_path(extra.path());
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents?project={}", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "intent=save&name=Worker&instructions=&primary=project&tool_list=on&alias_0=project&access_0=read-write&alias_1=docs&path_1={extra_path}&access_1=read-only"
                )))
                .unwrap(),
        )
        .await
        .expect("starter extra");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let agents = state.agents.list();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].directories.len(), 2);
    assert_eq!(agents[0].directories[0].host_path, project.host_path);
    assert_eq!(
        agents[0].directories[1].host_path,
        extra.path().canonicalize().expect("canonical")
    );
    assert_eq!(agents[0].directories[1].alias, "docs");
    keep_dir(&state, dir);
    keep_dir(&state, extra);
}

#[tokio::test]
async fn remove_directory_on_configuration_keeps_revision_and_does_not_save() {
    let state = test_state();
    let token = connected(&state);
    let first = tempfile::tempdir().expect("first");
    let second = tempfile::tempdir().expect("second");
    let record = state
        .agents
        .create(AgentDraft {
            name: "Two".to_owned(),
            instructions: "Stay".to_owned(),
            tools: vec![ToolId::List],
            network: NetworkAccess::None,
            directories: vec![
                DirectoryGrant {
                    alias: "project".to_owned(),
                    host_path: first.path().to_path_buf(),
                    access: AccessMode::ReadWrite,
                },
                DirectoryGrant {
                    alias: "docs".to_owned(),
                    host_path: second.path().to_path_buf(),
                    access: AccessMode::ReadOnly,
                },
            ],
            primary_directory: "project".to_owned(),
        })
        .expect("agent");
    let first_path = encoded_path(first.path());
    let second_path = encoded_path(second.path());
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{}/configuration", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "intent=remove-directory:0&name=Two&instructions=Stay&primary=project&tool_list=on&alias_0=project&path_0={first_path}&access_0=read-write&alias_1=docs&path_1={second_path}&access_1=read-only&revision={}",
                    record.revision
                )))
                .unwrap(),
        )
        .await
        .expect("remove");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"agent-form\""));
    assert!(text.contains("name=\"alias_0\""));
    assert!(text.contains("value=\"docs\""));
    assert!(!text.contains("name=\"alias_1\""));
    assert!(text.contains(&format!("name=\"revision\" value=\"{}\"", record.revision)));
    let current = state.agents.get(&record.id).expect("current");
    assert_eq!(current.directories.len(), 2);
    assert_eq!(current.revision, record.revision);
    state.keep_temp_dir(first);
    state.keep_temp_dir(second);
}

#[tokio::test]
async fn save_validation_preserves_submitted_form_state() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(
                    "intent=save&name=Reader&instructions=Be+careful&primary=project&tool_list=on&alias_0=project&path_0=relative%2Fpath&access_0=read-write",
                ))
                .unwrap(),
        )
        .await
        .expect("invalid save");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"agent-form\""));
    assert!(text.contains(AgentError::Path.message()));
    assert!(text.contains("value=\"Reader\""));
    assert!(text.contains("Be careful"));
    assert!(text.contains("value=\"relative/path\""));
    assert!(state.agents.list().is_empty());
}

#[tokio::test]
async fn sparse_directory_rows_are_rejected_without_saving() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(
                    "intent=save&name=Reader&instructions=&primary=project&tool_list=on&alias_0=project&path_0=%2Ftmp%2Fapp&access_0=read-write&alias_2=docs&path_2=%2Ftmp%2Fdocs&access_2=read-only",
                ))
                .unwrap(),
        )
        .await
        .expect("sparse");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"agent-form\""));
    assert!(text.contains("That form row is not valid."));
    assert!(state.agents.list().is_empty());
}

#[tokio::test]
async fn anonymous_agent_requests_redirect_to_connect() {
    let state = test_state();
    let cases = [
        ("GET", "/agents", None, false),
        ("GET", "/agents", Some("navigation"), true),
        ("GET", "/agents", Some("patch"), true),
        ("POST", "/agents", None, false),
        ("POST", "/agents", Some("patch"), true),
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
    if method == "POST" && graft.is_none() {
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    } else if enhanced {
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
