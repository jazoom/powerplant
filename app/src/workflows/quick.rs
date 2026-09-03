use crate::agents::{AccessMode, ToolId};
use crate::environments::seeds::ALPINE_GIT_V1;
use crate::environments::{EnvironmentCatalogue, EnvironmentId};

use super::commands::SystemCommandId;
use super::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, ArtefactKind, ArtefactSource, CandidateAuthority,
    DefinitionError, HumanGateStep, InputKey, OutputKey, OutputKind, PinnedWorkflowDefinition,
    RequiredInput, RequiredOutput, RoleDefinition, RoleKey, StepAction, StepDefinition,
    StepEnvironment, StepKey, SystemCommandStep, WorkflowDefinition, candidate_revision_output,
    initial_candidate_input,
};
use super::resolve::ResolveEnvironmentError;

pub(crate) const QUICK_TASK_NAME: &str = "Quick task";
const ROLE_KEY: &str = "agent";
const AGENT_STEP_KEY: &str = "work";
const GATE_STEP_KEY: &str = "gate";
const COMMIT_STEP_KEY: &str = "commit";
const DECISION_OUTPUT_KEY: &str = "decision";
const COMMITTED_OUTPUT_KEY: &str = "committed-candidate";

pub(crate) fn pin_quick_task(
    access: AccessMode,
    tools: &[ToolId],
    instructions: &str,
    environment: EnvironmentId,
) -> Result<PinnedWorkflowDefinition, DefinitionError> {
    let role = RoleDefinition::new(
        RoleKey::parse(ROLE_KEY).expect("quick task role"),
        "Agent".to_owned(),
        String::new(),
        instructions.to_owned(),
    )?;
    let work = agent_step(access, tools)?;
    let mut steps = vec![work];
    if access.is_writable() {
        steps.push(gate_step());
        steps.push(commit_step());
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

fn agent_step(access: AccessMode, tools: &[ToolId]) -> Result<StepDefinition, DefinitionError> {
    let (candidate_authority, required_outputs) = if access.is_writable() {
        (
            CandidateAuthority::Edit,
            vec![assistant_output(), candidate_revision_output()],
        )
    } else {
        (CandidateAuthority::ReadOnly, vec![assistant_output()])
    };
    Ok(StepDefinition {
        key: StepKey::parse(AGENT_STEP_KEY).expect("quick task step"),
        name: "Work on task".to_owned(),
        inputs: vec![initial_candidate_input()],
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse(ROLE_KEY).expect("quick task role"),
            environment: StepEnvironment::WorkflowDefault,
            candidate_authority,
            authority: AgentAuthority::new(tools.to_vec(), Vec::new())?,
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
mod tests;
