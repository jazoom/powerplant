use std::path::Path;

use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    agents::{AccessMode, AgentDraft, AgentError, DirectoryGrant, ToolId},
    config::RuntimeConfig,
    projects::ProjectError,
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

fn git_init(dir: &Path) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .expect("git")
            .success()
    );
}

fn git_worktree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("dir");
    git_init(dir.path());
    dir
}

fn encoded_path(dir: &tempfile::TempDir) -> String {
    dir.path()
        .canonicalize()
        .expect("canonical")
        .to_string_lossy()
        .replace(' ', "%20")
}

async fn body_text(response: axum::http::Response<Body>) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[tokio::test]
async fn a_catalogue_document_uses_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/projects")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("catalogue");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("<!doctype html>"));
    assert_eq!(text.matches("id=\"chat-main\"").count(), 1);
    assert!(text.contains("href=\"/projects/new\""));
    assert!(text.contains("data-graft"));
}

#[tokio::test]
async fn a_chat_document_enhances_provider_navigation() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/projects")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("catalogue");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    let connect_tags: Vec<&str> = text
        .split("<a ")
        .filter(|chunk| chunk.contains("href=\"/connect\""))
        .map(|chunk| chunk.split('>').next().expect("tag"))
        .collect();
    assert_eq!(connect_tags.len(), 2);
    for tag in connect_tags {
        assert!(tag.contains("data-graft"));
        assert!(tag.contains("data-nav=\"providers\""));
    }
}

#[tokio::test]
async fn a_catalogue_navigation_patches_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/projects")
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "navigation")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("navigation");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("operation=\"children\" target=\"chat-main\""));
}

#[tokio::test]
async fn a_catalogue_patch_is_rejected() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/projects")
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
async fn root_redirects_an_empty_catalogue_to_new_project() {
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
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/projects/new"
    );
}

#[tokio::test]
async fn root_redirects_one_project_to_its_detail() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    create_agent(&state, "Worker", &project.host_path);
    keep_dir(&state, dir);
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
        format!("/projects/{}", project.id.as_hex()).as_str()
    );
}

#[tokio::test]
async fn root_redirects_multiple_projects_to_the_catalogue() {
    let state = test_state();
    let token = connected(&state);
    let first = git_worktree();
    let second = git_worktree();
    state
        .projects
        .create("Harbour".to_owned(), first.path().to_path_buf())
        .expect("first");
    state
        .projects
        .create("Quay".to_owned(), second.path().to_path_buf())
        .expect("second");
    keep_dir(&state, first);
    keep_dir(&state, second);
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
        "/projects"
    );
}

#[tokio::test]
async fn the_catalogue_orders_recent_session_projects_first() {
    let state = test_state();
    let token = connected(&state);
    let first_dir = git_worktree();
    let second_dir = git_worktree();
    state
        .projects
        .create("Harbour".to_owned(), first_dir.path().to_path_buf())
        .expect("first");
    let second = state
        .projects
        .create("Quay".to_owned(), second_dir.path().to_path_buf())
        .expect("second");
    keep_dir(&state, first_dir);
    keep_dir(&state, second_dir);
    let session = sessions::SessionId::from_validated(
        &sessions::ValidatedToken::parse(&token).expect("session token"),
    );
    state.sessions.remember_conversation(
        &session,
        sessions::ConversationKey {
            project_id: second.id,
            agent_id: crate::agents::AgentId::generate().expect("agent"),
        },
    );
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/projects")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("catalogue");
    let text = body_text(response).await;
    let quay = text.find("Quay").expect("recent");
    let harbour = text.find("Harbour").expect("other");
    assert!(quay < harbour);
}

#[tokio::test]
async fn a_native_create_is_rejected_before_domain_work() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let encoded = encoded_path(&dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("name=Desk&path={encoded}")))
                .unwrap(),
        )
        .await
        .expect("create");
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert!(state.projects.list().is_empty());
}

#[tokio::test]
async fn enhanced_create_navigates_to_the_project_detail() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let encoded = encoded_path(&dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!("name=Desk&path={encoded}")))
                .unwrap(),
        )
        .await
        .expect("create");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("<graft-patch-set version=\"1\" navigate=\"/projects/"));
    assert_eq!(state.projects.list().len(), 1);
}

