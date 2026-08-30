use std::sync::Arc;

use crate::agents::{AgentId, DirectoryPolicy};
use crate::providers::{ProviderConnection, ProviderKind};
use crate::sessions::{Job, JobId, SESSION_LIFETIME};

#[test]
fn request_time_expiry_interrupts_a_gate_before_session_restore() {
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
    state.sessions.insert(session);

    let definition = crate::workflows::definition::test_named_definition("Expiry");
    let environments = crate::workflows::test_environment_set(&definition);
    let mut run = crate::workflows::WorkflowRun::create(
        crate::workflows::RunId::generate().expect("run"),
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
        agent_id: AgentId::generate().expect("agent"),
        connection: ProviderConnection::with_key(ProviderKind::Xai, "key", "model"),
        host_policy: DirectoryPolicy::from_grants(Vec::new(), "project".to_owned()),
        turns: Vec::new(),
        job: Job::new(JobId::generate().expect("job"), run_id, 0),
        eligible_reply: Arc::new(std::sync::Mutex::new(String::new())),
    };
    assert!(state.gate_continuations.insert(continuation));
    state
        .sessions
        .advance_clock(SESSION_LIFETIME + std::time::Duration::from_secs(1));

    assert!(matches!(
        super::existing_or_restore(&state, token.raw()),
        super::ResolvedSession::Present(id) if id == session
    ));

    assert!(!state.gate_continuations.available(&run_id, &session));
    assert_eq!(
        state.workflow_runs.get(&run_id).expect("run").state,
        crate::workflows::run::RunState::Interrupted
    );
    assert!(state.sessions.contains_live(&session));
}
