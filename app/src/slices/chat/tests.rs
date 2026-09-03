use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    agents::{AccessMode, AgentDraft, DirectoryGrant, ToolId},
    config::RuntimeConfig,
    providers::{
        ChatBackend, ProviderConnection, ProviderError, ProviderKind, Role, tests::ScriptedBackend,
    },
    sessions::{self, JobStatus},
    state::AppState,
};

use super::job::MAXIMUM_REPLY_BYTES;

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

fn state_with_backend(backend: ScriptedBackend) -> AppState {
    let mut state = test_state();
    state.chat = Arc::new(ChatBackend::Scripted(backend));
    state
}

#[derive(Clone, Copy)]
struct TestLiveGuard(sessions::SessionId);

impl hypergraft::live::LiveGuard for TestLiveGuard {
    type Connection = ();
    type Context = sessions::SessionId;

    async fn bind(
        &self,
        _extensions: &axum::http::Extensions,
    ) -> Result<Self::Connection, hypergraft::live::GuardFailure> {
        Ok(())
    }

    async fn revalidate(
        &self,
        _connection: &Self::Connection,
    ) -> Result<Self::Context, hypergraft::live::GuardFailure> {
        Ok(self.0)
    }
}

fn agent_hex(state: &AppState) -> String {
    state.agents.list()[0].id.as_hex()
}

fn agent_id(state: &AppState) -> crate::agents::AgentId {
    state.agents.list()[0].id
}

fn project_hex(state: &AppState) -> String {
    state.projects.list()[0].id.as_hex()
}

fn conversation_key(state: &AppState) -> crate::sessions::ConversationKey {
    crate::sessions::ConversationKey {
        project_id: state.projects.list()[0].id,
        agent_id: agent_id(state),
    }
}

fn chat_path(state: &AppState) -> String {
    format!(
        "/projects/{}/agents/{}",
        project_hex(state),
        agent_hex(state)
    )
}

fn git_init(path: &std::path::Path) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .expect("git")
            .success()
    );
}

async fn connected(state: &AppState) -> String {
    let token = connect_session(state).await;
    if state.workflows.list().is_empty() {
        seed_ready_workflow(state).await;
    }
    token
}

async fn connect_session(state: &AppState) -> String {
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
    git_init(dir.path());
    state
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
    let host = state.agents.list()[0].directories[0].host_path.clone();
    state
        .projects
        .create("Test project".to_owned(), host)
        .expect("project");
    state.keep_temp_dir(dir);
    token.raw().as_str().to_owned()
}

async fn seed_ready_workflow(state: &AppState) {
    state.environments.apply_production_seeds();
    let environment_id = crate::workflows::alpine_git_id(&state.environments).expect("alpine-git");
    let environment = state
        .environments
        .get(&environment_id)
        .expect("environment");
    if environment.ready_preparation.is_none() {
        let preparation = state
            .environments
            .claim_oldest_queued()
            .expect("claim")
            .expect("queued");
        let snapshot = crate::tests::sample_snapshot(preparation.id);
        state.environment_snapshots.mark(
            snapshot.artifact_key.clone(),
            crate::environments::SnapshotAvailability::Available,
        );
        state
            .environments
            .finish_ready(&preparation.id, snapshot, preparation.log)
            .expect("ready");
    }
    if state.workflows.list().is_empty() {
        state
            .workflows
            .create(crate::workflows::seeds::one_agent_definition(
                environment.id,
            ))
            .expect("workflow");
    }
}

fn workflow_token(state: &AppState) -> String {
    let record = &state.workflows.list()[0];
    crate::workflows::WorkflowSelection {
        workflow_id: record.id,
        definition_version: record.definition_version,
    }
    .as_token()
}

