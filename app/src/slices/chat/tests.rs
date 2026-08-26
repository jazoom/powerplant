use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    agents::{AccessMode, AgentDraft, DirectoryGrant, DirectoryPolicy, ToolId},
    config::RuntimeConfig,
    providers::{
        ChatBackend, ProviderConnection, ProviderError, ProviderKind, Role,
        scripted::ScriptedBackend,
    },
    sandbox::{GuestAccess, SandboxSpec},
    sessions::{self, JobStatus},
    state::AppState,
};

use super::job::MAXIMUM_REPLY_BYTES;

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

fn state_with_backend(backend: ScriptedBackend) -> AppState {
    let mut state = test_state();
    state.chat = Arc::new(ChatBackend::Scripted(backend));
    state
}

fn agent_hex(state: &AppState) -> String {
    state.agents.list()[0].id.as_hex()
}

fn agent_id(state: &AppState) -> crate::agents::AgentId {
    state.agents.list()[0].id
}

fn chat_path(state: &AppState) -> String {
    format!("/agents/{}", agent_hex(state))
}

async fn connected(state: &AppState) -> String {
    connected_with(state, true).await
}

async fn connected_idle(state: &AppState) -> String {
    connected_with(state, false).await
}

async fn connected_with(state: &AppState, start_guest: bool) -> String {
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
    let dir = tempfile::tempdir().expect("project");
    let record = state
        .agents
        .create(AgentDraft {
            name: "Test agent".to_owned(),
            instructions: String::new(),
            tools: ToolId::ALL.to_vec(),
            directories: vec![DirectoryGrant {
                alias: "project".to_owned(),
                host_path: dir.path().to_path_buf(),
                access: AccessMode::ReadWrite,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("agent");
    let sandbox = state.sandboxes.handle(record.id);
    let policy = DirectoryPolicy::from_record(&record);
    if start_guest {
        let access = state
            .vault
            .selected_connection()
            .as_ref()
            .map(GuestAccess::from_connection)
            .unwrap_or_default();
        sandbox
            .start_with(SandboxSpec::from_policy(&policy, access))
            .await
            .expect("start");
        sandbox.complete_start();
    }
    state
        .scratch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(dir);
    token.raw().as_str().to_owned()
}

fn patch_send_message(state: &AppState, token: &str, message: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(chat_path(state))
        .header(header::COOKIE, cookie(token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from(format!("message={message}")))
        .unwrap()
}

fn session_snapshot(state: &AppState, token: &str) -> sessions::SessionSnapshot {
    let validated = sessions::ValidatedToken::parse(token).expect("token");
    state
        .sessions
        .snapshot(
            &sessions::SessionId::from_validated(&validated),
            &agent_id(state),
        )
        .expect("session")
}

fn stream_frames(body: &[u8]) -> Vec<String> {
    let mut frames = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let newline = rest
            .iter()
            .position(|&b| b == b'\n')
            .expect("length prefix");
        let len: usize = std::str::from_utf8(&rest[..newline])
            .expect("length utf8")
            .parse()
            .expect("length");
        let start = newline + 1;
        let end = start + len;
        frames.push(String::from_utf8(rest[start..end].to_vec()).expect("frame utf8"));
        rest = &rest[end..];
    }
    frames
}

fn cookie(token: &str) -> String {
    format!("powerplant_session={token}")
}

fn patch_send(state: &AppState, token: &str) -> Request<Body> {
    patch_send_message(state, token, "Hello")
}

fn document_show(state: &AppState, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(chat_path(state))
        .header(header::COOKIE, cookie(token))
        .body(Body::empty())
        .unwrap()
}

fn navigation_show(state: &AppState, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(chat_path(state))
        .header(header::COOKIE, cookie(token))
        .header(hypergraft::GRAFT_REQUEST, "navigation")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .unwrap()
}

fn observe_patch(state: &AppState, token: &str, job: &str, cursor: u64) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("{}?job={job}&cursor={cursor}", chat_path(state)))
        .header(header::COOKIE, cookie(token))
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .unwrap()
}

fn model_refresh_patch(token: &str) -> Request<Body> {
    Request::builder()
        .uri("/model")
        .header(header::COOKIE, cookie(token))
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .unwrap()
}

fn model_update_patch(token: &str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/model")
        .header(header::COOKIE, cookie(token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from(body))
        .unwrap()
}

fn job_id_from_body(body: &axum::body::Bytes) -> String {
    job_id_from(std::str::from_utf8(body).expect("utf8"))
}

fn job_id_from(html: &str) -> String {
    let marker = "name=\"job\" value=\"";
    let start = html.find(marker).expect("job field") + marker.len();
    let end = html[start..].find('"').expect("job field end") + start;
    html[start..end].to_owned()
}

fn cursor_from(html: &str) -> u64 {
    let marker = "name=\"cursor\" value=\"";
    let start = html.rfind(marker).expect("cursor field") + marker.len();
    let end = html[start..].find('"').expect("cursor field end") + start;
    html[start..end].parse().expect("cursor")
}

fn job_active(html: &str) -> bool {
    html.contains("data-job-active=\"true\"")
}

async fn wait_until_job_idle(state: &AppState, token: &str) {
    for _ in 0..2_000 {
        match session_snapshot(state, token).job {
            Some(job) if job.status != JobStatus::Running => return,
            None => return,
            _ => tokio::task::yield_now().await,
        }
    }
    panic!("job did not finish");
}

async fn wait_until_job_events(state: &AppState, token: &str, minimum: u64) {
    for _ in 0..2_000 {
        if session_snapshot(state, token)
            .job
            .is_some_and(|job| job.latest_seq >= minimum)
        {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("job did not reach {minimum} events");
}

#[tokio::test]
async fn a_document_show_returns_the_full_page() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(document_show(&state, &token))
        .await
        .expect("chat document");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(response.headers().get(hypergraft::GRAFT_TRANSFER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert_eq!(text.matches("id=\"chat-main\"").count(), 1);
    assert!(text.contains("id=\"chat-main\" tabindex=\"-1\""));
    assert!(text.contains("href=\"/agents\""));
    assert!(text.contains("data-graft"));
}

#[tokio::test]
async fn a_navigation_show_patches_chat_main_children() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(navigation_show(&state, &token))
        .await
        .expect("chat navigation");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        hypergraft::MEDIA_TYPE
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("<!doctype html>"));
    assert_eq!(text.matches("chat-main").count(), 1);
    assert!(text.contains("operation=\"children\" target=\"chat-main\""));
    assert!(!text.contains("id=\"chat-main\""));
}

#[tokio::test]
async fn a_patch_send_starts_a_job_without_streaming() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(response.headers().get(hypergraft::GRAFT_TRANSFER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("phase=\""));
    assert!(text.contains("name=\"job\""));
    assert!(text.contains("name=\"cursor\" value=\"0\""));
    assert!(text.contains("data-job-active=\"true\""));
    assert!(text.contains("turn-1"));
    assert!(text.contains("Refresh"));
    assert!(text.contains("Stop"));
    assert!(text.contains("target=\"job-observe\""));
    assert!(!text.contains("target=\"composer\""));
    assert!(!text.contains("composer-message"));

    let stored = session_snapshot(&state, &token);
    assert_eq!(
        stored.turns.first().map(|turn| turn.text.as_str()),
        Some("Hello")
    );
    assert!(stored.job.is_some());
}

#[tokio::test]
async fn an_empty_patch_send_stays_a_complete_unprocessable_response() {
    let state = test_state();
    let token = connected(&state).await;
    let path = chat_path(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::COOKIE, cookie(&token))
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
    assert!(text.contains("target=\"composer\""));
    assert!(!text.contains("target=\"transcript\""));
    assert!(!text.contains("phase=\""));
}

#[tokio::test]
async fn a_document_send_returns_the_page_before_the_job_finishes() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(chat_path(&state))
                .header(header::COOKIE, cookie(&token))
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
    assert!(text.contains("Hello"));
    assert!(text.contains("Refresh"));
    assert!(!text.contains("Hello from Power Plant."));
}

#[tokio::test]
async fn a_later_document_show_renders_the_finished_job() {
    let state = test_state();
    let token = connected(&state).await;
    let _ = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    wait_until_job_idle(&state, &token).await;

    let response = app(&state)
        .oneshot(document_show(&state, &token))
        .await
        .expect("chat document");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert!(text.contains("Hello from Power Plant."));
    assert!(!text.contains("data-job-active=\"true\""));

    let stored = session_snapshot(&state, &token);
    assert_eq!(
        stored.turns.last().map(|turn| turn.text.as_str()),
        Some("Hello from Power Plant.")
    );
}

#[tokio::test]
async fn a_later_document_show_drops_a_failed_job_error() {
    let state = state_with_backend(ScriptedBackend::chunks([Err(ProviderError::Detail(
        "You have insufficient credits".to_owned(),
    ))]));
    let token = connected(&state).await;
    let _ = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    wait_until_job_idle(&state, &token).await;

    let response = app(&state)
        .oneshot(document_show(&state, &token))
        .await
        .expect("chat document");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("You have insufficient credits"));
    assert!(!text.contains("data-job-active=\"true\""));
}

#[tokio::test]
async fn observation_streams_one_bounded_segment_with_one_final_frame() {
    let state = test_state();
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    wait_until_job_idle(&state, &token).await;

    let response = app(&state)
        .oneshot(observe_patch(&state, &token, &job, 0))
        .await
        .expect("observe");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response.headers().get(hypergraft::GRAFT_TRANSFER).unwrap(),
        "stream"
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let frames = stream_frames(&body);
    assert!(!frames.is_empty());
    assert!(frames.len() <= hypergraft::MAX_STREAM_FRAMES);
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame.contains("phase=\"final\""))
            .count(),
        1
    );
    let final_frame = frames.last().expect("final frame");
    assert!(final_frame.contains("phase=\"final\""));
    assert!(
        frames
            .iter()
            .any(|frame| frame.contains("Hello from Power Plant."))
    );
    assert!(!job_active(final_frame));
}

