use super::{
    ActionKind, AttemptCleanupRecord, AttemptId, AttemptRecord, AttemptResult, AttemptSandboxKind,
    AttemptSandboxRecord, AttemptState, FailureCategory, RunState, TransitionCause,
    TransitionError, WorkflowRun, next_ordinal_for,
};
use crate::agents::{AccessMode, ToolId};
use crate::workflows::capabilities::{test_agent_capabilities, test_command_capabilities};
use crate::workflows::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, ArtefactKind, ArtefactSource, GuestDirectoryAccess,
    InputKey, OutputKey, OutputKind, PinnedWorkflowDefinition, RequiredInput, RequiredOutput,
    RoleDefinition, RoleKey, StepAction, StepDefinition, StepEnvironment, StepKey,
    SuccessTransition, SystemCommandId, SystemCommandStep, WorkflowDefinition,
    candidate_revision_output, initial_candidate_input, test_environment_id,
};
use crate::workflows::id::RunId;

fn definition() -> WorkflowDefinition {
    two_step(false)
}

fn two_step(include_command: bool) -> WorkflowDefinition {
    let role = RoleDefinition::new(
        RoleKey::parse("agent").expect("role"),
        "Maintainer".to_owned(),
        String::new(),
        String::new(),
    )
    .expect("role");
    let authority = AgentAuthority::new(
        vec![ToolId::List],
        vec![GuestDirectoryAccess {
            alias: "project".to_owned(),
            access: AccessMode::ReadWrite,
        }],
    )
    .expect("authority");
    let reply = StepDefinition {
        key: StepKey::parse("reply").expect("step"),
        name: "Reply".to_owned(),
        inputs: vec![initial_candidate_input()],
        action: StepAction::Agent(AgentStep {
            environment: StepEnvironment::WorkflowDefault,
            role: RoleKey::parse("agent").expect("role"),
            authority,
            required_outputs: vec![
                RequiredOutput {
                    key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                    kind: OutputKind::AssistantReply,
                },
                candidate_revision_output(),
            ],
        }),
        on_success: if include_command {
            SuccessTransition::Next(StepKey::parse("status").expect("next"))
        } else {
            SuccessTransition::CompleteRun
        },
    };
    let mut steps = vec![reply];
    if include_command {
        steps.push(StepDefinition {
            key: StepKey::parse("status").expect("step"),
            name: "Status".to_owned(),
            inputs: vec![RequiredInput {
                key: InputKey::parse("candidate").expect("input"),
                kind: ArtefactKind::CandidateRevision,
                source: ArtefactSource::StepOutput {
                    step: StepKey::parse("reply").expect("step"),
                    output: OutputKey::parse("candidate").expect("output"),
                },
            }],
            action: StepAction::SystemCommand(SystemCommandStep {
                environment: StepEnvironment::WorkflowDefault,
                command: SystemCommandId::RepositoryStatus,
                required_outputs: Vec::new(),
            }),
            on_success: SuccessTransition::CompleteRun,
        });
    }
    let first = StepKey::parse("reply").expect("first");
    WorkflowDefinition::from_parts(
        "Maintainer".to_owned(),
        test_environment_id(),
        vec![role],
        first,
        steps,
    )
    .expect("definition")
}

fn new_run() -> WorkflowRun {
    let definition = definition();
    let environments = crate::workflows::test_environment_set(&definition);
    WorkflowRun::create(
        RunId::generate().expect("run"),
        10,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    )
}

fn sandbox_record(run: &WorkflowRun) -> AttemptSandboxRecord {
    let digest = run.environments.steps[0].snapshot_digest.clone();
    AttemptSandboxRecord {
        kind: AttemptSandboxKind::IsolatedAttempt,
        snapshot_digest: digest,
    }
}

fn start(run: &mut WorkflowRun) -> AttemptId {
    let attempt = AttemptId::generate().expect("attempt");
    let caps = match run.pinned.definition.step(run.ready_step().expect("step")) {
        Some(step)
            if matches!(
                step.action,
                crate::workflows::definition::StepAction::SystemCommand(_)
            ) =>
        {
            test_command_capabilities()
        }
        _ => test_agent_capabilities(),
    };
    run.start_attempt(attempt, Vec::new(), caps, sandbox_record(run), 11)
        .expect("start");
    attempt
}

fn complete(run: &mut WorkflowRun, attempt: AttemptId, at_ms: u64) {
    run.record_cleanup(attempt, AttemptCleanupRecord::Complete)
        .expect("cleanup");
    run.complete_attempt(attempt, at_ms).expect("complete");
}

fn fail(run: &mut WorkflowRun, attempt: AttemptId, category: FailureCategory, at_ms: u64) {
    run.record_cleanup(attempt, AttemptCleanupRecord::Complete)
        .expect("cleanup");
    run.fail_attempt(attempt, category, at_ms).expect("fail");
}

#[test]
fn creation_stores_ready_without_a_transition() {
    let run = new_run();
    assert!(matches!(run.state, RunState::Ready { .. }));
    assert!(run.transitions.is_empty());
    assert!(run.attempts.is_empty());
}

#[test]
fn parallel_attempts_are_rejected() {
    let mut run = new_run();
    start(&mut run);
    let second = AttemptId::generate().expect("attempt");
    assert_eq!(
        run.start_attempt(
            second,
            Vec::new(),
            test_agent_capabilities(),
            sandbox_record(&run),
            12,
        ),
        Err(TransitionError::Invalid)
    );
    assert_eq!(run.attempts.len(), 1);
    assert_eq!(run.transitions.len(), 1);
}