fn patch_send_message(state: &AppState, token: &str, message: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(chat_path(state))
        .header(header::COOKIE, cookie(token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from(format!(
            "message={message}&mode=configured&workflow={}",
            workflow_token(state)
        )))
        .unwrap()
}

fn session_id(token: &str) -> sessions::SessionId {
    sessions::SessionId::from_validated(&sessions::ValidatedToken::parse(token).expect("token"))
}

fn session_snapshot(state: &AppState, token: &str) -> sessions::SessionSnapshot {
    state
        .sessions
        .snapshot(&session_id(token), &conversation_key(state))
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

fn patch_send_quick(state: &AppState, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(chat_path(state))
        .header(header::COOKIE, cookie(token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from("message=Hello&mode=quick"))
        .unwrap()
}

fn patch_send_configured(state: &AppState, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!(
            "{}?workflow={}",
            chat_path(state),
            workflow_token(state)
        ))
        .header(header::COOKIE, cookie(token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from("message=Hello&mode=configured"))
        .unwrap()
}

fn document_show(state: &AppState, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(chat_path(state))
        .header(header::COOKIE, cookie(token))
        .body(Body::empty())
        .unwrap()
}

async fn desk_html(state: &AppState, token: &str) -> String {
    let response = app(state)
        .oneshot(document_show(state, token))
        .await
        .expect("document");
    String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

fn alpine_environment_id(state: &AppState) -> crate::environments::EnvironmentId {
    crate::workflows::alpine_git_id(&state.environments).expect("alpine-git")
}

fn configuration_href(id: &crate::environments::EnvironmentId) -> String {
    format!("/environments/{}/configuration", id.as_hex())
}

fn opening_tag_for<'a>(html: &'a str, marker: &str) -> &'a str {
    let marker_start = html.find(marker).expect("element marker");
    let tag_start = html[..marker_start].rfind('<').expect("opening tag");
    let tag_end = html[marker_start..].find('>').expect("opening tag end") + marker_start;
    &html[tag_start..=tag_end]
}

fn quick_send_disabled(html: &str) -> bool {
    opening_tag_for(html, "value=\"quick\"").contains(" disabled")
}

fn composer_message_disabled(html: &str) -> bool {
    opening_tag_for(html, "id=\"composer-message\"").contains(" disabled")
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

fn sandbox_observe_patch(
    state: &AppState,
    token: &str,
    sandbox: &str,
    workflow: &str,
) -> Request<Body> {
    let mut uri = format!("{}?sandbox={sandbox}", chat_path(state));
    if !workflow.is_empty() {
        uri.push_str("&workflow=");
        uri.push_str(workflow);
    }
    Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::COOKIE, cookie(token))
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .unwrap()
}

fn sandbox_cursor_token(state: &AppState) -> String {
    crate::environments::EnvironmentCatalogue::cursor_token(state.environments.refresh_cursor())
}

fn sandbox_cursor_from(html: &str) -> String {
    let name = html.find("name=\"sandbox\"").expect("sandbox field");
    let marker = "value=\"";
    let start = html[name..].find(marker).expect("sandbox value") + name + marker.len();
    let end = html[start..].find('"').expect("sandbox value end") + start;
    html[start..end].to_owned()
}

async fn sandbox_observe_text(
    state: &AppState,
    token: &str,
    sandbox: &str,
    workflow: &str,
) -> (axum::http::StatusCode, String) {
    let response = app(state)
        .oneshot(sandbox_observe_patch(state, token, sandbox, workflow))
        .await
        .expect("sandbox observe");
    let status = response.status();
    let text = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    (status, text)
}

fn model_refresh_patch(state: &AppState, token: &str) -> Request<Body> {
    Request::builder()
        .uri(format!(
            "/model?project={}&agent={}",
            project_hex(state),
            agent_hex(state)
        ))
        .header(header::COOKIE, cookie(token))
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .unwrap()
}

fn model_update_patch(state: &AppState, token: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/model")
        .header(header::COOKIE, cookie(token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from(format!(
            "{body}&project={}&agent={}",
            project_hex(state),
            agent_hex(state)
        )))
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
    html.contains("data-observe-active=\"true\"")
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

#[test]
fn workflow_policy_follows_the_commit_edges() {
    use crate::workflows::definition::{ArtefactKind, ArtefactSource, WorkflowDefinition};

    let environment = crate::tests::test_environment_id();
    let read_only = crate::workflows::seeds::read_only_review_definition(environment);
    assert_eq!(
        super::workflow_policy(&read_only),
        "Read-only review before commit"
    );
    let independent = crate::workflows::seeds::review_with_fixes_definition(environment);
    assert_eq!(
        super::workflow_policy(&independent),
        "Fixing review with independent read-only review"
    );

    let mut steps = independent.steps().to_vec();
    steps.retain(|step| step.key.as_str() != "independent-reviewer");
    let commit = steps
        .iter_mut()
        .find(|step| step.key.as_str() == "commit")
        .expect("commit");
    let report = commit
        .inputs
        .iter_mut()
        .find(|input| input.kind == ArtefactKind::ReviewReport)
        .expect("report");
    report.source = ArtefactSource::StepOutput {
        step: crate::workflows::definition::StepKey::parse("fixing-reviewer").expect("step"),
        output: crate::workflows::definition::OutputKey::parse("review").expect("output"),
    };
    let roles = independent
        .roles()
        .iter()
        .filter(|role| role.key.as_str() != "independent-reviewer")
        .cloned()
        .collect();
    let direct = WorkflowDefinition::from_parts("Direct".to_owned(), environment, roles, steps)
        .expect("direct workflow");
    assert_eq!(
        super::workflow_policy(&direct),
        "Fixing review with direct commit policy"
    );
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
    assert!(text.contains("data-observe-active=\"true\""));
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
async fn a_native_send_is_rejected_before_the_job_starts() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(chat_path(&state))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("message=Hello&mode=quick".to_owned()))
                .unwrap(),
        )
        .await
        .expect("chat send");

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let stored = session_snapshot(&state, &token);
    assert!(stored.turns.is_empty());
    assert!(stored.job.is_none());
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
    assert!(!text.contains("data-observe-active=\"true\""));

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
    assert!(!text.contains("data-observe-active=\"true\""));
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
    assert!(state.sessions.finish_turn(
        &session_id(&token),
        &conversation_key(&state),
        &job_id,
        "Done".to_owned()
    ));
    if let Some(job) = state
        .sessions
        .job(&session_id(&token), &conversation_key(&state), &job_id)
    {
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
    let id = session_id(&token);
    let begun = state
        .sessions
        .begin_turn(
            &id,
            conversation_key(&state),
            crate::workflows::RunId::generate().expect("run"),
            "Hello".to_owned(),
        )
        .expect("begin");
    assert!(state.sessions.finish_turn(
        &id,
        &conversation_key(&state),
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
        .oneshot(model_update_patch(
            &state,
            &token,
            &format!("provider=xai&model={long_model}"),
        ))
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
async fn model_updates_require_an_eligible_project_and_agent_pair() {
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
                .body(Body::from(format!(
                    "provider=xai&model=other&project={}&agent={}",
                    "0".repeat(32),
                    agent_hex(&state)
                )))
                .unwrap(),
        )
        .await
        .expect("model update");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("navigate=\"/projects\""));
    assert_eq!(
        state.vault.selected_connection().map(|item| item.model),
        Some("grok-4.6".to_owned())
    );
}

#[tokio::test]
async fn the_model_live_projection_sends_current_truth_after_an_invalidation() {
    let state = test_state();
    let token = connected(&state).await;
    let url = format!(
        "/model?project={}&agent={}",
        project_hex(&state),
        agent_hex(&state)
    );
    let harness = hypergraft::live::LiveHarness::new(super::live_router(), state.clone());
    let mut projection = harness
        .subscribe(&url, TestLiveGuard(session_id(&token)))
        .await
        .expect("live model projection");

    assert_eq!(projection.first_patch().targets.len(), 1);
    assert_eq!(
        projection.first_patch().targets[0].target,
        "desk-model-catalogue"
    );

    state
        .models
        .set_catalogue(ProviderKind::Xai, vec!["grok-live".to_owned()], false);
    let patch = tokio::time::timeout(std::time::Duration::from_secs(1), projection.next_patch())
        .await
        .expect("live patch timeout")
        .expect("live patch");
    assert_eq!(patch.targets[0].target, "desk-model-catalogue");
    assert!(patch.targets[0].html.contains("grok-live"));
}

#[tokio::test]
async fn the_model_live_projection_rejects_an_invalid_query() {
    let state = test_state();
    let token = connected(&state).await;
    let harness = hypergraft::live::LiveHarness::new(super::live_router(), state);
    let result = harness
        .subscribe(
            "/model?project=invalid&agent=invalid",
            TestLiveGuard(session_id(&token)),
        )
        .await;

    assert!(matches!(
        result,
        Err(hypergraft::live::HarnessError::Invalid)
    ));
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
        .set_catalogue(ProviderKind::Synthetic, Vec::new(), true);

    let pending = app(&state)
        .oneshot(model_refresh_patch(&state, &token))
        .await
        .expect("pending model refresh");
    let pending_body = to_bytes(pending.into_body(), usize::MAX).await.unwrap();
    let pending_text = String::from_utf8(pending_body.to_vec()).unwrap();
    assert!(pending_text.contains("target=\"desk-model-catalogue\""));
    assert!(!pending_text.contains("id=\"desk-provider\""));
    assert!(pending_text.contains("data-catalogue-pending=\"true\""));
    assert!(!pending_text.contains("data-model-value=\"syn:large:text\""));

    state.models.set_catalogue(
        ProviderKind::Synthetic,
        vec![
            "hf:moonshotai/Kimi-K3".to_owned(),
            "syn:large:text".to_owned(),
            "syn:small:text".to_owned(),
        ],
        false,
    );
    let refreshed = app(&state)
        .oneshot(model_refresh_patch(&state, &token))
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
    state.models.set_catalogue(
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
            &state,
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
        .oneshot(model_update_patch(
            &state,
            &token,
            "provider=openai-codex&model=grok-4.6",
        ))
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
            .oneshot(model_update_patch(
                &state,
                &token,
                "provider=xai&model=grok-4.6&favourite=grok-4-mini",
            ))
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
        .oneshot(model_update_patch(
            &state,
            &token,
            "provider=xai&model=grok-4.6&favourite=",
        ))
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

#[tokio::test]
async fn a_missing_environment_rejects_a_task_before_a_run_starts() {
    let state = test_state();
    let token = connected(&state).await;
    let base = crate::workflows::seeds::one_agent_definition(crate::tests::test_environment_id());
    let workflow = state
        .workflows
        .create(
            crate::workflows::definition::WorkflowDefinition::from_parts(
                "Missing environment".to_owned(),
                crate::tests::test_environment_id(),
                base.roles().to_vec(),
                base.steps().to_vec(),
            )
            .expect("definition"),
        )
        .expect("unresolved");
    let selection = crate::workflows::WorkflowSelection {
        workflow_id: workflow.id,
        definition_version: workflow.definition_version,
    }
    .as_token();
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(chat_path(&state))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "message=ls&mode=configured&workflow={selection}"
                )))
                .unwrap(),
        )
        .await
        .expect("send");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("That environment is no longer in the catalogue."));
    assert!(session_snapshot(&state, &token).job.is_none());
}

#[tokio::test]
async fn a_workflow_preview_patch_updates_composer() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "{}?workflow={}",
                    chat_path(&state),
                    workflow_token(&state)
                ))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("preview");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"composer\""));
    assert!(!text.contains("target=\"readiness-route\""));
    assert!(!text.contains("target=\"sandbox-status\""));
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
    state.sandboxes.hang_next_command();
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
    git_init(dir.path());
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
    state
        .projects
        .create(
            "Reader project".to_owned(),
            second.directories[0].host_path.clone(),
        )
        .expect("second project");
    state.keep_temp_dir(dir);
    let environment_id = state.environments.list()[0].id;
    let reader = {
        let base = crate::workflows::seeds::one_agent_definition(environment_id);
        let mut steps = base.steps().to_vec();
        if let crate::workflows::definition::StepAction::Agent(action) = &mut steps[0].action {
            action.authority = crate::workflows::definition::AgentAuthority::new(
                vec![ToolId::List, ToolId::Read],
                action.authority.directories.clone(),
            )
            .expect("authority");
        }
        crate::workflows::definition::WorkflowDefinition::from_parts(
            "Reader".to_owned(),
            environment_id,
            base.roles().to_vec(),
            steps,
        )
        .expect("reader")
    };
    let reader = state.workflows.create(reader).expect("reader workflow");
    let reader_token = crate::workflows::WorkflowSelection {
        workflow_id: reader.id,
        definition_version: reader.definition_version,
    }
    .as_token();

    let reader_project = state
        .projects
        .list()
        .into_iter()
        .find(|project| project.host_path == second.directories[0].host_path)
        .expect("reader project");
    let send = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/projects/{}/agents/{}",
                    reader_project.id.as_hex(),
                    second.id.as_hex()
                ))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "message=Hello&mode=configured&workflow={reader_token}"
                )))
                .unwrap(),
        )
        .await
        .expect("second send");
    assert_eq!(send.status(), axum::http::StatusCode::OK);
    for _ in 0..2_000 {
        if state
            .sessions
            .snapshot(
                &session_id(&token),
                &crate::sessions::ConversationKey {
                    project_id: reader_project.id,
                    agent_id: second.id,
                },
            )
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

#[tokio::test]
async fn a_second_session_cannot_start_while_a_workflow_runs() {
    let state = state_with_backend(ScriptedBackend::hang());
    let first = connected(&state).await;
    let second_token = sessions::generate_session_token().expect("session token");
    state.sessions.insert(second_token.id());
    let second = second_token.raw().as_str().to_owned();
    let started = app(&state)
        .oneshot(patch_send(&state, &first))
        .await
        .expect("first");
    assert_eq!(started.status(), axum::http::StatusCode::OK);
    let response = app(&state)
        .oneshot(patch_send(&state, &second))
        .await
        .expect("second");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Wait until the current workflow finishes."));
}

#[tokio::test]
async fn a_missing_run_mode_is_rejected_before_a_job_starts() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(chat_path(&state))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("message=Hello"))
                .unwrap(),
        )
        .await
        .expect("send");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("Choose a run mode."));
    assert!(session_snapshot(&state, &token).job.is_none());
    assert!(state.workflow_runs.summaries().is_empty());
}

