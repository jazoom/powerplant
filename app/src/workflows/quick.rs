use crate::agents::{AccessMode, ToolId};
use crate::environments::seeds::ALPINE_GIT_V1;
use crate::environments::{EnvironmentCatalogue, EnvironmentId};

use super::commands::SystemCommandId;
use super::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, ArtefactKind, ArtefactSource, CandidateAuthority,
    DefinitionError, GuestDirectoryAccess, HumanGateStep, HumanRevisionPolicy, InputKey, OutputKey,
    OutputKind, PinnedWorkflowDefinition, RequiredInput, RequiredOutput, RoleDefinition, RoleKey,
    StepAction, StepDefinition, StepEnvironment, StepKey, SystemCommandStep, WorkflowDefinition,
    candidate_revision_output, initial_candidate_input,
};
use super::resolve::ResolveEnvironmentError;

pub(crate) const QUICK_TASK_NAME: &str = "Quick task";
pub(crate) const HOST_UNCHANGED: &str =
    "The host project is unchanged. Review the candidate before you apply it.";
const ROLE_KEY: &str = "agent";
const AGENT_STEP_KEY: &str = "work";
const GATE_STEP_KEY: &str = "gate";
const COMMIT_STEP_KEY: &str = "commit";
const DECISION_OUTPUT_KEY: &str = "decision";
const COMMITTED_OUTPUT_KEY: &str = "committed-candidate";

pub(crate) fn pin_quick_task_with_context(
    access: AccessMode,
    tools: &[ToolId],
    instructions: &str,
    environment: EnvironmentId,
    secondary: Vec<GuestDirectoryAccess>,
) -> Result<PinnedWorkflowDefinition, DefinitionError> {
    let role = RoleDefinition::new(
        RoleKey::parse(ROLE_KEY).expect("quick task role"),
        "Agent".to_owned(),
        String::new(),
        instructions.to_owned(),
    )?;
    let work = agent_step(access, tools, secondary)?;
    let mut steps = vec![work];
    if access.is_writable() {
        steps.push(gate_step());
        steps.push(commit_step());
    }
    let definition =
        WorkflowDefinition::from_parts(QUICK_TASK_NAME.to_owned(), environment, vec![role], steps)?;
    Ok(PinnedWorkflowDefinition::pin(None, definition))
}

pub(crate) fn pin_project_free_quick_task_with_directories(
    tools: &[ToolId],
    instructions: &str,
    environment: EnvironmentId,
    directories: Vec<GuestDirectoryAccess>,
    reviewed: bool,
) -> Result<PinnedWorkflowDefinition, DefinitionError> {
    let role = RoleDefinition::new(
        RoleKey::parse(ROLE_KEY).expect("quick task role"),
        "Agent".to_owned(),
        String::new(),
        instructions.to_owned(),
    )?;
    let work = StepDefinition {
        key: StepKey::parse(AGENT_STEP_KEY).expect("quick task step"),
        name: if reviewed {
            "Prepare changes"
        } else {
            "Use tools"
        }
        .to_owned(),
        inputs: reviewed
            .then(|| RequiredInput {
                key: InputKey::parse("candidate").expect("quick candidate"),
                kind: ArtefactKind::CandidateRevision,
                source: ArtefactSource::RunCurrentCandidate,
            })
            .into_iter()
            .collect(),
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse(ROLE_KEY).expect("quick task role"),
            environment: StepEnvironment::WorkflowDefault,
            candidate_authority: if reviewed {
                CandidateAuthority::Edit
            } else {
                CandidateAuthority::ReadOnly
            },
            authority: AgentAuthority::new(tools.to_vec(), directories)?,
            settings: super::definition::ModelStepSettings::SameAsRunDefaults,
            required_outputs: if reviewed {
                vec![assistant_output(), candidate_revision_output()]
            } else {
                vec![assistant_output()]
            },
        }),
        review: None,
    };
    let mut steps = vec![work];
    if reviewed {
        // Ordinary reviewed-directory Quick tasks share the Git-backed
        // revision route: the new attempt binds the rejected candidate and
        // the original diff base through the engine reservation.
        steps.push(gate_step());
        steps.push(apply_step());
    }
    let definition =
        WorkflowDefinition::from_parts(QUICK_TASK_NAME.to_owned(), environment, vec![role], steps)?;
    Ok(PinnedWorkflowDefinition::pin(None, definition))
}

pub(crate) fn alpine_git_id(
    catalogue: &EnvironmentCatalogue,
) -> Result<EnvironmentId, ResolveEnvironmentError> {
    catalogue
        .seed_id(ALPINE_GIT_V1)
        .ok_or(ResolveEnvironmentError::Missing)
}