#[test]
fn stale_completions_are_rejected() {
    let mut run = new_run();
    start(&mut run);
    let stale = AttemptId::generate().expect("attempt");
    assert_eq!(
        run.complete_attempt(stale, 12),
        Err(TransitionError::Invalid)
    );
    assert!(matches!(run.state, RunState::Active { .. }));
}

#[test]
fn transition_times_cannot_move_backwards() {
    let mut run = new_run();
    let attempt = start(&mut run);
    assert_eq!(
        run.fail_attempt(attempt, FailureCategory::Provider, 10),
        Err(TransitionError::Invalid)
    );
    assert!(matches!(run.state, RunState::Active { .. }));
}

#[test]
fn duplicate_terminal_results_are_rejected() {
    let mut run = new_run();
    let attempt = start(&mut run);
    complete(&mut run, attempt, 12);
    assert_eq!(
        run.complete_attempt(attempt, 13),
        Err(TransitionError::Invalid)
    );
    assert_eq!(
        run.fail_attempt(attempt, FailureCategory::Provider, 13),
        Err(TransitionError::Invalid)
    );
}

#[test]
fn transitions_after_a_terminal_state_are_rejected() {
    let mut run = new_run();
    let attempt = start(&mut run);
    fail(&mut run, attempt, FailureCategory::Provider, 12);
    assert_eq!(run.cancel(13), Err(TransitionError::Invalid));
    assert_eq!(
        run.start_attempt(
            AttemptId::generate().expect("attempt"),
            Vec::new(),
            test_agent_capabilities(),
            sandbox_record(&run),
            13,
        ),
        Err(TransitionError::Invalid)
    );
    assert_eq!(run.interrupt(13), Err(TransitionError::Invalid));
}

#[test]
fn completed_attempts_follow_on_success() {
    let definition = two_step(true);
    let environments = crate::workflows::test_environment_set(&definition);
    let mut run = WorkflowRun::create(
        RunId::generate().expect("run"),
        10,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    let first = AttemptId::generate().expect("attempt");
    run.start_attempt(
        first,
        Vec::new(),
        test_agent_capabilities(),
        sandbox_record(&run),
        11,
    )
    .expect("start");
    complete(&mut run, first, 12);
    assert!(matches!(run.state, RunState::Ready { ref step } if step.as_str() == "status"));
    let second = AttemptId::generate().expect("attempt");
    run.start_attempt(
        second,
        Vec::new(),
        test_command_capabilities(),
        sandbox_record(&run),
        13,
    )
    .expect("start");
    complete(&mut run, second, 14);
    assert_eq!(run.state, RunState::Completed);
}

#[test]
fn failed_attempts_move_the_run_to_failed() {
    let mut run = new_run();
    let attempt = start(&mut run);
    fail(&mut run, attempt, FailureCategory::Command, 12);
    assert_eq!(run.state, RunState::Failed);
    assert_eq!(
        run.transitions.last().map(|item| item.cause),
        Some(TransitionCause::AttemptFailed)
    );
}

#[test]
fn cancellation_moves_the_run_directly_to_cancelled() {
    let mut run = new_run();
    let attempt = start(&mut run);
    run.record_cleanup(attempt, AttemptCleanupRecord::Complete)
        .expect("cleanup");
    run.cancel(12).expect("cancel");
    assert_eq!(run.state, RunState::Cancelled);
    assert_eq!(run.attempts[0].state, AttemptState::Cancelled);
}

#[test]
fn attempt_ordinals_count_repeated_step_attempts() {
    let step = StepKey::parse("reply").expect("step");
    let first = AttemptRecord {
        id: AttemptId::generate().expect("attempt"),
        step: step.clone(),
        ordinal: 1,
        action_kind: ActionKind::Agent,
        started_at_ms: 1,
        finished_at_ms: Some(2),
        state: AttemptState::Failed,
        result: Some(AttemptResult::Failed {
            category: FailureCategory::Provider,
        }),
        inputs: Vec::new(),
        outputs: Vec::new(),
        capabilities: test_agent_capabilities(),
        sandbox: AttemptSandboxRecord {
            kind: AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest: crate::environments::SnapshotDigest::parse(&format!(
                "sha256:{}",
                "a".repeat(64)
            ))
            .expect("digest"),
        },
        cleanup: AttemptCleanupRecord::Complete,
        commit_transaction: None,
        commit_result: None,
    };
    let second = AttemptRecord {
        id: AttemptId::generate().expect("attempt"),
        step: step.clone(),
        ordinal: 2,
        action_kind: ActionKind::Agent,
        started_at_ms: 3,
        finished_at_ms: None,
        state: AttemptState::Active,
        result: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        capabilities: test_agent_capabilities(),
        sandbox: AttemptSandboxRecord {
            kind: AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest: crate::environments::SnapshotDigest::parse(&format!(
                "sha256:{}",
                "a".repeat(64)
            ))
            .expect("digest"),
        },
        cleanup: AttemptCleanupRecord::Pending,
        commit_transaction: None,
        commit_result: None,
    };
    assert_eq!(next_ordinal_for(&[], &step), 1);
    assert_eq!(next_ordinal_for(std::slice::from_ref(&first), &step), 2);
    assert_eq!(next_ordinal_for(&[first, second], &step), 3);
}
