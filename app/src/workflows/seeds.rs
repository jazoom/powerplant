use crate::agents::{AccessMode, ToolId};
use crate::environments::EnvironmentId;

use super::commands::SystemCommandId;
use super::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, ArtefactKind, ArtefactSource, GuestDirectoryAccess,
    InputKey, OutputKey, OutputKind, RequiredInput, RequiredOutput, RoleDefinition, RoleKey,
    StepAction, StepDefinition, StepEnvironment, StepKey, SuccessTransition, SystemCommandStep,
    WorkflowDefinition, candidate_revision_output, initial_candidate_input,
};

pub(crate) const ONE_AGENT_V1: &str = "one-agent-v1";
pub(crate) const SEQUENTIAL_TEAM_V1: &str = "sequential-team-v1";

const SEED_KEY_BYTES: usize = 32;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct SeedKey(String);

#[derive(Clone, Debug)]
pub(crate) struct WorkflowSeed {
    pub(crate) key: SeedKey,
    pub(crate) definition: WorkflowDefinition,
}

impl SeedKey {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let key = value.trim();
        if key.is_empty() || key.len() > SEED_KEY_BYTES {
            return None;
        }
        let mut characters = key.chars();
        let first = characters.next()?;
        if !first.is_ascii_alphabetic() {
            return None;
        }
        if !characters.all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        }) {
            return None;
        }
        Some(Self(key.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) fn production_seeds(default_environment: EnvironmentId) -> Vec<WorkflowSeed> {
    vec![
        WorkflowSeed {
            key: SeedKey::parse(ONE_AGENT_V1).expect("one-agent seed key"),
            definition: one_agent_definition(default_environment),
        },
        WorkflowSeed {
            key: SeedKey::parse(SEQUENTIAL_TEAM_V1).expect("sequential-team seed key"),
            definition: sequential_team_definition(default_environment),
        },
    ]
}

pub(crate) fn one_agent_definition(default_environment: EnvironmentId) -> WorkflowDefinition {
    let role = RoleDefinition::new(
        RoleKey::parse("coding-agent").expect("role"),
        "Coding agent".to_owned(),
        String::new(),
        String::new(),
    )
    .expect("role");
    let authority = AgentAuthority::new(
        ToolId::ALL.to_vec(),
        vec![GuestDirectoryAccess {
            alias: "project".to_owned(),
            access: AccessMode::ReadWrite,
        }],
    )
    .expect("authority");
    let step = StepDefinition {
        key: StepKey::parse("work-on-task").expect("step"),
        name: "Work on task".to_owned(),
        inputs: vec![initial_candidate_input()],
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse("coding-agent").expect("role"),
            environment: StepEnvironment::WorkflowDefault,
            authority,
            required_outputs: vec![
                RequiredOutput {
                    key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                    kind: OutputKind::AssistantReply,
                },
                candidate_revision_output(),
            ],
        }),
        on_success: SuccessTransition::CompleteRun,
    };
    WorkflowDefinition::from_parts(
        "One agent".to_owned(),
        default_environment,
        vec![role],
        StepKey::parse("work-on-task").expect("first"),
        vec![step],
    )
    .expect("one agent definition")
}