#[tokio::test]
async fn a_configured_send_without_a_workflow_is_rejected() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(chat_path(&state))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("message=Hello&mode=configured"))
                .unwrap(),
        )
        .await
        .expect("send");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("Choose a workflow."));
    assert!(session_snapshot(&state, &token).job.is_none());
    assert!(state.workflow_runs.summaries().is_empty());
}

#[tokio::test]
async fn a_stale_workflow_selection_is_a_conflict() {
    let state = test_state();
    let token = connected(&state).await;
    let stale = workflow_token(&state);
    let current = state.workflows.list().into_iter().next().expect("workflow");
    state
        .workflows
        .update(
            &current.id,
            current.revision,
            crate::workflows::definition::WorkflowDefinition::from_parts(
                "Edited agent".to_owned(),
                crate::tests::test_environment_id(),
                current.definition.roles().to_vec(),
                current.definition.steps().to_vec(),
            )
            .expect("edited"),
        )
        .expect("update");
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(chat_path(&state))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "message=Hello&mode=configured&workflow={stale}"
                )))
                .unwrap(),
        )
        .await
        .expect("send");
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    assert!(session_snapshot(&state, &token).job.is_none());
}

#[tokio::test]
async fn a_new_run_pins_the_selected_catalogue_identity() {
    let state = test_state();
    let token = connected(&state).await;
    let record = state.workflows.list().into_iter().next().expect("workflow");
    let _ = app(&state)
        .oneshot(patch_send_configured(&state, &token))
        .await
        .expect("send");
    wait_until_job_idle(&state, &token).await;
    let run = &state.workflow_runs.summaries()[0];
    assert_eq!(run.workflow_id, Some(record.id));
    assert_eq!(run.version, record.definition_version);
    assert_eq!(run.project_id, state.projects.list()[0].id);
    let stored = state.workflow_runs.get(&run.id).expect("run");
    assert_eq!(stored.kind, crate::workflows::RunKind::Configured);
    assert_eq!(stored.project_id, run.project_id);
    state
        .workflows
        .update(
            &record.id,
            record.revision,
            crate::workflows::definition::WorkflowDefinition::from_parts(
                "Later name".to_owned(),
                crate::tests::test_environment_id(),
                record.definition.roles().to_vec(),
                record.definition.steps().to_vec(),
            )
            .expect("later"),
        )
        .expect("update");
    let stored = state.workflow_runs.get(&run.id).expect("run");
    assert_eq!(stored.pinned.definition.name(), "One agent");
    assert_eq!(stored.pinned.workflow_id, Some(record.id));
}