#[tokio::test]
async fn a_long_job_uses_repeated_observation_segments() {
    const EVENTS: usize = 300;
    let state = state_with_backend(ScriptedBackend::chunks(
        (0..EVENTS).map(|_| Ok("x".to_owned())),
    ));
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    wait_until_job_events(&state, &token, EVENTS as u64).await;

    let mut cursor = 0;
    let mut total_progress = 0usize;
    let mut segments = 0usize;
    let mut last_output = String::new();
    loop {
        let response = app(&state)
            .oneshot(observe_patch(&state, &token, &job, cursor))
            .await
            .expect("observe");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers().get(hypergraft::GRAFT_TRANSFER).unwrap(),
            "stream"
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(body.len() <= hypergraft::MAX_STREAM_BYTES);
        let frames = stream_frames(&body);
        assert!(frames.len() <= hypergraft::MAX_STREAM_FRAMES);
        assert!(
            frames
                .iter()
                .all(|frame| frame.len() <= hypergraft::MAX_RESPONSE_BYTES)
        );
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame.contains("phase=\"final\""))
                .count(),
            1
        );
        total_progress += frames
            .iter()
            .filter(|frame| frame.contains("phase=\"progress\""))
            .count();
        let final_frame = frames.last().expect("final frame");
        if let Some(progress) = frames
            .iter()
            .rev()
            .find(|frame| frame.contains("name=\"cursor\""))
        {
            cursor = cursor_from(progress);
        }
        if let Some(progress) = frames.iter().rev().find(|frame| frame.contains("turn-2")) {
            last_output = progress.clone();
        }
        segments += 1;
        if !job_active(final_frame) {
            break;
        }
        cursor = cursor_from(final_frame);
        assert!(segments < 8, "observation did not settle");
    }

    assert!(total_progress > hypergraft::MAX_STREAM_FRAMES);
    assert!(last_output.contains(&"x".repeat(EVENTS)));
    assert_eq!(cursor, EVENTS as u64);

    let retry = app(&state)
        .oneshot(observe_patch(&state, &token, &job, 1))
        .await
        .expect("retry");
    let retry_body = to_bytes(retry.into_body(), usize::MAX).await.unwrap();
    let retry_frames = stream_frames(&retry_body);
    let retry_progress = retry_frames
        .iter()
        .filter(|frame| frame.contains("phase=\"progress\""))
        .count();
    assert!(retry_progress > 0);
    assert_eq!(
        cursor_from(
            retry_frames
                .iter()
                .find(|frame| frame.contains("phase=\"progress\""))
                .unwrap()
        ),
        2
    );

    let stored = session_snapshot(&state, &token);
    assert_eq!(
        stored.turns.last().map(|turn| turn.text.as_str()),
        Some(&*"x".repeat(EVENTS))
    );
}