pub(crate) fn sequential_team_definition(default_environment: EnvironmentId) -> WorkflowDefinition {
    let planner = RoleDefinition::new(
        RoleKey::parse("planner").expect("role"),
        "Planner".to_owned(),
        "Analyses the task, repository, constraints, and implementation sequence.".to_owned(),
        "Submit a plan that describes the implementation sequence. Do not implement the change."
            .to_owned(),
    )
    .expect("planner");
    let implementer = RoleDefinition::new(
        RoleKey::parse("implementer").expect("role"),
        "Implementer".to_owned(),
        "Applies the accepted plan and produces the complete candidate.".to_owned(),
        "Apply the accepted plan. Submit the complete candidate.".to_owned(),
    )
    .expect("implementer");
    let reviewer = RoleDefinition::new(
        RoleKey::parse("reviewer").expect("role"),
        "Reviewer".to_owned(),
        "Checks correctness, security, regressions, scope, and plan compliance.".to_owned(),
        "Assess the accepted plan and the candidate. Submit a review report for this exact candidate."
            .to_owned(),
    )
    .expect("reviewer");
    let read_authority = AgentAuthority::new(
        vec![ToolId::List, ToolId::Read, ToolId::Run],
        vec![GuestDirectoryAccess {
            alias: "project".to_owned(),
            access: AccessMode::ReadOnly,
        }],
    )
    .expect("read authority");
    let write_authority = AgentAuthority::new(
        ToolId::ALL.to_vec(),
        vec![GuestDirectoryAccess {
            alias: "project".to_owned(),
            access: AccessMode::ReadWrite,
        }],
    )
    .expect("write authority");
    let planner_step = StepDefinition {
        key: StepKey::parse("planner").expect("step"),
        name: "Planner".to_owned(),
        inputs: vec![initial_candidate_input()],
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse("planner").expect("role"),
            environment: StepEnvironment::WorkflowDefault,
            authority: read_authority.clone(),
            required_outputs: vec![
                assistant_output(),
                RequiredOutput {
                    key: OutputKey::parse("plan").expect("output"),
                    kind: OutputKind::Plan,
                },
            ],
        }),
        on_success: SuccessTransition::Next(StepKey::parse("implementer").expect("next")),
    };
    let implementer_step = StepDefinition {
        key: StepKey::parse("implementer").expect("step"),
        name: "Implementer".to_owned(),
        inputs: vec![
            initial_candidate_input(),
            RequiredInput {
                key: InputKey::parse("plan").expect("input"),
                kind: ArtefactKind::Plan,
                source: ArtefactSource::StepOutput {
                    step: StepKey::parse("planner").expect("step"),
                    output: OutputKey::parse("plan").expect("output"),
                },
            },
        ],
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse("implementer").expect("role"),
            environment: StepEnvironment::WorkflowDefault,
            authority: write_authority,
            required_outputs: vec![assistant_output(), candidate_revision_output()],
        }),
        on_success: SuccessTransition::Next(StepKey::parse("reviewer").expect("next")),
    };
    let reviewer_step = StepDefinition {
        key: StepKey::parse("reviewer").expect("step"),
        name: "Reviewer".to_owned(),
        inputs: vec![
            RequiredInput {
                key: InputKey::parse("candidate").expect("input"),
                kind: ArtefactKind::CandidateRevision,
                source: ArtefactSource::StepOutput {
                    step: StepKey::parse("implementer").expect("step"),
                    output: OutputKey::parse("candidate").expect("output"),
                },
            },
            RequiredInput {
                key: InputKey::parse("plan").expect("input"),
                kind: ArtefactKind::Plan,
                source: ArtefactSource::StepOutput {
                    step: StepKey::parse("planner").expect("step"),
                    output: OutputKey::parse("plan").expect("output"),
                },
            },
        ],
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse("reviewer").expect("role"),
            environment: StepEnvironment::WorkflowDefault,
            authority: read_authority,
            required_outputs: vec![
                assistant_output(),
                RequiredOutput {
                    key: OutputKey::parse("review").expect("output"),
                    kind: OutputKind::ReviewReport,
                },
            ],
        }),
        on_success: SuccessTransition::Next(StepKey::parse("commit").expect("next")),
    };
    let commit_step = StepDefinition {
        key: StepKey::parse("commit").expect("step"),
        name: "Commit".to_owned(),
        inputs: vec![
            RequiredInput {
                key: InputKey::parse("candidate").expect("input"),
                kind: ArtefactKind::CandidateRevision,
                source: ArtefactSource::StepOutput {
                    step: StepKey::parse("implementer").expect("step"),
                    output: OutputKey::parse("candidate").expect("output"),
                },
            },
            RequiredInput {
                key: InputKey::parse("review").expect("input"),
                kind: ArtefactKind::ReviewReport,
                source: ArtefactSource::StepOutput {
                    step: StepKey::parse("reviewer").expect("step"),
                    output: OutputKey::parse("review").expect("output"),
                },
            },
        ],
        action: StepAction::SystemCommand(SystemCommandStep {
            command: SystemCommandId::CommitCandidate,
            environment: StepEnvironment::WorkflowDefault,
            required_outputs: vec![RequiredOutput {
                key: OutputKey::parse("committed-candidate").expect("output"),
                kind: OutputKind::CandidateRevision,
            }],
        }),
        on_success: SuccessTransition::CompleteRun,
    };
    WorkflowDefinition::from_parts(
        "Sequential team".to_owned(),
        default_environment,
        vec![planner, implementer, reviewer],
        StepKey::parse("planner").expect("first"),
        vec![planner_step, implementer_step, reviewer_step, commit_step],
    )
    .expect("sequential team definition")
}

fn assistant_output() -> RequiredOutput {
    RequiredOutput {
        key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
        kind: OutputKind::AssistantReply,
    }
}

#[cfg(test)]
mod tests;