#[tokio::test]
async fn create_validation_returns_unprocessable_project_form_patch() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("name=Desk&path=relative/project"))
                .unwrap(),
        )
        .await
        .expect("invalid");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = body_text(response).await;
    assert!(text.contains("target=\"project-form\""));
    assert!(text.contains(ProjectError::Path.message()));
    assert!(text.contains("value=\"Desk\""));
    assert!(text.contains("value=\"relative/project\""));
    assert!(state.projects.list().is_empty());
}

#[tokio::test]
async fn create_rejects_an_unsupported_worktree() {
    let state = test_state();
    let token = connected(&state);
    let dir = tempfile::tempdir().expect("dir");
    let encoded = dir.path().to_string_lossy().replace(' ', "%20");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!("name=Desk&path={encoded}")))
                .unwrap(),
        )
        .await
        .expect("unsupported");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = body_text(response).await;
    assert!(text.contains("target=\"project-form\""));
    assert!(text.contains(ProjectError::Worktree.message()));
    assert!(state.projects.list().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn create_rejects_a_symlink_alias_of_a_stored_path() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    state
        .projects
        .create("One".to_owned(), dir.path().to_path_buf())
        .expect("first");
    let parent = tempfile::tempdir().expect("parent");
    let alias = parent.path().join("alias");
    std::os::unix::fs::symlink(dir.path(), &alias).expect("symlink");
    let encoded = alias.to_string_lossy().replace(' ', "%20");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!("name=Two&path={encoded}")))
                .unwrap(),
        )
        .await
        .expect("duplicate");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = body_text(response).await;
    assert!(text.contains(ProjectError::DuplicatePath.message()));
    assert_eq!(state.projects.list().len(), 1);
}

#[tokio::test]
async fn detail_shows_name_path_and_availability() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let record = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("create");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("Desk"));
    assert!(text.contains(&record.host_path.to_string_lossy().into_owned()));
    assert!(text.contains("Available"));
    assert!(text.contains(&format!(
        "href=\"/projects/{}/configuration\"",
        record.id.as_hex()
    )));
}

#[tokio::test]
async fn detail_keeps_an_unavailable_record_visible() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let record = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("create");
    let path = record.host_path.to_string_lossy().into_owned();
    drop(dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("Desk"));
    assert!(text.contains(&path));
    assert!(text.contains("not available on the host"));
    assert!(state.projects.get(&record.id).is_some());
}

#[tokio::test]
async fn missing_project_redirects_to_the_catalogue() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/projects/0123456789abcdef0123456789abcdef")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("missing");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/projects"
    );
}

#[tokio::test]
async fn configuration_omits_a_path_field() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let record = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("create");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}/configuration", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("configuration");
    let text = body_text(response).await;
    assert!(text.contains(&record.host_path.to_string_lossy().into_owned()));
    assert!(!text.contains("name=\"path\""));
    assert!(text.contains("name=\"name\""));
}

#[tokio::test]
async fn rename_ignores_a_submitted_path_and_updates_the_name() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let other = git_worktree();
    let record = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("create");
    let original = record.host_path.clone();
    let encoded = encoded_path(&other);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/configuration", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "name=Later&path={encoded}&revision={}",
                    record.revision
                )))
                .unwrap(),
        )
        .await
        .expect("rename");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let updated = state.projects.get(&record.id).expect("updated");
    assert_eq!(updated.name, "Later");
    assert_eq!(updated.host_path, original);
    assert_eq!(updated.revision, record.revision + 1);
}

#[tokio::test]
async fn stale_rename_returns_conflict() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let record = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("create");
    state
        .projects
        .update_name(&record.id, record.revision, "Later".to_owned())
        .expect("rename");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/configuration", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "name=Stale&revision={}",
                    record.revision
                )))
                .unwrap(),
        )
        .await
        .expect("stale");
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let text = body_text(response).await;
    assert!(text.contains("target=\"project-form\""));
    assert!(text.contains(ProjectError::Conflict.message()));
    assert!(text.contains("Later"));
    assert_eq!(
        state.projects.get(&record.id).expect("current").name,
        "Later"
    );
}

#[tokio::test]
async fn a_detail_navigation_patches_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let record = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("create");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}", record.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "navigation")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("navigation");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("operation=\"children\" target=\"chat-main\""));
    assert!(text.contains("Desk"));
}

fn keep_dir(state: &AppState, dir: tempfile::TempDir) {
    state.keep_temp_dir(dir);
}

