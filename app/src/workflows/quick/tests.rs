use super::{
    AGENT_STEP_KEY, COMMIT_STEP_KEY, COMMITTED_OUTPUT_KEY, DECISION_OUTPUT_KEY, DefinitionError,
    EnvironmentId, GATE_STEP_KEY, PinnedWorkflowDefinition, QUICK_TASK_NAME, ROLE_KEY,
    pin_quick_task_with_context,
};
use crate::agents::{AccessMode, ToolId};
use crate::tests::test_environment_id;
use crate::workflows::commands::SystemCommandId;
use crate::workflows::definition::{
    ASSISTANT_REPLY, ArtefactKind, ArtefactSource, CandidateAuthority, OutputKind, StepAction,
};

fn pin(
    access: AccessMode,
    tools: &[ToolId],
) -> crate::workflows::definition::PinnedWorkflowDefinition {
    pin_quick_task(access, tools, "Fix the bug.", test_environment_id()).expect("quick task")
}

#[test]
fn a_read_only_definition_has_one_inspect_step() {
    let pinned = pin(AccessMode::ReadOnly, &[ToolId::List, ToolId::Read]);
    assert_eq!(pinned.workflow_id, None);
    assert_eq!(pinned.definition.name(), QUICK_TASK_NAME);
    assert_eq!(
        pinned.definition.default_environment(),
        test_environment_id()
    );
    assert_eq!(pinned.definition.roles().len(), 1);
    let role = &pinned.definition.roles()[0];
    assert_eq!(role.key.as_str(), ROLE_KEY);
    assert_eq!(role.prompt_defaults, "Fix the bug.");
    assert_eq!(pinned.definition.steps().len(), 1);
    let work = &pinned.definition.steps()[0];
    assert_eq!(work.key.as_str(), AGENT_STEP_KEY);
    assert_eq!(work.inputs.len(), 1);
    assert_eq!(work.inputs[0].source, ArtefactSource::RunInitialCandidate);
    let StepAction::Agent(action) = &work.action else {
        panic!("agent step");
    };
    assert_eq!(action.candidate_authority, CandidateAuthority::ReadOnly);
    assert_eq!(action.authority.tools, vec![ToolId::List, ToolId::Read]);
    assert!(action.authority.directories.is_empty());
    assert_eq!(action.required_outputs.len(), 1);
    assert_eq!(action.required_outputs[0].key.as_str(), ASSISTANT_REPLY);
    assert_eq!(action.required_outputs[0].kind, OutputKind::AssistantReply);
}

#[test]
fn a_writable_definition_gates_and_commits_the_candidate() {
    let pinned = pin(AccessMode::ReadWrite, &ToolId::ALL);
    assert_eq!(pinned.workflow_id, None);
    assert_eq!(pinned.definition.steps().len(), 3);
    let work = &pinned.definition.steps()[0];
    let StepAction::Agent(action) = &work.action else {
        panic!("agent step");
    };
    assert_eq!(action.candidate_authority, CandidateAuthority::Edit);
    assert_eq!(action.authority.tools, ToolId::ALL.to_vec());
    assert!(action.authority.directories.is_empty());
    assert_eq!(
        action
            .required_outputs
            .iter()
            .map(|output| output.kind)
            .collect::<Vec<_>>(),
        vec![OutputKind::AssistantReply, OutputKind::CandidateRevision]
    );

    let gate = &pinned.definition.steps()[1];
    assert_eq!(gate.key.as_str(), GATE_STEP_KEY);
    assert_eq!(gate.inputs.len(), 1);
    assert_eq!(gate.inputs[0].kind, ArtefactKind::CandidateRevision);
    assert_eq!(
        gate.inputs[0].source,
        ArtefactSource::StepOutput {
            step: crate::workflows::definition::StepKey::parse(AGENT_STEP_KEY).expect("work"),
            output: crate::workflows::definition::OutputKey::parse("candidate").expect("candidate"),
        }
    );
    let StepAction::HumanGate(action) = &gate.action else {
        panic!("gate");
    };
    assert_eq!(action.required_output.key.as_str(), DECISION_OUTPUT_KEY);
    assert_eq!(action.required_output.kind, OutputKind::HumanDecision);

    let commit = &pinned.definition.steps()[2];
    assert_eq!(commit.key.as_str(), COMMIT_STEP_KEY);
    assert_eq!(commit.inputs.len(), 2);
    assert_eq!(commit.inputs[0].kind, ArtefactKind::CandidateRevision);
    assert_eq!(commit.inputs[1].kind, ArtefactKind::HumanDecision);
    assert!(
        !commit
            .inputs
            .iter()
            .any(|input| input.kind == ArtefactKind::ReviewReport)
    );
    let StepAction::SystemCommand(action) = &commit.action else {
        panic!("commit");
    };
    assert_eq!(action.command, SystemCommandId::CommitCandidate);
    assert_eq!(action.required_outputs.len(), 1);
    assert_eq!(
        action.required_outputs[0].key.as_str(),
        COMMITTED_OUTPUT_KEY
    );
    assert_eq!(
        action.required_outputs[0].kind,
        OutputKind::CandidateRevision
    );
}

#[test]
fn project_free_review_prepares_and_applies_without_a_commit_step() {
    let pinned = super::pin_project_free_quick_task_with_directories(
        &[ToolId::Read, ToolId::Write],
        "Change the file.",
        test_environment_id(),
        vec![crate::workflows::definition::GuestDirectoryAccess {
            alias: "source".to_owned(),
            access: AccessMode::ReadOnly,
        }],
        true,
    )
    .expect("reviewed quick task");

    assert_eq!(pinned.definition.steps().len(), 3);
    assert!(pinned.definition.steps()[0].writes_primary_source());
    assert!(matches!(
        pinned.definition.steps()[1].action,
        StepAction::HumanGate(_)
    ));
    assert!(matches!(
        &pinned.definition.steps()[2].action,
        StepAction::SystemCommand(action)
            if action.command == SystemCommandId::ApplyChanges
    ));
    assert!(!pinned.definition.steps().iter().any(|step| {
        matches!(
            &step.action,
            StepAction::SystemCommand(action)
                if action.command == SystemCommandId::CommitCandidate
        )
    }));
}

#[test]
fn pin_versions_are_stable_for_the_same_inputs() {
    let first = pin(AccessMode::ReadWrite, &ToolId::ALL);
    let second = pin(AccessMode::ReadWrite, &ToolId::ALL);
    assert_eq!(first.version, second.version);
    let read_only = pin(AccessMode::ReadOnly, &ToolId::ALL);
    assert_ne!(first.version, read_only.version);
    let fewer_tools = pin(AccessMode::ReadWrite, &[ToolId::List, ToolId::Read]);
    assert_ne!(first.version, fewer_tools.version);
    let other_env = pin_quick_task(
        AccessMode::ReadWrite,
        &ToolId::ALL,
        "Fix the bug.",
        crate::environments::EnvironmentId::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").expect("env"),
    )
    .expect("other env");
    assert_ne!(first.version, other_env.version);
    let other_instructions = pin_quick_task(
        AccessMode::ReadWrite,
        &ToolId::ALL,
        "Write tests.",
        test_environment_id(),
    )
    .expect("other instructions");
    assert_ne!(first.version, other_instructions.version);
}

pub(crate) fn pin_quick_task(
    access: AccessMode,
    tools: &[ToolId],
    instructions: &str,
    environment: EnvironmentId,
) -> Result<PinnedWorkflowDefinition, DefinitionError> {
    pin_quick_task_with_context(access, tools, instructions, environment, Vec::new())
}
