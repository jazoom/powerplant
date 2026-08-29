use super::{
    ActionKind, AttemptId, AttemptRecord, AttemptResult, AttemptState, FailureCategory, RunState,
    TransitionCause, TransitionError, WorkflowRun, next_ordinal_for,
};
use crate::agents::{AccessMode, ToolId};
use crate::workflows::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, GuestDirectoryAccess, OutputKey, OutputKind,
    PinnedWorkflowDefinition, RequiredOutput, RoleDefinition, RoleKey, StepAction, StepDefinition,
    StepKey, SuccessTransition, SystemCommandId, SystemCommandStep, WorkflowDefinition,
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
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse("agent").expect("role"),
            authority,
            required_outputs: vec![RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                kind: OutputKind::AssistantReply,
            }],
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
            action: StepAction::SystemCommand(SystemCommandStep {
                command: SystemCommandId::RepositoryStatus,
                required_outputs: Vec::new(),
            }),
            on_success: SuccessTransition::CompleteRun,
        });
    }
    let first = StepKey::parse("reply").expect("first");
    WorkflowDefinition::from_parts("Maintainer".to_owned(), vec![role], first, steps)
        .expect("definition")
}

fn new_run() -> WorkflowRun {
    WorkflowRun::create(
        RunId::generate().expect("run"),
        10,
        PinnedWorkflowDefinition::pin(None, definition()),
    )
}

fn start(run: &mut WorkflowRun) -> AttemptId {
    let attempt = AttemptId::generate().expect("attempt");
    run.start_attempt(attempt, 11).expect("start");
    attempt
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
    assert_eq!(run.start_attempt(second, 12), Err(TransitionError::Invalid));
    assert_eq!(run.attempts.len(), 1);
    assert_eq!(run.transitions.len(), 1);
}

#[test]
fn stale_completions_are_rejected() {
    let mut run = new_run();
    start(&mut run);
    let stale = AttemptId::generate().expect("attempt");
    assert_eq!(
        run.complete_attempt(stale, vec![ASSISTANT_REPLY.to_owned()], 12),
        Err(TransitionError::Invalid)
    );
    assert!(matches!(run.state, RunState::Active { .. }));
}

#[test]
fn completion_requires_the_declared_outputs() {
    let mut run = new_run();
    let attempt = start(&mut run);
    assert_eq!(
        run.complete_attempt(attempt, Vec::new(), 12),
        Err(TransitionError::Invalid)
    );
    assert!(matches!(run.state, RunState::Active { .. }));
    assert_eq!(run.attempts[0].state, AttemptState::Active);
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
    run.complete_attempt(attempt, vec![ASSISTANT_REPLY.to_owned()], 12)
        .expect("complete");
    assert_eq!(
        run.complete_attempt(attempt, vec![ASSISTANT_REPLY.to_owned()], 13),
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
    run.fail_attempt(attempt, FailureCategory::Provider, 12)
        .expect("fail");
    assert_eq!(run.cancel(13), Err(TransitionError::Invalid));
    assert_eq!(
        run.start_attempt(AttemptId::generate().expect("attempt"), 13),
        Err(TransitionError::Invalid)
    );
    assert_eq!(run.interrupt(13), Err(TransitionError::Invalid));
}

#[test]
fn completed_attempts_follow_on_success() {
    let mut run = WorkflowRun::create(
        RunId::generate().expect("run"),
        10,
        PinnedWorkflowDefinition::pin(None, two_step(true)),
    );
    let first = AttemptId::generate().expect("attempt");
    run.start_attempt(first, 11).expect("start");
    run.complete_attempt(first, vec![ASSISTANT_REPLY.to_owned()], 12)
        .expect("complete");
    assert!(matches!(run.state, RunState::Ready { ref step } if step.as_str() == "status"));
    let second = AttemptId::generate().expect("attempt");
    run.start_attempt(second, 13).expect("start");
    run.complete_attempt(second, Vec::new(), 14)
        .expect("complete");
    assert_eq!(run.state, RunState::Completed);
}

#[test]
fn failed_attempts_move_the_run_to_failed() {
    let mut run = new_run();
    let attempt = start(&mut run);
    run.fail_attempt(attempt, FailureCategory::Command, 12)
        .expect("fail");
    assert_eq!(run.state, RunState::Failed);
    assert_eq!(
        run.transitions.last().map(|item| item.cause),
        Some(TransitionCause::AttemptFailed)
    );
}

#[test]
fn cancellation_moves_the_run_directly_to_cancelled() {
    let mut run = new_run();
    start(&mut run);
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
    };
    assert_eq!(next_ordinal_for(&[], &step), 1);
    assert_eq!(next_ordinal_for(std::slice::from_ref(&first), &step), 2);
    assert_eq!(next_ordinal_for(&[first, second], &step), 3);
}