fn create_agent(state: &AppState, name: &str, path: &Path) -> crate::agents::AgentRecord {
    state
        .agents
        .create(AgentDraft {
            name: name.to_owned(),
            instructions: String::new(),
            tools: vec![ToolId::List],
            directories: vec![DirectoryGrant {
                alias: "project".to_owned(),
                host_path: path.to_path_buf(),
                access: AccessMode::ReadWrite,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("agent")
}

#[tokio::test]
async fn one_eligible_agent_redirects_to_the_desk() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Worker", &project.host_path);
    keep_dir(&state, dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        crate::projects::desk_path(&project.id, &agent.id).as_str()
    );
}

#[tokio::test]
async fn a_desk_document_uses_the_project_title() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Worker", &project.host_path);
    keep_dir(&state, dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(crate::projects::desk_path(&project.id, &agent.id))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("desk");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("<!doctype html>"));
    assert_eq!(text.matches("id=\"chat-main\"").count(), 1);
    assert!(text.contains("Desk"));
    assert!(text.contains("Worker"));
    assert!(text.contains(&project.host_path.to_string_lossy().into_owned()));
}

#[tokio::test]
async fn a_desk_navigation_patches_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Worker", &project.host_path);
    keep_dir(&state, dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(crate::projects::desk_path(&project.id, &agent.id))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "navigation")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("navigation");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("operation=\"children\" target=\"chat-main\""));
    assert!(text.contains("Desk"));
}

#[tokio::test]
async fn a_desk_patch_refreshes_job_observe() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Worker", &project.host_path);
    keep_dir(&state, dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(crate::projects::desk_path(&project.id, &agent.id))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("patch");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("target=\"job-observe\""));
}

#[tokio::test]
async fn an_ineligible_agent_cannot_open_the_desk() {
    let state = test_state();
    let token = connected(&state);
    let project_dir = git_worktree();
    let agent_dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), project_dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Other", agent_dir.path());
    keep_dir(&state, project_dir);
    keep_dir(&state, agent_dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(crate::projects::desk_path(&project.id, &agent.id))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("ineligible");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        format!("/projects/{}", project.id.as_hex()).as_str()
    );
}

#[tokio::test]
async fn a_stale_grant_cannot_open_the_desk() {
    let state = test_state();
    let token = connected(&state);
    let project_dir = git_worktree();
    let other = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), project_dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Worker", &project.host_path);
    let session = sessions::SessionId::from_validated(
        &sessions::ValidatedToken::parse(&token).expect("session token"),
    );
    state.sessions.remember_conversation(
        &session,
        sessions::ConversationKey {
            project_id: project.id,
            agent_id: agent.id,
        },
    );
    state
        .agents
        .update(
            &agent.id,
            agent.revision,
            AgentDraft {
                name: agent.name.clone(),
                instructions: agent.instructions.clone(),
                tools: agent.tools.clone(),
                directories: vec![DirectoryGrant {
                    alias: "project".to_owned(),
                    host_path: other.path().to_path_buf(),
                    access: AccessMode::ReadWrite,
                }],
                primary_directory: "project".to_owned(),
            },
        )
        .expect("update");
    keep_dir(&state, project_dir);
    keep_dir(&state, other);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(crate::projects::desk_path(&project.id, &agent.id))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("stale");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        format!("/projects/{}", project.id.as_hex()).as_str()
    );

    let detail = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("project detail");
    assert_eq!(detail.status(), axum::http::StatusCode::OK);
    assert!(state.sessions.last_agent(&session, &project.id).is_none());
}

#[tokio::test]
async fn two_eligible_agents_render_canonical_desk_links() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let first = create_agent(&state, "First", &project.host_path);
    let second = create_agent(&state, "Second", &project.host_path);
    keep_dir(&state, dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("choice");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("Open with an agent"));
    assert!(text.contains(&crate::projects::desk_path(&project.id, &first.id)));
    assert!(text.contains(&crate::projects::desk_path(&project.id, &second.id)));
    assert!(text.contains("data-graft"));
}

#[tokio::test]
async fn a_remembered_eligible_agent_is_preferred() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let first = create_agent(&state, "First", &project.host_path);
    let _second = create_agent(&state, "Second", &project.host_path);
    keep_dir(&state, dir);
    let opened = app(&state)
        .oneshot(
            Request::builder()
                .uri(crate::projects::desk_path(&project.id, &first.id))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("desk");
    assert_eq!(opened.status(), axum::http::StatusCode::OK);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        crate::projects::desk_path(&project.id, &first.id).as_str()
    );
}

