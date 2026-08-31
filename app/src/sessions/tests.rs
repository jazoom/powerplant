use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    middleware::from_fn_with_state,
    routing::get,
};
use cookie::Cookie;
use tower::ServiceExt;

use crate::agents::{AgentId, DirectoryPolicy};
use crate::providers::{ProviderConnection, ProviderKind};
use crate::sessions::{Job, JobId, SESSION_LIFETIME, SessionId, ValidatedToken};
use crate::state::AppState;
use crate::workflows::RunId;

fn state_with_gate(name: &str) -> (AppState, ValidatedToken, SessionId, RunId) {
    let state = crate::state::for_test(crate::config::RuntimeConfig::development_for_test());
    state
        .vault
        .put(ProviderConnection::with_key(
            ProviderKind::Xai,
            "key",
            "model",
        ))
        .expect("provider");
    let token = super::generate_session_token().expect("token");
    let session = token.id();
    let raw = token.raw().clone();
    state.sessions.insert(session);

    let definition = crate::workflows::definition::test_named_definition(name);
    let environments = crate::workflows::test_environment_set(&definition);
    let mut run = crate::workflows::WorkflowRun::create_for_test(
        RunId::generate().expect("run"),
        1,
        AgentId::generate().expect("agent"),
        crate::workflows::definition::PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    run.state = crate::workflows::run::RunState::InitialisingSource;
    let run_id = run.id;
    state.workflow_runs.create(run).expect("store run");
    let continuation = crate::workflows::WorkflowJob {
        run_id,
        session_id: session,
        project_id: crate::projects::ProjectId::generate().expect("project"),
        agent_id: AgentId::generate().expect("agent"),
        agent_revision: 1,
        grant_alias: "project".to_owned(),
        grant_access: crate::agents::AccessMode::ReadWrite,
        connection: ProviderConnection::with_key(ProviderKind::Xai, "key", "model"),
        host_policy: DirectoryPolicy::from_grants(Vec::new(), "project".to_owned()),
        turns: Vec::new(),
        job: Job::new(JobId::generate().expect("job"), run_id, 0),
        eligible_reply: Arc::new(std::sync::Mutex::new(String::new())),
    };
    assert!(state.gate_continuations.insert(continuation));
    (state, raw, session, run_id)
}

#[test]
fn request_time_expiry_interrupts_a_gate_before_session_restore() {
    let (state, token, session, run_id) = state_with_gate("Expiry");
    state
        .sessions
        .advance_clock(SESSION_LIFETIME + std::time::Duration::from_secs(1));

    assert!(matches!(
        super::existing_or_restore(&state, &token),
        super::ResolvedSession::Present(id) if id == session
    ));

    assert!(!state.gate_continuations.available(&run_id, &session));
    assert_eq!(
        state.workflow_runs.get(&run_id).expect("run").state,
        crate::workflows::run::RunState::Interrupted
    );
    assert!(state.sessions.contains_live(&session));
}

#[tokio::test]
async fn a_live_cookie_after_final_provider_removal_revokes_the_session() {
    let (state, token, session, run_id) = state_with_gate("Forget");
    state.vault.forget(ProviderKind::Xai).expect("forget");
    assert!(!state.vault.has_providers());
    assert!(state.sessions.contains_live(&session));

    let app = axum::Router::new()
        .route(
            "/private",
            get(|_session: super::RequiredSession| async { StatusCode::NO_CONTENT }),
        )
        .layer(from_fn_with_state(state.clone(), super::resolve_session))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/private")
                .header(
                    header::COOKIE,
                    format!("powerplant_session={}", token.as_str()),
                )
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/connect"
    );
    let deletion = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| Cookie::parse(value.to_owned()).ok())
        .find(|cookie| cookie.name() == "powerplant_session")
        .expect("session deletion cookie");
    assert!(deletion.value().is_empty());
    assert_eq!(deletion.max_age(), Some(cookie::time::Duration::ZERO));
    assert!(!state.gate_continuations.available(&run_id, &session));
    assert_eq!(
        state.workflow_runs.get(&run_id).expect("run").state,
        crate::workflows::run::RunState::Interrupted
    );
    assert!(!state.sessions.contains(&session));
}
