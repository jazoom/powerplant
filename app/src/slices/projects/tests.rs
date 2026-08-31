use std::path::Path;

use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    config::RuntimeConfig,
    projects::ProjectError,
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
async fn create_redirects_to_the_project_detail() {
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
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.starts_with("/projects/"));
    assert!(!location.contains("/configuration"));
    assert_eq!(state.projects.list().len(), 1);
    assert_eq!(state.projects.list()[0].name, "Desk");
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
    assert!(text.contains("<!doctype html>"));
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