#[tokio::test]
async fn no_agent_detail_shows_starter_and_grant_actions() {
    let state = test_state();
    let token = connected(&state);
    let project_dir = git_worktree();
    let agent_dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), project_dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Other", agent_dir.path());
    keep_dir(&state, project_dir);
    keep_dir(&state, agent_dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("No agent"));
    assert!(text.contains(&format!(
        "action=\"/projects/{}/agents/starter\"",
        project.id.as_hex()
    )));
    assert!(text.contains("Create starter agent"));
    assert!(text.contains("Your project stays unchanged until you apply the proposed changes."));
    assert!(text.contains("Set custom permissions instead"));
    assert!(text.contains(&format!(
        "href=\"/agents/new?project={}\"",
        project.id.as_hex()
    )));
    assert!(text.contains(&format!(
        "action=\"/projects/{}/agents/grant\"",
        project.id.as_hex()
    )));
    assert!(text.contains(&agent.name));
    assert!(text.contains(&format!(
        "name=\"agent_id\" value=\"{}\"",
        agent.id.as_hex()
    )));
    assert!(!text.contains("name=\"path\""));
}

#[tokio::test]
async fn grant_redirects_to_the_canonical_desk() {
    let state = test_state();
    let token = connected(&state);
    let project_dir = git_worktree();
    let agent_dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), project_dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Other", agent_dir.path());
    keep_dir(&state, project_dir);
    keep_dir(&state, agent_dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/agents/grant", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "agent_id={}&revision={}&alias=code&access=read-write",
                    agent.id.as_hex(),
                    agent.revision
                )))
                .unwrap(),
        )
        .await
        .expect("grant");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains(&format!(
        "navigate=\"{}\"",
        crate::projects::desk_path(&project.id, &agent.id)
    )));
    let updated = state.agents.get(&agent.id).expect("updated");
    assert_eq!(updated.directories.len(), 2);
    assert_eq!(updated.directories[1].alias, "code");
    assert_eq!(updated.directories[1].host_path, project.host_path);
    assert_eq!(updated.primary_directory, "project");
}

#[tokio::test]
async fn grant_ignores_a_submitted_host_path() {
    let state = test_state();
    let token = connected(&state);
    let project_dir = git_worktree();
    let agent_dir = git_worktree();
    let forged = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), project_dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Other", agent_dir.path());
    let encoded = encoded_path(&forged);
    keep_dir(&state, project_dir);
    keep_dir(&state, agent_dir);
    keep_dir(&state, forged);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/agents/grant", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "agent_id={}&revision={}&alias=code&access=read-only&path={encoded}",
                    agent.id.as_hex(),
                    agent.revision
                )))
                .unwrap(),
        )
        .await
        .expect("grant");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let updated = state.agents.get(&agent.id).expect("updated");
    assert_eq!(updated.directories.len(), 2);
    assert_eq!(updated.directories[1].host_path, project.host_path);
    assert_eq!(updated.directories[1].access, AccessMode::ReadOnly);
}

#[tokio::test]
async fn stale_grant_returns_conflict() {
    let state = test_state();
    let token = connected(&state);
    let project_dir = git_worktree();
    let agent_dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), project_dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Other", agent_dir.path());
    state
        .agents
        .update(
            &agent.id,
            agent.revision,
            AgentDraft {
                name: "Later".to_owned(),
                instructions: agent.instructions.clone(),
                tools: agent.tools.clone(),
                directories: agent.directories.clone(),
                primary_directory: agent.primary_directory.clone(),
            },
        )
        .expect("update");
    keep_dir(&state, project_dir);
    keep_dir(&state, agent_dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/agents/grant", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "agent_id={}&revision={}&alias=project&access=read-write",
                    agent.id.as_hex(),
                    agent.revision
                )))
                .unwrap(),
        )
        .await
        .expect("stale grant");
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let text = body_text(response).await;
    assert!(text.contains("target=\"chat-main\""));
    assert!(text.contains(AgentError::Conflict.message()));
    let current = state.agents.get(&agent.id).expect("current");
    assert_eq!(current.name, "Later");
    assert_eq!(current.directories.len(), 1);
}