#[tokio::test]
async fn a_quick_task_send_pins_the_system_definition() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(patch_send_quick(&state, &token))
        .await
        .expect("send");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    wait_until_job_idle(&state, &token).await;
    let run = state
        .workflow_runs
        .get(&state.workflow_runs.summaries()[0].id)
        .expect("run");
    assert_eq!(run.kind, crate::workflows::RunKind::QuickTask);
    assert_eq!(run.pinned.workflow_id, None);
    assert_eq!(run.pinned.definition.name(), "Quick task");
    assert_eq!(run.project_id, state.projects.list()[0].id);
}

#[tokio::test]
async fn a_quick_task_uses_the_pinned_agent_instructions_once() {
    let backend = ScriptedBackend::accept();
    let state = state_with_backend(backend.clone());
    let token = connected(&state).await;
    let agent = state.agents.list()[0].clone();
    state
        .agents
        .update(
            &agent.id,
            agent.revision,
            AgentDraft {
                name: agent.name,
                instructions: "Keep this exact instruction.".to_owned(),
                tools: agent.tools,
                directories: agent.directories,
                primary_directory: agent.primary_directory,
            },
        )
        .expect("update agent");

    let response = app(&state)
        .oneshot(patch_send_quick(&state, &token))
        .await
        .expect("send");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    wait_until_job_idle(&state, &token).await;

    let preamble = backend.last_preamble().expect("preamble");
    assert_eq!(preamble.matches("Keep this exact instruction.").count(), 1);
}