#[tokio::test]
async fn an_oversized_reply_is_bounded_and_reported_as_truncated() {
    let oversized = "a".repeat(MAXIMUM_REPLY_BYTES + 128);
    let bounded = "a".repeat(MAXIMUM_REPLY_BYTES);
    let state = state_with_backend(ScriptedBackend::chunks([Ok(oversized)]));
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    wait_until_job_idle(&state, &token).await;

    let response = app(&state)
        .oneshot(observe_patch(&state, &token, &job, 0))
        .await
        .expect("observe");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let frames = stream_frames(&body);
    let joined = frames.join("");
    assert!(joined.contains(&bounded));
    assert!(!joined.contains(&format!("{bounded}a")));
    assert!(joined.contains("Power Plant truncated the model reply because it was too long."));
    assert!(frames.last().unwrap().contains("status=\"422\""));
    assert!(
        frames
            .iter()
            .all(|frame| frame.len() <= hypergraft::MAX_RESPONSE_BYTES)
    );

    let stored = session_snapshot(&state, &token);
    assert_eq!(
        stored.turns.last().map(|turn| turn.text.as_str()),
        Some(bounded.as_str())
    );
    assert_eq!(
        stored.job.as_ref().map(|job| job.status),
        Some(JobStatus::Failed)
    );
}

