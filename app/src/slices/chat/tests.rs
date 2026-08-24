use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    config::RuntimeConfig,
    providers::{
        ChatBackend, ProviderConnection, ProviderError, ProviderKind, Role, SecretString,
        scripted::ScriptedBackend,
    },
    sessions::{self, JobStatus},
    state::AppState,
};

use super::job::MAXIMUM_REPLY_BYTES;

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

fn state_with_backend(backend: ScriptedBackend) -> AppState {
    let mut state = test_state();
    state.chat = Arc::new(ChatBackend::Scripted(backend));
    state
}

fn connected(state: &AppState) -> String {
    let token = sessions::generate_session_token().expect("session token");
    state
        .vault
        .put(ProviderConnection {
            kind: ProviderKind::Xai,
            api_key: SecretString::new("test-key".to_owned()),
            model: "grok-4.6".to_owned(),
        })
        .expect("vault");
    state.sessions.insert(token.id());
    token.raw().as_str().to_owned()
}

fn patch_send_message(token: &str, message: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/")
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
        .snapshot(&sessions::SessionId::from_validated(&validated))
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
    format!("circus_session={token}")
}

fn patch_send(token: &str) -> Request<Body> {
    patch_send_message(token, "Hello")
}

fn document_show(token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/")
        .header(header::COOKIE, cookie(token))
        .body(Body::empty())
        .unwrap()
}

fn navigation_show(token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/")
        .header(header::COOKIE, cookie(token))
        .header(hypergraft::GRAFT_REQUEST, "navigation")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .unwrap()
}

fn observe_patch(token: &str, job: &str, cursor: u64) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("/?job={job}&cursor={cursor}"))
        .header(header::COOKIE, cookie(token))
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::empty())
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
    let token = connected(&state);
    let response = app(state)
        .oneshot(document_show(&token))
        .await
        .expect("chat document");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(response.headers().get(hypergraft::GRAFT_TRANSFER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert_eq!(text.matches("id=\"chat-main\"").count(), 1);
    assert!(text.contains("id=\"chat-main\" tabindex=\"-1\""));
    assert!(text.contains("href=\"/\""));
    assert!(text.contains("data-graft"));
}

#[tokio::test]
async fn a_navigation_show_patches_chat_main_children() {
    let state = test_state();
    let token = connected(&state);
    let response = app(state)
        .oneshot(navigation_show(&token))
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
    let token = connected(&state);
    let response = app(state.clone())
        .oneshot(patch_send(&token))
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
    let token = connected(&state);
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
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
    let token = connected(&state);
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
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
    assert!(!text.contains("Hello from Circus."));
}

#[tokio::test]
async fn a_later_document_show_renders_the_finished_job() {
    let state = test_state();
    let token = connected(&state);
    let _ = app(state.clone())
        .oneshot(patch_send(&token))
        .await
        .expect("chat send");
    wait_until_job_idle(&state, &token).await;

    let response = app(state.clone())
        .oneshot(document_show(&token))
        .await
        .expect("chat document");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert!(text.contains("Hello from Circus."));
    assert!(!text.contains("data-job-active=\"true\""));

    let stored = session_snapshot(&state, &token);
    assert_eq!(
        stored.turns.last().map(|turn| turn.text.as_str()),
        Some("Hello from Circus.")
    );
}

#[tokio::test]
async fn observation_streams_one_bounded_segment_with_one_final_frame() {
    let state = test_state();
    let token = connected(&state);
    let started = app(state.clone())
        .oneshot(patch_send(&token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    wait_until_job_idle(&state, &token).await;

    let response = app(state)
        .oneshot(observe_patch(&token, &job, 0))
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
            .any(|frame| frame.contains("Hello from Circus."))
    );
    assert!(!job_active(final_frame));
}

#[tokio::test]
async fn a_long_job_uses_repeated_observation_segments() {
    const EVENTS: usize = 300;
    let state = state_with_backend(ScriptedBackend::chunks(
        (0..EVENTS).map(|_| Ok("x".to_owned())),
    ));
    let token = connected(&state);
    let started = app(state.clone())
        .oneshot(patch_send(&token))
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
        let response = app(state.clone())
            .oneshot(observe_patch(&token, &job, cursor))
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

    let retry = app(state.clone())
        .oneshot(observe_patch(&token, &job, 1))
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
    let token = connected(&state);
    let started = app(state.clone())
        .oneshot(patch_send(&token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    wait_until_job_idle(&state, &token).await;

    let response = app(state.clone())
        .oneshot(observe_patch(&token, &job, 0))
        .await
        .expect("observe");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let frames = stream_frames(&body);
    let joined = frames.join("");
    assert!(joined.contains(&bounded));
    assert!(!joined.contains(&format!("{bounded}a")));
    assert!(joined.contains("Circus truncated the model reply because it was too long."));
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
    let token = connected(&state);
    let started = app(state.clone())
        .oneshot(patch_send(&token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    wait_until_job_idle(&state, &token).await;

    let response = app(state.clone())
        .oneshot(observe_patch(&token, &job, 0))
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
    let token = connected(&state);
    let started = app(state.clone())
        .oneshot(patch_send(&token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/?job={job}&cursor=nope"))
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
    let token = connected(&state);
    let started = app(state.clone())
        .oneshot(patch_send(&token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/?job={job}&cursor=1000001"))
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
    let token = connected(&state);
    let started = app(state.clone())
        .oneshot(patch_send(&token))
        .await
        .expect("chat send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);

    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/jobs/{job}/cancel"))
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
    let token = connected(&state);
    let first = app(state.clone())
        .oneshot(patch_send_message(&token, "First"))
        .await
        .expect("first send");
    assert_eq!(first.status(), axum::http::StatusCode::OK);

    let second = app(state.clone())
        .oneshot(patch_send_message(&token, "Second"))
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
            .finish_turn(&stored.id, &job_id, "Done".to_owned())
    );
    if let Some(job) = state.sessions.job(&stored.id, &job_id) {
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
    let token = connected(&state);
    let id = session_snapshot(&state, &token).id;
    let begun = state
        .sessions
        .begin_turn(&id, "Hello".to_owned())
        .expect("begin");
    assert!(
        state
            .sessions
            .finish_turn(&id, &begun.job.id(), "a".repeat(1_200_000),)
    );
    let response = app(state)
        .oneshot(navigation_show(&token))
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
    let token = connected(&state);
    let response = app(state)
        .oneshot(document_show(&token))
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
    let token = connected(&state);
    let long_model = "a".repeat(crate::providers::MAXIMUM_MODEL_BYTES + 1);
    let response = app(state.clone())
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