#[tokio::test]
async fn a_quick_task_does_not_need_a_workflow_catalogue() {
    let state = test_state();
    let token = connected(&state).await;
    for record in state.workflows.list() {
        state
            .workflows
            .delete(&record.id, record.revision)
            .expect("delete workflow");
    }
    assert!(state.workflows.list().is_empty());
    let response = app(&state)
        .oneshot(patch_send_quick(&state, &token))
        .await
        .expect("send");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    wait_until_job_idle(&state, &token).await;
    let run = state
        .workflow_runs
        .get(&state.workflow_runs.summaries()[0].id)
        .expect("run");
    assert_eq!(run.kind, crate::workflows::RunKind::QuickTask);
    assert_eq!(run.pinned.workflow_id, None);
}

#[tokio::test]
async fn an_unavailable_alpine_git_seed_rejects_a_quick_task() {
    let state = test_state();
    let token = connected(&state).await;
    let environment_id = crate::workflows::alpine_git_id(&state.environments).expect("alpine-git");
    let environment = state
        .environments
        .get(&environment_id)
        .expect("environment");
    state
        .environments
        .delete(&environment_id, environment.revision)
        .expect("retire alpine-git");
    let response = app(&state)
        .oneshot(patch_send_quick(&state, &token))
        .await
        .expect("send");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("That environment is no longer in the catalogue."));
    assert!(session_snapshot(&state, &token).job.is_none());
    assert!(state.workflow_runs.summaries().is_empty());
}