#[tokio::test]
async fn a_provider_failure_keeps_a_bounded_partial_reply() {
    let state = state_with_backend(ScriptedBackend::chunks([
        Ok("partial-reply".to_owned()),
        Err(ProviderError::Unreachable),
    ]));
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    wait_until_job_idle(&state, &token).await;

    let response = app(&state)
        .oneshot(observe_patch(&state, &token, &job, 0))
        .await
        .expect("observe");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let frames = stream_frames(&body);
    let final_frame = frames.last().expect("final frame");
    assert!(final_frame.contains("phase=\"final\""));
    assert!(final_frame.contains("status=\"422\""));
    assert!(frames.iter().any(|frame| frame.contains("partial-reply")));
    assert!(final_frame.contains("The provider could not be reached"));

    let stored = session_snapshot(&state, &token);
    assert_eq!(
        stored.turns.last().map(|turn| turn.text.as_str()),
        Some("partial-reply")
    );
}

#[tokio::test]
async fn a_malformed_cursor_is_rejected() {
    let state = test_state();
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("{}?job={job}&cursor=nope", chat_path(&state)))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("observe");

    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(response.headers().get(hypergraft::GRAFT_TRANSFER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("That cursor is not valid."));
    assert!(!text.contains("phase=\""));
}

#[tokio::test]
async fn an_excessive_cursor_is_rejected() {
    let state = test_state();
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("{}?job={job}&cursor=1000001", chat_path(&state)))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("observe");

    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("That cursor is not valid."));
}

#[tokio::test]
async fn cancel_stops_a_running_job() {
    let state = state_with_backend(ScriptedBackend::hang());
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);

    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("{}/jobs/{job}/cancel", chat_path(&state)))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("cancel");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(response.headers().get(hypergraft::GRAFT_TRANSFER).is_none());

    wait_until_job_idle(&state, &token).await;
    let stored = session_snapshot(&state, &token);
    assert_eq!(
        stored.job.as_ref().map(|job| job.status),
        Some(JobStatus::Cancelled)
    );
}

#[tokio::test]
async fn parallel_tabs_cannot_overwrite_a_completed_turn() {
    let state = state_with_backend(ScriptedBackend::hang());
    let token = connected(&state).await;
    let first = app(&state)
        .oneshot(patch_send_message(&state, &token, "First"))
        .await
        .expect("first send");
    assert_eq!(first.status(), axum::http::StatusCode::OK);

    let second = app(&state)
        .oneshot(patch_send_message(&state, &token, "Second"))
        .await
        .expect("second send");
    assert_eq!(second.status(), axum::http::StatusCode::CONFLICT);
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    let second_text = String::from_utf8(second_body.to_vec()).unwrap();
    assert!(second_text.contains("First"));
    assert!(second_text.contains("target=\"transcript\""));
    assert!(second_text.contains("Wait until this reply finishes."));
    assert!(!second_text.contains("Second"));

    let stored = session_snapshot(&state, &token);
    assert_eq!(
        stored
            .turns
            .iter()
            .map(|turn| turn.text.as_str())
            .collect::<Vec<_>>(),
        ["First"]
    );
    let job_id = stored.job.expect("job").id;
    assert!(
        state
            .sessions
            .finish_turn(&stored.id, &agent_id(&state), &job_id, "Done".to_owned())
    );
    if let Some(job) = state.sessions.job(&stored.id, &agent_id(&state), &job_id) {
        job.finish(JobStatus::Completed, None);
    }

    let stored = session_snapshot(&state, &token);
    assert_eq!(
        stored
            .turns
            .iter()
            .map(|turn| (turn.role, turn.text.as_str()))
            .collect::<Vec<_>>(),
        [(Role::User, "First"), (Role::Assistant, "Done")]
    );
}