#[tokio::test]
async fn grant_duplicate_alias_is_rejected() {
    let state = test_state();
    let token = connected(&state);
    let project_dir = git_worktree();
    let agent_dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), project_dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Other", agent_dir.path());
    keep_dir(&state, project_dir);
    keep_dir(&state, agent_dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/agents/grant", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "agent_id={}&revision={}&alias=project&access=read-write",
                    agent.id.as_hex(),
                    agent.revision
                )))
                .unwrap(),
        )
        .await
        .expect("duplicate alias");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = body_text(response).await;
    assert!(text.contains("target=\"chat-main\""));
    assert!(text.contains(AgentError::DuplicateAlias.message()));
    assert_eq!(
        state
            .agents
            .get(&agent.id)
            .expect("current")
            .directories
            .len(),
        1
    );
}

#[tokio::test]
async fn enhanced_grant_navigates_to_the_desk() {
    let state = test_state();
    let token = connected(&state);
    let project_dir = git_worktree();
    let agent_dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), project_dir.path().to_path_buf())
        .expect("project");
    let agent = create_agent(&state, "Other", agent_dir.path());
    keep_dir(&state, project_dir);
    keep_dir(&state, agent_dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/agents/grant", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "agent_id={}&revision={}&alias=code&access=read-write",
                    agent.id.as_hex(),
                    agent.revision
                )))
                .unwrap(),
        )
        .await
        .expect("enhanced grant");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains(&format!(
        "navigate=\"{}\"",
        crate::projects::desk_path(&project.id, &agent.id)
    )));
}

#[tokio::test]
async fn starter_creates_exact_path_authority_and_opens_the_desk() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let forged = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let encoded = encoded_path(&forged);
    keep_dir(&state, dir);
    keep_dir(&state, forged);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/agents/starter", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!("path={encoded}")))
                .unwrap(),
        )
        .await
        .expect("starter");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let agents = state.agents.list();
    assert_eq!(agents.len(), 1);
    let agent = &agents[0];
    assert_eq!(agent.name, project.name);
    assert!(agent.instructions.is_empty());
    assert_eq!(agent.tools, ToolId::ALL.to_vec());
    assert_eq!(agent.directories.len(), 1);
    assert_eq!(agent.directories[0].alias, "project");
    assert_eq!(agent.directories[0].host_path, project.host_path);
    assert_eq!(agent.directories[0].access, AccessMode::ReadWrite);
    assert_eq!(agent.primary_directory, "project");
    let text = body_text(response).await;
    assert!(text.contains(&format!(
        "navigate=\"{}\"",
        crate::projects::desk_path(&project.id, &agent.id)
    )));
}

#[cfg(unix)]
#[tokio::test]
async fn starter_rejects_a_project_path_that_now_resolves_elsewhere() {
    let state = test_state();
    let token = connected(&state);
    let parent = tempfile::tempdir().expect("parent");
    let original = parent.path().join("project");
    std::fs::create_dir(&original).expect("project");
    git_init(&original);
    let project = state
        .projects
        .create("Desk".to_owned(), original.clone())
        .expect("project");
    let moved = parent.path().join("moved");
    std::fs::rename(&original, &moved).expect("move project");
    std::os::unix::fs::symlink(&moved, &original).expect("replace project path");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/agents/starter", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("starter");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = body_text(response).await;
    assert!(text.contains("target=\"chat-main\""));
    assert!(text.contains(AgentError::PathAccess.message()));
    assert!(state.agents.list().is_empty());
}

#[tokio::test]
async fn a_repeated_starter_command_does_not_create_a_duplicate() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    keep_dir(&state, dir);
    let first = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/agents/starter", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("first starter");
    assert_eq!(first.status(), axum::http::StatusCode::OK);
    let created = state.agents.list();
    assert_eq!(created.len(), 1);
    let desk = crate::projects::desk_path(&project.id, &created[0].id);
    assert!(
        body_text(first)
            .await
            .contains(&format!("navigate=\"{desk}\""))
    );
    let second = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/agents/starter", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("second starter");
    assert_eq!(second.status(), axum::http::StatusCode::OK);
    assert_eq!(state.agents.list().len(), 1);
    assert_eq!(state.agents.list()[0].id, created[0].id);
    assert!(
        body_text(second)
            .await
            .contains(&format!("navigate=\"{desk}\""))
    );
}