#[tokio::test]
async fn a_read_only_quick_task_omits_the_gate_and_commit() {
    let state = test_state();
    let token = connected(&state).await;
    let dir = tempfile::tempdir().expect("reader");
    git_init(dir.path());
    let agent = state
        .agents
        .create(AgentDraft {
            name: "Reader".to_owned(),
            instructions: "Only read files.".to_owned(),
            tools: vec![ToolId::List, ToolId::Read],
            directories: vec![DirectoryGrant {
                alias: "project".to_owned(),
                host_path: dir.path().to_path_buf(),
                access: AccessMode::ReadOnly,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("reader");
    let project = state
        .projects
        .create(
            "Reader project".to_owned(),
            agent.directories[0].host_path.clone(),
        )
        .expect("project");
    state.keep_temp_dir(dir);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/projects/{}/agents/{}",
                    project.id.as_hex(),
                    agent.id.as_hex()
                ))
                .header(header::COOKIE, cookie(&token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from("message=Explain&mode=quick"))
                .unwrap(),
        )
        .await
        .expect("send");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    for _ in 0..2_000 {
        if state
            .sessions
            .snapshot(
                &session_id(&token),
                &crate::sessions::ConversationKey {
                    project_id: project.id,
                    agent_id: agent.id,
                },
            )
            .expect("session")
            .job
            .is_some_and(|job| job.status != JobStatus::Running)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    let run = state
        .workflow_runs
        .get(&state.workflow_runs.summaries()[0].id)
        .expect("run");
    assert_eq!(run.kind, crate::workflows::RunKind::QuickTask);
    assert_eq!(run.pinned.definition.steps().len(), 1);
}

#[tokio::test]
async fn a_desk_document_uses_quick_task_as_the_default_send() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(document_show(&state, &token))
        .await
        .expect("document");
    let text = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("name=\"mode\""));
    assert!(text.contains("value=\"quick\""));
    assert!(text.contains("value=\"configured\""));
    assert!(text.contains("Configured workflow"));
    assert!(text.contains("Sandbox is ready"));
    assert!(!text.contains("data-observe-target=\"sandbox-status\""));
}

#[tokio::test]
async fn a_ready_sandbox_keeps_quick_task_enabled_during_replacement() {
    let state = test_state();
    let token = connected(&state).await;
    let environment_id = alpine_environment_id(&state);
    let environment = state
        .environments
        .get(&environment_id)
        .expect("environment");
    state
        .environments
        .retry_preparation(
            &environment.id,
            environment.revision,
            &environment.recipe_version,
        )
        .expect("replacement");
    let text = desk_html(&state, &token).await;
    assert!(text.contains("id=\"sandbox-status\""));
    assert!(text.contains("Sandbox is ready"));
    assert!(!text.contains("Sandbox preparation is in progress"));
    assert!(!text.contains("Sandbox preparation failed"));
    assert!(!text.contains(&configuration_href(&environment_id)));
    assert!(!text.contains("id=\"readiness-route\""));
    assert!(!quick_send_disabled(&text));

    let replacement = state
        .environments
        .claim_oldest_queued()
        .expect("claim replacement")
        .expect("replacement");
    state
        .environments
        .finish_failed(
            &replacement.id,
            crate::tests::FailureCategory::SetupExit,
            replacement.log,
        )
        .expect("failed replacement");
    let text = desk_html(&state, &token).await;
    assert!(text.contains("Sandbox is ready"));
    assert!(!text.contains("Sandbox preparation failed"));
    assert!(!quick_send_disabled(&text));
}

#[tokio::test]
async fn an_active_sandbox_disables_quick_task() {
    let state = test_state();
    let token = connect_session(&state).await;
    state.environments.apply_production_seeds();
    let environment_id = alpine_environment_id(&state);
    let text = desk_html(&state, &token).await;
    assert!(text.contains("Sandbox preparation is in progress"));
    assert!(!text.contains(&configuration_href(&environment_id)));
    assert!(quick_send_disabled(&text));
    assert!(!composer_message_disabled(&text));
    assert!(text.contains("data-island=\"observe\""));
    assert!(text.contains("data-observe-target=\"sandbox-status\""));
    assert!(text.contains("name=\"sandbox\""));
    assert!(text.contains(&format!("method=\"get\" action=\"{}\"", chat_path(&state))));
}

#[tokio::test]
async fn a_failed_sandbox_links_to_environment_configuration() {
    let state = test_state();
    let token = connect_session(&state).await;
    state.environments.apply_production_seeds();
    let environment_id = alpine_environment_id(&state);
    let preparation = state
        .environments
        .claim_oldest_queued()
        .expect("claim")
        .expect("queued");
    state
        .environments
        .finish_failed(
            &preparation.id,
            crate::tests::FailureCategory::SetupExit,
            preparation.log,
        )
        .expect("failed");
    let text = desk_html(&state, &token).await;
    assert!(text.contains("Sandbox preparation failed"));
    assert!(text.contains(&configuration_href(&environment_id)));
    assert!(text.contains("Environment configuration"));
    assert!(quick_send_disabled(&text));
}

#[tokio::test]
async fn an_invalid_sandbox_snapshot_links_to_environment_configuration() {
    let state = test_state();
    let token = connected(&state).await;
    let environment_id = alpine_environment_id(&state);
    let pointer = state
        .environments
        .copy_ready_pointer(&environment_id)
        .expect("ready pointer");
    state.environment_snapshots.mark(
        pointer.snapshot.artifact_key.clone(),
        crate::environments::SnapshotAvailability::Corrupt,
    );
    let text = desk_html(&state, &token).await;
    assert!(text.contains("Sandbox snapshot is invalid"));
    assert!(text.contains(&configuration_href(&environment_id)));
    assert!(quick_send_disabled(&text));
}

#[tokio::test]
async fn a_missing_alpine_git_seed_falls_back_to_environments() {
    let state = test_state();
    let token = connect_session(&state).await;
    assert!(
        state
            .environments
            .seed_id(crate::environments::seeds::ALPINE_GIT_V1)
            .is_none()
    );
    let text = desk_html(&state, &token).await;
    assert!(text.contains("Sandbox is unavailable"));
    assert!(text.contains("href=\"/environments\" data-graft"));
    assert!(quick_send_disabled(&text));
    assert!(!composer_message_disabled(&text));
}

#[tokio::test]
async fn a_missing_alpine_git_record_falls_back_to_environments() {
    let state = test_state();
    let token = connected(&state).await;
    let environment_id = alpine_environment_id(&state);
    let environment = state
        .environments
        .get(&environment_id)
        .expect("environment");
    state
        .environments
        .delete(&environment_id, environment.revision)
        .expect("delete alpine-git");
    assert!(
        state
            .environments
            .seed_id(crate::environments::seeds::ALPINE_GIT_V1)
            .is_some()
    );
    assert!(state.environments.get(&environment_id).is_none());
    let text = desk_html(&state, &token).await;
    assert!(text.contains("Sandbox is unavailable"));
    assert!(!text.contains(&configuration_href(&environment_id)));
    assert!(text.contains("href=\"/environments\" data-graft"));
    assert!(quick_send_disabled(&text));
}

#[tokio::test]
async fn malformed_and_oversized_sandbox_cursors_are_rejected_first() {
    let state = test_state();
    let token = connected(&state).await;
    for sandbox in ["not-valid", "0000000000000000-111111111111111111111"] {
        let response = app(&state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "{}?sandbox={sandbox}&cursor=nope",
                        chat_path(&state)
                    ))
                    .header(header::COOKIE, cookie(&token))
                    .header(hypergraft::GRAFT_REQUEST, "patch")
                    .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("sandbox observe");
        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNPROCESSABLE_ENTITY
        );
        let text = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(text.contains("target=\"sandbox-status\""));
        assert!(!text.contains("target=\"composer\""));
        assert!(!text.contains("target=\"job-observe\""));
    }
}