#[tokio::test]
async fn an_oversized_navigation_falls_back_to_a_document() {
    let state = test_state();
    let token = connected(&state).await;
    let id = session_snapshot(&state, &token).id;
    let begun = state
        .sessions
        .begin_turn(&id, agent_id(&state), "Hello".to_owned())
        .expect("begin");
    assert!(state.sessions.finish_turn(
        &id,
        &agent_id(&state),
        &begun.job.id(),
        "a".repeat(1_200_000),
    ));
    let response = app(&state)
        .oneshot(navigation_show(&state, &token))
        .await
        .expect("chat navigation");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(response.headers().get(hypergraft::GRAFT_TRANSFER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert_eq!(text.matches("id=\"chat-main\"").count(), 1);
}

#[tokio::test]
async fn a_document_show_includes_the_desk_model_controls() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(document_show(&state, &token))
        .await
        .expect("chat document");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("id=\"desk-settings\""));
    assert!(text.contains("id=\"desk-model\""));
    assert!(text.contains("href=\"/connect\""));
    assert!(!text.contains("/disconnect"));
    assert!(!text.contains("test-key"));
}

#[tokio::test]
async fn an_oversized_model_name_is_rejected() {
    let state = test_state();
    let token = connected(&state).await;
    let long_model = "a".repeat(crate::providers::MAXIMUM_MODEL_BYTES + 1);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/model")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!("provider=xai&model={long_model}")))
                .unwrap(),
        )
        .await
        .expect("model");

    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("That model name is too long."));
    assert!(text.contains("target=\"desk-settings\""));
    assert!(!text.contains(&long_model));
    assert_eq!(
        state.vault.selected_connection().map(|item| item.model),
        Some("grok-4.6".to_owned())
    );
}

#[tokio::test]
async fn a_pending_catalogue_refresh_updates_the_rendered_desk() {
    let state = test_state();
    let token = connected(&state).await;
    state
        .vault
        .put(ProviderConnection::with_key(
            ProviderKind::Synthetic,
            "test-synthetic-key",
            "hf:moonshotai/Kimi-K3",
        ))
        .unwrap();
    state
        .models
        .set_for_test(ProviderKind::Synthetic, Vec::new(), true);

    let pending = app(&state)
        .oneshot(model_refresh_patch(&token))
        .await
        .expect("pending model refresh");
    let pending_body = to_bytes(pending.into_body(), usize::MAX).await.unwrap();
    let pending_text = String::from_utf8(pending_body.to_vec()).unwrap();
    assert!(pending_text.contains("target=\"desk-model-catalogue\""));
    assert!(!pending_text.contains("id=\"desk-provider\""));
    assert!(pending_text.contains("data-catalogue-pending=\"true\""));
    assert!(!pending_text.contains("data-model-value=\"syn:large:text\""));

    state.models.set_for_test(
        ProviderKind::Synthetic,
        vec![
            "hf:moonshotai/Kimi-K3".to_owned(),
            "syn:large:text".to_owned(),
            "syn:small:text".to_owned(),
        ],
        false,
    );
    let refreshed = app(&state)
        .oneshot(model_refresh_patch(&token))
        .await
        .expect("completed model refresh");
    let refreshed_body = to_bytes(refreshed.into_body(), usize::MAX).await.unwrap();
    let refreshed_text = String::from_utf8(refreshed_body.to_vec()).unwrap();

    assert!(refreshed_text.contains("data-catalogue-pending=\"false\""));
    assert!(refreshed_text.contains("data-model-value=\"syn:large:text\""));
    assert!(refreshed_text.contains("data-model-value=\"syn:small:text\""));
}