#[tokio::test]
async fn concurrent_starter_commands_create_one_eligible_agent() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    keep_dir(&state, dir);
    let router = app(&state);
    let uri = format!("/projects/{}/agents/starter", project.id.as_hex());
    let request = || {
        Request::builder()
            .method("POST")
            .uri(&uri)
            .header(header::COOKIE, cookie(&token))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(hypergraft::GRAFT_REQUEST, "patch")
            .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
            .body(Body::empty())
            .unwrap()
    };
    let (first, second) =
        tokio::join!(router.clone().oneshot(request()), router.oneshot(request()),);
    let first = first.expect("first starter");
    let second = second.expect("second starter");
    assert_eq!(first.status(), axum::http::StatusCode::OK);
    assert_eq!(second.status(), axum::http::StatusCode::OK);
    let agents = state.agents.list();
    assert_eq!(agents.len(), 1);
    let desk = crate::projects::desk_path(&project.id, &agents[0].id);
    let marker = format!("navigate=\"{desk}\"");
    assert!(body_text(first).await.contains(&marker));
    assert!(body_text(second).await.contains(&marker));
}

#[tokio::test]
async fn several_eligible_agents_leave_the_catalogue_unchanged() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    let first = create_agent(&state, "First", &project.host_path);
    let second = create_agent(&state, "Second", &project.host_path);
    keep_dir(&state, dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/agents/starter", project.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("several");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains(&format!("navigate=\"/projects/{}\"", project.id.as_hex())));
    let agents = state.agents.list();
    assert_eq!(agents.len(), 2);
    assert!(agents.iter().any(|agent| agent.id == first.id));
    assert!(agents.iter().any(|agent| agent.id == second.id));
}

#[tokio::test]
async fn the_starter_command_rejects_native_post_document_and_navigation_use() {
    let state = test_state();
    let token = connected(&state);
    let dir = git_worktree();
    let project = state
        .projects
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("project");
    keep_dir(&state, dir);
    let uri = format!("/projects/{}/agents/starter", project.id.as_hex());
    let rejected = [
        (
            Request::builder()
                .method("POST")
                .uri(&uri)
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::empty())
                .unwrap(),
            axum::http::StatusCode::BAD_REQUEST,
        ),
        (
            Request::builder()
                .uri(&uri)
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
            axum::http::StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            Request::builder()
                .uri(&uri)
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "navigation")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
            axum::http::StatusCode::METHOD_NOT_ALLOWED,
        ),
    ];
    for (request, status) in rejected {
        let response = app(&state)
            .oneshot(request)
            .await
            .expect("rejected starter");
        assert_eq!(response.status(), status);
        assert!(state.agents.list().is_empty());
    }
    let created = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&uri)
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("accepted starter");
    assert_eq!(created.status(), axum::http::StatusCode::OK);
    assert_eq!(state.agents.list().len(), 1);
}

fn named_git_worktree(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let parent = tempfile::tempdir().expect("parent");
    let project = parent.path().join(name);
    std::fs::create_dir(&project).expect("project");
    git_init(&project);
    let canonical = project.canonicalize().expect("canonical");
    (parent, canonical)
}

#[tokio::test]
async fn new_project_defaults_to_the_folder_chooser() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/projects/new")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("new project");
    let text = body_text(response).await;
    assert!(text.contains("Choose project folder"));
    assert!(text.contains("action=\"/projects/folder\""));
    assert!(text.contains("href=\"/projects/new?entry=manual\""));
    assert!(text.contains("Enter path manually"));
    assert!(!text.contains("Add project"));
    assert!(!text.contains("Git project path"));
}

#[tokio::test]
async fn new_project_manual_entry_uses_the_canonical_query() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/projects/new?entry=manual")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("manual");
    let text = body_text(response).await;
    assert!(text.contains("Git project folder"));
    assert!(text.contains("/home/me/projects/my-app"));
    assert!(text.contains("Add project"));
    assert!(!text.contains("action=\"/projects/folder\""));
}

#[tokio::test]
async fn new_project_rejects_an_unknown_entry_query() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/projects/new?entry=selected")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("unknown entry");
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn folder_selection_fills_the_form_without_creating_a_project() {
    let state = test_state();
    let token = connected(&state);
    let (dir, path) = named_git_worktree("my-app");
    state.folder_picker.queue(Some(path.clone()));
    keep_dir(&state, dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects/folder")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("folder");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("target=\"project-form\""));
    assert!(text.contains("value=\"my-app\""));
    assert!(text.contains(&path.to_string_lossy().into_owned()));
    assert!(text.contains("Add project"));
    assert!(text.contains("Choose another folder"));
    assert!(state.projects.list().is_empty());
}