fn agent_step(
    access: AccessMode,
    tools: &[ToolId],
    secondary: Vec<GuestDirectoryAccess>,
) -> Result<StepDefinition, DefinitionError> {
    let (candidate_authority, required_outputs) = if access.is_writable() {
        (
            CandidateAuthority::Edit,
            vec![assistant_output(), candidate_revision_output()],
        )
    } else {
        (CandidateAuthority::ReadOnly, vec![assistant_output()])
    };
    let candidate_input = if access.is_writable() {
        RequiredInput {
            key: InputKey::parse("candidate").expect("quick candidate"),
            kind: ArtefactKind::CandidateRevision,
            source: ArtefactSource::RunCurrentCandidate,
        }
    } else {
        initial_candidate_input()
    };
    Ok(StepDefinition {
        key: StepKey::parse(AGENT_STEP_KEY).expect("quick task step"),
        name: "Work on task".to_owned(),
        inputs: vec![candidate_input],
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse(ROLE_KEY).expect("quick task role"),
            environment: StepEnvironment::WorkflowDefault,
            candidate_authority,
            authority: AgentAuthority::new(tools.to_vec(), secondary)?,
            settings: super::definition::ModelStepSettings::SameAsRunDefaults,
            required_outputs,
        }),
        review: None,
    })
}

pub(super) fn is_expected_gate_step(step: &StepDefinition) -> bool {
    step.key.as_str() == GATE_STEP_KEY
        && step.inputs.len() == 1
        && step.inputs[0].key.as_str() == "candidate"
        && step.inputs[0].kind == ArtefactKind::CandidateRevision
        && matches!(
            &step.inputs[0].source,
            ArtefactSource::StepOutput { step, output }
                if step.as_str() == AGENT_STEP_KEY && output.as_str() == "candidate"
        )
        && matches!(
            &step.action,
            StepAction::HumanGate(action)
                if action.required_output.key.as_str() == DECISION_OUTPUT_KEY
                    && action.required_output.kind == OutputKind::HumanDecision
        )
        && step.review.is_none()
}

fn gate_step() -> StepDefinition {
    StepDefinition {
        key: StepKey::parse(GATE_STEP_KEY).expect("quick task gate"),
        name: "Review changes".to_owned(),
        inputs: vec![step_output(
            "candidate",
            ArtefactKind::CandidateRevision,
            AGENT_STEP_KEY,
            "candidate",
        )],
        action: StepAction::HumanGate(HumanGateStep {
            required_output: RequiredOutput {
                key: OutputKey::parse(DECISION_OUTPUT_KEY).expect("quick task decision"),
                kind: OutputKind::HumanDecision,
            },
            revision: Some(HumanRevisionPolicy {
                revision_target: StepKey::parse(AGENT_STEP_KEY).expect("quick task work"),
                attempt_limit: 3,
            }),
        }),
        review: None,
    }
}

fn apply_step() -> StepDefinition {
    StepDefinition {
        key: StepKey::parse("apply").expect("quick task apply"),
        name: "Apply changes".to_owned(),
        inputs: vec![
            step_output(
                "candidate",
                ArtefactKind::CandidateRevision,
                AGENT_STEP_KEY,
                "candidate",
            ),
            step_output(
                "decision",
                ArtefactKind::HumanDecision,
                GATE_STEP_KEY,
                DECISION_OUTPUT_KEY,
            ),
        ],
        action: StepAction::SystemCommand(SystemCommandStep {
            command: SystemCommandId::ApplyChanges,
            environment: StepEnvironment::WorkflowDefault,
            required_outputs: vec![RequiredOutput {
                key: OutputKey::parse("applied-candidate").expect("quick task applied"),
                kind: OutputKind::CandidateRevision,
            }],
        }),
        review: None,
    }
}

fn commit_step() -> StepDefinition {
    StepDefinition {
        key: StepKey::parse(COMMIT_STEP_KEY).expect("quick task commit"),
        name: "Commit".to_owned(),
        inputs: vec![
            step_output(
                "candidate",
                ArtefactKind::CandidateRevision,
                AGENT_STEP_KEY,
                "candidate",
            ),
            step_output(
                "decision",
                ArtefactKind::HumanDecision,
                GATE_STEP_KEY,
                DECISION_OUTPUT_KEY,
            ),
        ],
        action: StepAction::SystemCommand(SystemCommandStep {
            command: SystemCommandId::CommitCandidate,
            environment: StepEnvironment::WorkflowDefault,
            required_outputs: vec![RequiredOutput {
                key: OutputKey::parse(COMMITTED_OUTPUT_KEY).expect("quick task committed"),
                kind: OutputKind::CandidateRevision,
            }],
        }),
        review: None,
    }
}

fn assistant_output() -> RequiredOutput {
    RequiredOutput {
        key: OutputKey::parse(ASSISTANT_REPLY).expect("assistant output"),
        kind: OutputKind::AssistantReply,
    }
}

fn step_output(key: &str, kind: ArtefactKind, step: &str, output: &str) -> RequiredInput {
    RequiredInput {
        key: InputKey::parse(key).expect("quick task input"),
        kind,
        source: ArtefactSource::StepOutput {
            step: StepKey::parse(step).expect("quick task source step"),
            output: OutputKey::parse(output).expect("quick task source output"),
        },
    }
}

#[cfg(test)]
pub(crate) mod tests;