#[tokio::test]
async fn a_sandbox_observation_reports_an_active_status() {
    let state = test_state();
    let token = connect_session(&state).await;
    state.environments.apply_production_seeds();
    let (status, text) =
        sandbox_observe_text(&state, &token, &sandbox_cursor_token(&state), "").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(text.contains("target=\"sandbox-status\""));
    assert!(text.contains("target=\"composer\""));
    assert!(text.contains("Sandbox preparation is in progress"));
    assert!(text.contains("data-observe-target=\"sandbox-status\""));
    assert!(text.contains(&format!("method=\"get\" action=\"{}\"", chat_path(&state))));
    assert!(quick_send_disabled(&text));
}

#[tokio::test]
async fn a_sandbox_observation_reports_a_ready_status() {
    let state = test_state();
    let token = connected(&state).await;
    let (status, text) =
        sandbox_observe_text(&state, &token, &sandbox_cursor_token(&state), "").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(text.contains("target=\"sandbox-status\""));
    assert!(text.contains("target=\"composer\""));
    assert!(text.contains("Sandbox is ready"));
    assert!(!text.contains("data-observe-target=\"sandbox-status\""));
    assert!(!quick_send_disabled(&text));
}

#[tokio::test]
async fn a_sandbox_observation_reports_a_failed_status() {
    let state = test_state();
    let token = connect_session(&state).await;
    state.environments.apply_production_seeds();
    let environment_id = alpine_environment_id(&state);
    let preparation = state
        .environments
        .claim_oldest_queued()
        .expect("claim")
        .expect("queued");
    state
        .environments
        .finish_failed(
            &preparation.id,
            crate::tests::FailureCategory::SetupExit,
            preparation.log,
        )
        .expect("failed");
    let (status, text) =
        sandbox_observe_text(&state, &token, &sandbox_cursor_token(&state), "").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(text.contains("Sandbox preparation failed"));
    assert!(text.contains(&configuration_href(&environment_id)));
    assert!(!text.contains("data-observe-target=\"sandbox-status\""));
    assert!(quick_send_disabled(&text));
}

#[tokio::test]
async fn a_sandbox_observation_reports_an_invalid_status() {
    let state = test_state();
    let token = connected(&state).await;
    let environment_id = alpine_environment_id(&state);
    let pointer = state
        .environments
        .copy_ready_pointer(&environment_id)
        .expect("ready pointer");
    state.environment_snapshots.mark(
        pointer.snapshot.artifact_key.clone(),
        crate::environments::SnapshotAvailability::Corrupt,
    );
    let (status, text) =
        sandbox_observe_text(&state, &token, &sandbox_cursor_token(&state), "").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(text.contains("Sandbox snapshot is invalid"));
    assert!(!text.contains("data-observe-target=\"sandbox-status\""));
    assert!(quick_send_disabled(&text));
}

#[tokio::test]
async fn a_sandbox_observation_reports_an_unavailable_status() {
    let state = test_state();
    let token = connect_session(&state).await;
    let (status, text) =
        sandbox_observe_text(&state, &token, &sandbox_cursor_token(&state), "").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(text.contains("Sandbox is unavailable"));
    assert!(!text.contains("data-observe-target=\"sandbox-status\""));
    assert!(quick_send_disabled(&text));
}

#[tokio::test]
async fn an_active_sandbox_observation_enables_quick_task_after_refresh() {
    let state = test_state();
    let token = connect_session(&state).await;
    state.environments.apply_production_seeds();
    let (status, active) =
        sandbox_observe_text(&state, &token, &sandbox_cursor_token(&state), "").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(active.contains("Sandbox preparation is in progress"));
    assert!(quick_send_disabled(&active));
    let cursor = sandbox_cursor_from(&active);

    let environment_id = alpine_environment_id(&state);
    let environment = state
        .environments
        .get(&environment_id)
        .expect("environment");
    if environment.ready_preparation.is_none() {
        let preparation = state
            .environments
            .claim_oldest_queued()
            .expect("claim")
            .expect("queued");
        let snapshot = crate::tests::sample_snapshot(preparation.id);
        state.environment_snapshots.mark(
            snapshot.artifact_key.clone(),
            crate::environments::SnapshotAvailability::Available,
        );
        state
            .environments
            .finish_ready(&preparation.id, snapshot, preparation.log)
            .expect("ready");
    }

    let (status, ready) = sandbox_observe_text(&state, &token, &cursor, "").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(ready.contains("Sandbox is ready"));
    assert!(!ready.contains("data-observe-target=\"sandbox-status\""));
    assert!(!quick_send_disabled(&ready));
}