#[tokio::test]
async fn folder_selection_leaves_the_name_empty_when_the_component_is_invalid() {
    let state = test_state();
    let token = connected(&state);
    let (dir, path) = named_git_worktree(&"a".repeat(81));
    state.folder_picker.queue(Some(path));
    keep_dir(&state, dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects/folder")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("folder");
    let text = body_text(response).await;
    assert!(text.contains("name=\"name\""));
    assert!(text.contains("value=\"\""));
    assert!(state.projects.list().is_empty());
}

#[tokio::test]
async fn folder_cancellation_preserves_the_submitted_draft() {
    let state = test_state();
    let token = connected(&state);
    state.folder_picker.queue(None);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects/folder")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("name=Harbour&path=/srv/harbour"))
                .unwrap(),
        )
        .await
        .expect("cancel");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("target=\"project-form\""));
    assert!(text.contains("value=\"Harbour\""));
    assert!(text.contains("value=\"/srv/harbour\""));
    assert!(!text.contains("Another project folder chooser is open."));
    assert!(state.projects.list().is_empty());
}

#[tokio::test]
async fn a_busy_folder_chooser_returns_conflict_and_keeps_the_form() {
    let state = test_state();
    let token = connected(&state);
    let _hold = state.folder_picker.occupy().expect("permit");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects/folder")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("name=Harbour&path=/srv/harbour"))
                .unwrap(),
        )
        .await
        .expect("busy");
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let text = body_text(response).await;
    assert!(text.contains("target=\"project-form\""));
    assert!(text.contains("Another project folder chooser is open."));
    assert!(text.contains("value=\"Harbour\""));
    assert!(text.contains("value=\"/srv/harbour\""));
    assert!(state.projects.list().is_empty());
}

#[tokio::test]
async fn folder_validation_returns_project_error_copy_without_creating() {
    let state = test_state();
    let token = connected(&state);
    let dir = tempfile::tempdir().expect("dir");
    state.folder_picker.queue(Some(dir.path().to_path_buf()));
    keep_dir(&state, dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects/folder")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("invalid folder");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = body_text(response).await;
    assert!(text.contains("target=\"project-form\""));
    assert!(text.contains(ProjectError::Worktree.message()));
    assert!(text.contains("Choose project folder"));
    assert!(state.projects.list().is_empty());
}

#[tokio::test]
async fn the_folder_command_rejects_other_representations_before_selection() {
    let state = test_state();
    let token = connected(&state);
    for (graft, name) in [
        (None, "document-project"),
        (Some("navigation"), "navigation-project"),
    ] {
        let (dir, path) = named_git_worktree(name);
        state.folder_picker.queue(Some(path.clone()));
        keep_dir(&state, dir);

        let mut request = Request::builder()
            .method("POST")
            .uri("/projects/folder")
            .header(header::COOKIE, cookie(&token))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(graft) = graft {
            request = request
                .header(hypergraft::GRAFT_REQUEST, graft)
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
        }
        let rejected = app(&state)
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .expect("rejected folder");
        assert_eq!(rejected.status(), axum::http::StatusCode::BAD_REQUEST);

        let accepted = app(&state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/projects/folder")
                    .header(header::COOKIE, cookie(&token))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header(hypergraft::GRAFT_REQUEST, "patch")
                    .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("accepted folder");
        assert_eq!(accepted.status(), axum::http::StatusCode::OK);
        assert!(
            body_text(accepted)
                .await
                .contains(&path.to_string_lossy().into_owned())
        );
        assert!(state.projects.list().is_empty());
    }
}

#[tokio::test]
async fn create_rejects_a_forged_path_after_folder_selection() {
    let state = test_state();
    let token = connected(&state);
    let (dir, path) = named_git_worktree("my-app");
    state.folder_picker.queue(Some(path));
    keep_dir(&state, dir);
    let selected = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects/folder")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("folder");
    assert_eq!(selected.status(), axum::http::StatusCode::OK);
    assert!(state.projects.list().is_empty());
    let forged = tempfile::tempdir().expect("forged");
    let encoded = forged.path().to_string_lossy().replace(' ', "%20");
    keep_dir(&state, forged);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "name=Desk&path={encoded}&entry=selected"
                )))
                .unwrap(),
        )
        .await
        .expect("forged create");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = body_text(response).await;
    assert!(text.contains("target=\"project-form\""));
    assert!(text.contains(ProjectError::Worktree.message()));
    assert!(state.projects.list().is_empty());
}