#[tokio::test]
async fn multiple_synthetic_models_are_rendered_and_selectable() {
    let state = test_state();
    let token = connected(&state).await;
    state
        .vault
        .put(ProviderConnection::with_key(
            ProviderKind::Synthetic,
            "test-synthetic-key",
            "hf:moonshotai/Kimi-K3",
        ))
        .unwrap();
    state.models.set_for_test(
        ProviderKind::Synthetic,
        vec![
            "hf:moonshotai/Kimi-K3".to_owned(),
            "syn:large:text".to_owned(),
            "syn:small:text".to_owned(),
        ],
        false,
    );

    let document = app(&state)
        .oneshot(document_show(&state, &token))
        .await
        .expect("chat document");
    let document_body = to_bytes(document.into_body(), usize::MAX).await.unwrap();
    let document_text = String::from_utf8(document_body.to_vec()).unwrap();
    assert!(document_text.contains("data-model-value=\"hf:moonshotai/Kimi-K3\""));
    assert!(document_text.contains("data-model-value=\"syn:large:text\""));
    assert!(document_text.contains("data-model-value=\"syn:small:text\""));

    let response = app(&state)
        .oneshot(model_update_patch(
            &token,
            "provider=synthetic&model=syn%3Alarge%3Atext",
        ))
        .await
        .expect("model update");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let response_body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let response_text = String::from_utf8(response_body.to_vec()).unwrap();
    assert!(response_text.contains("value=\"syn:large:text\""));
    assert_eq!(
        state.vault.selected_connection().map(|item| item.model),
        Some("syn:large:text".to_owned())
    );
}

#[tokio::test]
async fn a_native_provider_change_keeps_that_providers_saved_model() {
    let state = test_state();
    let token = connected(&state).await;
    state
        .vault
        .put(ProviderConnection::with_key(
            ProviderKind::OpenaiCodex,
            "test-openai-key",
            "gpt-5.1-codex",
        ))
        .unwrap();
    state
        .vault
        .select(ProviderKind::Xai, "grok-4.6".to_owned())
        .unwrap();

    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/model")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("provider=openai-codex&model=grok-4.6"))
                .unwrap(),
        )
        .await
        .expect("provider change");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let selected = state.vault.selected_connection().unwrap();
    assert_eq!(selected.kind, ProviderKind::OpenaiCodex);
    assert_eq!(selected.model, "gpt-5.1-codex");
}

#[tokio::test]
async fn the_desk_can_toggle_a_model_favourite() {
    let state = test_state();
    let token = connected(&state).await;
    for expected in [true, false] {
        let response = app(&state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/model")
                    .header(header::COOKIE, cookie(&token))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header(hypergraft::GRAFT_REQUEST, "patch")
                    .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                    .body(Body::from(
                        "provider=xai&model=grok-4.6&favourite=grok-4-mini",
                    ))
                    .unwrap(),
            )
            .await
            .expect("favourite toggle");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("target=\"desk-model-catalogue\""));
        assert!(!text.contains("id=\"desk-provider\""));
        assert!(!text.contains("id=\"desk-model-search\""));
        assert!(text.contains(if expected {
            "aria-pressed=\"true\""
        } else {
            "aria-pressed=\"false\""
        }));

        let desk = state.vault.desk_providers();
        let favourites = &desk
            .iter()
            .find(|provider| provider.kind == ProviderKind::Xai)
            .expect("xai stored")
            .favourites;
        assert_eq!(favourites.contains(&"grok-4-mini".to_owned()), expected);
        assert_eq!(
            state.vault.selected_connection().map(|item| item.model),
            Some("grok-4.6".to_owned())
        );
    }
}

#[tokio::test]
async fn a_favourite_toggle_without_a_model_is_rejected() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/model")
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("provider=xai&model=grok-4.6&favourite="))
                .unwrap(),
        )
        .await
        .expect("favourite toggle");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Choose a model."));
    assert_eq!(
        state.vault.selected_connection().map(|item| item.model),
        Some("grok-4.6".to_owned())
    );
}

fn sandbox_handle(state: &AppState) -> std::sync::Arc<crate::sandbox::GuestSandbox> {
    state.sandboxes.handle(agent_id(state))
}