#[tokio::test]
async fn a_sandbox_observation_preserves_the_selected_workflow() {
    let state = test_state();
    let token = connect_session(&state).await;
    state.environments.apply_production_seeds();
    let environment_id = alpine_environment_id(&state);
    let first = state
        .workflows
        .create(crate::workflows::seeds::one_agent_definition(
            environment_id,
        ))
        .expect("first workflow");
    state
        .workflows
        .create(crate::workflows::seeds::read_only_review_definition(
            environment_id,
        ))
        .expect("second workflow");
    let selection = crate::workflows::WorkflowSelection {
        workflow_id: first.id,
        definition_version: first.definition_version,
    }
    .as_token();
    let response = app(&state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("{}?workflow={selection}", chat_path(&state)))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("select workflow");
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let (status, text) =
        sandbox_observe_text(&state, &token, &sandbox_cursor_token(&state), "").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let sandbox_name = text.find("name=\"sandbox\"").expect("sandbox field");
    let sandbox_form_end =
        text[sandbox_name..].find("</form>").expect("sandbox form") + sandbox_name;
    let sandbox_form = &text[sandbox_name..sandbox_form_end];
    assert!(sandbox_form.contains("name=\"workflow\""));
    assert!(sandbox_form.contains(&format!("value=\"{selection}\"")));
    let composer = text
        .split("operation=\"children\" target=\"composer\"")
        .nth(1)
        .expect("composer patch");
    assert!(opening_tag_for(composer, &format!("value=\"{selection}\"")).contains("selected"));
    assert!(text.contains("Sandbox preparation is in progress"));
}

#[tokio::test]
async fn a_quick_task_does_not_start_before_the_sandbox_is_ready() {
    let state = test_state();
    let token = connect_session(&state).await;
    state.environments.apply_production_seeds();
    let response = app(&state)
        .oneshot(patch_send_quick(&state, &token))
        .await
        .expect("send");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let text = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("That environment is not ready."));
    assert!(session_snapshot(&state, &token).job.is_none());
    assert!(state.workflow_runs.summaries().is_empty());
}

#[tokio::test]
async fn a_completed_quick_task_shows_task_finished_on_the_document_and_patch() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(patch_send_quick(&state, &token))
        .await
        .expect("send");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    wait_until_job_idle(&state, &token).await;
    assert_eq!(
        session_snapshot(&state, &token)
            .job
            .as_ref()
            .map(|job| job.status),
        Some(JobStatus::Completed)
    );
    let document = desk_html(&state, &token).await;
    assert!(opening_tag_for(&document, "Task finished.").contains("role=\"status\""));

    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(chat_path(&state))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("patch");
    let patch = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(patch.contains("target=\"job-observe\""));
    assert!(patch.contains("role=\"status\""));
    assert!(patch.contains("Task finished."));
}

#[tokio::test]
async fn a_completed_quick_task_shows_task_finished_on_the_final_stream() {
    let state = test_state();
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send_quick(&state, &token))
        .await
        .expect("send");
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
    assert!(final_frame.contains("Task finished."));
    assert!(final_frame.contains("role=\"status\""));
    assert!(!job_active(final_frame));
}

#[tokio::test]
async fn a_failed_quick_task_omits_task_finished() {
    let state = state_with_backend(ScriptedBackend::chunks([Err(ProviderError::Unreachable)]));
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send_quick(&state, &token))
        .await
        .expect("send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    wait_until_job_idle(&state, &token).await;
    assert_eq!(
        session_snapshot(&state, &token)
            .job
            .as_ref()
            .map(|job| job.status),
        Some(JobStatus::Failed)
    );
    let text = desk_html(&state, &token).await;
    assert!(!text.contains("Task finished."));
    let response = app(&state)
        .oneshot(observe_patch(&state, &token, &job, 0))
        .await
        .expect("observe");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let frames = stream_frames(&body);
    let final_frame = frames.last().expect("final frame");
    assert!(!final_frame.contains("Task finished."));
}

#[tokio::test]
async fn a_cancelled_quick_task_omits_task_finished() {
    let state = state_with_backend(ScriptedBackend::hang());
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send_quick(&state, &token))
        .await
        .expect("send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    assert!(!String::from_utf8_lossy(&started_body).contains("Task finished."));
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
    assert_eq!(
        session_snapshot(&state, &token)
            .job
            .as_ref()
            .map(|job| job.status),
        Some(JobStatus::Cancelled)
    );
    let text = desk_html(&state, &token).await;
    assert!(!text.contains("Task finished."));
}

#[tokio::test]
async fn a_completed_configured_run_omits_task_finished() {
    let state = test_state();
    let token = connected(&state).await;
    let started = app(&state)
        .oneshot(patch_send(&state, &token))
        .await
        .expect("send");
    let started_body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job = job_id_from_body(&started_body);
    wait_until_job_idle(&state, &token).await;
    assert_eq!(
        session_snapshot(&state, &token)
            .job
            .as_ref()
            .map(|job| job.status),
        Some(JobStatus::Completed)
    );
    let stored = state
        .workflow_runs
        .get(&state.workflow_runs.summaries()[0].id)
        .expect("run");
    assert_eq!(stored.kind, crate::workflows::RunKind::Configured);
    let text = desk_html(&state, &token).await;
    assert!(!text.contains("Task finished."));
    let response = app(&state)
        .oneshot(observe_patch(&state, &token, &job, 0))
        .await
        .expect("observe");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let frames = stream_frames(&body);
    let final_frame = frames.last().expect("final frame");
    assert!(!final_frame.contains("Task finished."));
}
