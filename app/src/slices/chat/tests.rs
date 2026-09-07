use std::sync::Arc;

use askama::Template;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    agents::{AccessMode, AgentDraft, DirectoryGrant, ToolId},
    config::RuntimeConfig,
    providers::{
        AssistantActivity, AssistantReply, ChatBackend, ProviderConnection, ProviderError,
        ProviderKind, ToolOutput, tests::ScriptedBackend,
    },
    sessions::{self, JobStatus},
    state::AppState,
};

#[test]
fn tool_output_stays_html_text_in_the_transcript() {
    let turn = super::page::assistant_reply_turn(
        1,
        &AssistantReply {
            text: "Done.".to_owned(),
            thinking: String::new(),
            tools: vec![ToolOutput {
                label: "<img src=x onerror=alert(1)>".to_owned(),
                output: "</code><script>alert(1)</script>".to_owned(),
            }],
            activity: Vec::new(),
            usage: None,
        },
        false,
    );
    let html = super::page::TurnArticle { turn: &turn }
        .render()
        .expect("turn");
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img src=x"));
    assert!(html.contains("&lt;") || html.contains("&#60;"));
    assert!(html.contains("img src=x"));
    assert!(html.contains("script"));
}

#[test]
fn assistant_activity_keeps_thinking_and_tools_in_event_order() {
    let tool = ToolOutput {
        label: "read `/project/src/lib.rs`".to_owned(),
        output: "source".to_owned(),
    };
    let turn = super::page::assistant_reply_turn(
        1,
        &AssistantReply {
            text: "Done.".to_owned(),
            thinking: "Inspect the projectUse the result".to_owned(),
            tools: vec![tool.clone()],
            activity: vec![
                AssistantActivity::Thinking("Inspect the project".to_owned()),
                AssistantActivity::Tool(tool),
                AssistantActivity::Thinking("Use the result".to_owned()),
            ],
            usage: None,
        },
        false,
    );
    let html = super::page::TurnArticle { turn: &turn }
        .render()
        .expect("turn");

    let first_thought = html.find("Inspect the project").expect("first thought");
    let tool = html.find("read `/project/src/lib.rs`").expect("tool");
    let second_thought = html.find("Use the result").expect("second thought");
    assert!(first_thought < tool);
    assert!(tool < second_thought);
}

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
            selection: None,
            tools: ToolId::ALL.to_vec(),
            network: crate::agents::NetworkAccess::None,
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
async fn malformed_thinking_visibility_is_rejected() {
    let state = test_state();
    let token = connected(&state).await;

    let response = app(&state)
        .oneshot(thinking_visibility_patch(&token, "show_thinking=unknown"))
        .await
        .expect("thinking visibility");

    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(!state.preferences.show_thinking());
}

fn thinking_visibility_patch(token: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/thinking-visibility")
        .header(header::COOKIE, cookie(token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn the_desk_updates_the_thinking_level() {
    let state = test_state();
    let token = connected(&state).await;
    let response = app(&state)
        .oneshot(model_update_patch(
            &state,
            &token,
            "provider=xai&model=grok-4.6&thinking=high",
        ))
        .await
        .expect("thinking update");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        state
            .preferences
            .selected_provider(&state.vault)
            .map(|item| item.thinking),
        Some(Some(
            crate::providers::ThinkingEffort::new("high".to_owned()).unwrap()
        ))
    );
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
        state
            .preferences
            .selected_provider(&state.vault)
            .map(|item| item.model),
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
        state
            .preferences
            .selected_provider(&state.vault)
            .map(|item| item.model),
        Some("grok-4.6".to_owned())
    );
}

#[tokio::test]
async fn the_model_live_projection_sends_current_catalogue_metadata() {
    let state = test_state();
    let token = connected(&state).await;
    let url = format!(
        "/model?project={}&agent={}",
        project_hex(&state),
        agent_hex(&state)
    );
    let harness = hypergraft::live::LiveHarness::new(super::live_router(), state.clone());
    let projection = harness
        .subscribe(&url, TestLiveGuard(session_id(&token)))
        .await
        .expect("live model projection");

    assert_eq!(projection.first_patch().targets.len(), 3);
    assert_eq!(
        projection.first_patch().targets[0].target,
        "desk-model-catalogue"
    );
    assert_eq!(
        projection.first_patch().targets[1].target,
        "desk-thinking-control"
    );
    assert_eq!(
        projection.first_patch().targets[2].target,
        "desk-model-context"
    );
    assert!(
        projection.first_patch().targets[0]
            .html
            .contains("grok-4.6")
    );
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
        .preferences
        .select_settings(ProviderKind::OpenaiCodex, "gpt-5.1-codex".to_owned(), None)
        .unwrap();
    state
        .preferences
        .select_settings(ProviderKind::Xai, "grok-4.6".to_owned(), None)
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
    let selected = state.preferences.selected_provider(&state.vault).unwrap();
    assert_eq!(selected.kind, ProviderKind::OpenaiCodex);
    assert_eq!(selected.model, "gpt-5.1-codex");
}

#[tokio::test]
async fn provider_changes_preserve_each_providers_saved_thinking_effort() {
    let state = test_state();
    let token = connected(&state).await;
    state
        .vault
        .put(ProviderConnection::with_key(
            ProviderKind::OpenaiCodex,
            "test-openai-key",
            "gpt-5.2",
        ))
        .unwrap();
    state
        .preferences
        .select_settings(
            ProviderKind::OpenaiCodex,
            "gpt-5.2".to_owned(),
            crate::providers::ThinkingEffort::new("low".to_owned()),
        )
        .unwrap();
    state
        .preferences
        .select_settings(
            ProviderKind::Xai,
            "grok-4.6".to_owned(),
            crate::providers::ThinkingEffort::new("high".to_owned()),
        )
        .unwrap();

    let openai = app(&state)
        .oneshot(model_update_patch(
            &state,
            &token,
            "provider=openai-codex&model=grok-4.6",
        ))
        .await
        .expect("openai provider change");
    assert_eq!(openai.status(), axum::http::StatusCode::OK);
    let selected = state
        .preferences
        .selected_provider(&state.vault)
        .expect("openai selected");
    assert_eq!(selected.kind, ProviderKind::OpenaiCodex);
    assert_eq!(selected.model, "gpt-5.2");
    assert_eq!(
        selected.thinking.as_ref().map(|value| value.as_str()),
        Some("low")
    );

    let xai = app(&state)
        .oneshot(model_update_patch(
            &state,
            &token,
            "provider=xai&model=gpt-5.2",
        ))
        .await
        .expect("xai provider change");
    assert_eq!(xai.status(), axum::http::StatusCode::OK);
    let selected = state
        .preferences
        .selected_provider(&state.vault)
        .expect("xai selected");
    assert_eq!(selected.kind, ProviderKind::Xai);
    assert_eq!(selected.model, "grok-4.6");
    assert_eq!(
        selected.thinking.as_ref().map(|value| value.as_str()),
        Some("high")
    );
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

        let desk = state.preferences.desk_providers(&state.vault);
        let favourites = &desk
            .iter()
            .find(|provider| provider.kind == ProviderKind::Xai)
            .expect("xai stored")
            .favourites;
        assert_eq!(favourites.contains(&"grok-4-mini".to_owned()), expected);
        assert_eq!(
            state
                .preferences
                .selected_provider(&state.vault)
                .map(|item| item.model),
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
        state
            .preferences
            .selected_provider(&state.vault)
            .map(|item| item.model),
        Some("grok-4.6".to_owned())
    );
}