fn sandbox_patch(state: &AppState, token: &str, action: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{}/sandbox", chat_path(state)))
        .header(header::COOKIE, cookie(token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from(format!("command={action}")))
        .unwrap()
}

#[tokio::test]
async fn a_patch_sandbox_start_returns_starting_status() {
    let state = test_state();
    let token = connected_idle(&state).await;
    let response = app(&state)
        .oneshot(sandbox_patch(&state, &token, "start"))
        .await
        .expect("sandbox start");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(response.headers().get(hypergraft::GRAFT_TRANSFER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"sandbox-status\""));
    assert!(text.contains("data-sandbox-status=\"starting\""));
    assert!(text.contains("data-sandbox-active=\"true\""));
    assert!(text.contains("Starting the virtual machine"));
    assert!(!text.contains("phase=\""));
    assert_eq!(
        sandbox_handle(&state).view().await.status.as_str(),
        "starting"
    );
}

#[tokio::test]
async fn a_sandbox_observe_patch_settles_when_start_finishes() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("{}/sandbox", chat_path(&state)))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("sandbox observe");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"sandbox-status\""));
    assert!(text.contains("data-sandbox-status=\"running\""));
    assert!(!text.contains("data-sandbox-active=\"true\""));
}

#[tokio::test]
async fn a_sandbox_document_get_redirects_to_chat() {
    let state = test_state();
    let token = connected(&state).await;
    let expected = chat_path(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("{}/sandbox", chat_path(&state)))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("sandbox document");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        expected.as_str()
    );
}

#[tokio::test]
async fn a_patch_sandbox_stop_updates_sandbox_status() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(sandbox_patch(&state, &token, "stop"))
        .await
        .expect("sandbox stop");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"sandbox-status\""));
    assert!(text.contains("data-sandbox-status=\"stopped\""));
}

#[tokio::test]
async fn an_unknown_sandbox_action_is_rejected() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(sandbox_patch(&state, &token, "remove"))
        .await
        .expect("sandbox action");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Choose start or stop."));
    assert!(text.contains("target=\"sandbox-status\""));
}

#[tokio::test]
async fn a_project_without_a_running_sandbox_rejects_an_agent_turn() {
    let state = test_state();
    let token = connected_idle(&state).await;
    let response = app(&state)
        .oneshot(patch_send_message(&state, &token, "ls"))
        .await
        .expect("chat send");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Start the sandbox."));
    assert!(text.contains("target=\"composer\""));
    assert!(!text.contains("target=\"transcript\""));
}

#[tokio::test]
async fn an_agent_turn_streams_a_tool_trace() {
    let state = state_with_backend(ScriptedBackend::tool_then(
        "write",
        serde_json::json!({"path": "note.txt", "contents": "hello"}),
        "Wrote the note.",
    ));
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send_message(&state, &token, "Add a note"))
        .await
        .expect("chat send");
    assert_eq!(started.status(), axum::http::StatusCode::OK);
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    wait_until_job_idle(&state, &token).await;

    let response = app(&state)
        .oneshot(observe_patch(&state, &token, &job, 0))
        .await
        .expect("observe");
    assert_eq!(
        response.headers().get(hypergraft::GRAFT_TRANSFER).unwrap(),
        "stream"
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let frames = stream_frames(&body);
    assert!(frames.iter().any(|frame| frame.contains("write")));
    assert!(frames.iter().any(|frame| frame.contains("note.txt")));
    assert!(!job_active(frames.last().expect("final")));

    let stored = session_snapshot(&state, &token);
    let assistant = stored
        .turns
        .iter()
        .find(|turn| turn.role == Role::Assistant)
        .expect("assistant");
    assert!(assistant.text.contains("write"));
    assert!(assistant.text.contains("/project/note.txt"));
    assert!(assistant.text.contains("Wrote the note."));
}

#[tokio::test]
async fn cancel_stops_a_running_command() {
    let state = state_with_backend(ScriptedBackend::tool_then(
        "run",
        serde_json::json!({"command": "sleep 30"}),
        "done",
    ));
    let token = connected(&state).await;
    sandbox_handle(&state).hang_next_command();
    let started = app(&state)
        .oneshot(patch_send_message(&state, &token, "sleep 30"))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("{}/jobs/{job}/cancel", chat_path(&state)))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("cancel");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    wait_until_job_idle(&state, &token).await;
    let stored = session_snapshot(&state, &token);
    assert_eq!(
        stored.job.as_ref().map(|job| job.status),
        Some(JobStatus::Cancelled)
    );
}

#[tokio::test]
async fn a_later_document_show_renders_a_tool_trace() {
    let state = state_with_backend(ScriptedBackend::tool_then(
        "write",
        serde_json::json!({"path": "note.txt", "contents": "hello"}),
        "Wrote the note.",
    ));
    let token = connected(&state).await;
    let _ = app(&state)
        .oneshot(patch_send_message(&state, &token, "Add a note"))
        .await
        .expect("chat send");
    wait_until_job_idle(&state, &token).await;

    let response = app(&state)
        .oneshot(document_show(&state, &token))
        .await
        .expect("chat document");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("note.txt"));
    assert!(text.contains("Wrote the note."));
    assert_eq!(
        session_snapshot(&state, &token)
            .turns
            .last()
            .map(|turn| turn.role),
        Some(Role::Assistant)
    );
}

#[tokio::test]
async fn a_sandbox_stop_during_a_job_is_rejected() {
    let state = state_with_backend(ScriptedBackend::hang());
    let token = connected(&state).await;
    let _ = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    let response = app(&state)
        .oneshot(sandbox_patch(&state, &token, "stop"))
        .await
        .expect("sandbox stop");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Wait until this reply finishes."));
    assert!(text.contains("target=\"sandbox-status\""));
}

#[tokio::test]
async fn a_document_show_disables_sandbox_controls_during_a_job() {
    let state = state_with_backend(ScriptedBackend::hang());
    let token = connected(&state).await;
    let _ = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("chat send");
    let response = app(&state)
        .oneshot(document_show(&state, &token))
        .await
        .expect("chat document");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    let start_at = text.find("Stop sandbox").expect("stop control");
    assert!(text[start_at.saturating_sub(160)..start_at].contains("disabled"));
}

#[tokio::test]
async fn two_agents_advertise_distinct_prompts_and_tools() {
    let backend = ScriptedBackend::accept();
    let state = state_with_backend(backend.clone());
    let token = connected(&state).await;
    let _ = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("first send");
    wait_until_job_idle(&state, &token).await;
    let first_preamble = backend.last_preamble().expect("first preamble");
    let first_tools = backend.last_tools();
    assert!(first_preamble.contains("# Power Plant contract"));
    assert!(first_preamble.contains("/project"));
    assert!(
        !first_preamble.contains(
            state.agents.list()[0].directories[0]
                .host_path
                .to_string_lossy()
                .as_ref()
        )
    );
    assert!(first_tools.iter().any(|name| name == "write"));

    let dir = tempfile::tempdir().expect("second");
    let second = state
        .agents
        .create(AgentDraft {
            name: "Reader".to_owned(),
            instructions: "Only read files.".to_owned(),
            tools: vec![ToolId::List, ToolId::Read],
            directories: vec![DirectoryGrant {
                alias: "project".to_owned(),
                host_path: dir.path().to_path_buf(),
                access: AccessMode::ReadWrite,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("second agent");
    let sandbox = state.sandboxes.handle(second.id);
    let policy = DirectoryPolicy::from_record(&second);
    let access =
        GuestAccess::from_connection(&state.vault.selected_connection().expect("connection"));
    sandbox
        .start_with(SandboxSpec::from_policy(&policy, access))
        .await
        .expect("start");
    sandbox.complete_start();
    state
        .scratch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(dir);

    let send = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/agents/{}", second.id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("message=Hello"))
                .unwrap(),
        )
        .await
        .expect("second send");
    assert_eq!(send.status(), axum::http::StatusCode::OK);
    for _ in 0..2_000 {
        if state
            .sessions
            .snapshot(&session_snapshot(&state, &token).id, &second.id)
            .expect("second session")
            .job
            .is_some_and(|job| job.status != JobStatus::Running)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    let second_preamble = backend.last_preamble().expect("second preamble");
    let second_tools = backend.last_tools();
    assert!(second_preamble.contains("Only read files."));
    assert_ne!(first_preamble, second_preamble);
    assert_eq!(second_tools, ["list".to_owned(), "read".to_owned()]);
    assert_ne!(first_tools, second_tools);
}
