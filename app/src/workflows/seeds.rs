use crate::agents::ToolId;
use crate::environments::EnvironmentId;

use super::commands::SystemCommandId;
use super::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, ArtefactKind, ArtefactSource, CandidateAuthority,
    InputKey, OutputKey, OutputKind, RequiredInput, RequiredOutput, RoleDefinition, RoleKey,
    StepAction, StepDefinition, StepEnvironment, StepKey, SuccessTransition, SystemCommandStep,
    WorkflowDefinition, candidate_revision_output, initial_candidate_input,
};

pub(crate) const ONE_AGENT_V1: &str = "one-agent-v1";
pub(crate) const SEQUENTIAL_TEAM_V1: &str = "sequential-team-v1";
pub(crate) const READ_ONLY_REVIEW_V1: &str = "read-only-review-v1";
pub(crate) const REVIEW_WITH_FIXES_V1: &str = "review-with-fixes-v1";

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
    [
        (ONE_AGENT_V1, one_agent_definition(default_environment)),
        (
            SEQUENTIAL_TEAM_V1,
            sequential_team_definition(default_environment),
        ),
        (
            READ_ONLY_REVIEW_V1,
            read_only_review_definition(default_environment),
        ),
        (
            REVIEW_WITH_FIXES_V1,
            review_with_fixes_definition(default_environment),
        ),
    ]
    .into_iter()
    .map(|(key, definition)| WorkflowSeed {
        key: SeedKey::parse(key).expect("workflow seed key"),
        definition,
    })
    .collect()
}

pub(crate) fn one_agent_definition(default_environment: EnvironmentId) -> WorkflowDefinition {
    let role = role("coding-agent", "Coding agent", "", "");
    let step = agent_step(
        "work-on-task",
        "Work on task",
        "coding-agent",
        CandidateAuthority::Edit,
        ToolId::ALL.to_vec(),
        vec![initial_candidate_input()],
        vec![assistant_output(), candidate_revision_output()],
        SuccessTransition::CompleteRun,
    );
    definition(
        "One agent",
        default_environment,
        vec![role],
        "work-on-task",
        vec![step],
    )
}

pub(crate) fn sequential_team_definition(default_environment: EnvironmentId) -> WorkflowDefinition {
    let roles = vec![
        role(
            "planner",
            "Planner",
            "Analyses the task, repository, constraints, and implementation sequence.",
            "Submit a plan that describes the implementation sequence. Do not implement the change.",
        ),
        role(
            "implementer",
            "Implementer",
            "Applies the accepted plan and produces the complete candidate.",
            "Apply the accepted plan. Submit the complete candidate.",
        ),
        role(
            "reviewer",
            "Reviewer",
            "Checks correctness, security, regressions, scope, and plan compliance.",
            "Assess the accepted plan and the candidate. Submit a review report for this exact candidate.",
        ),
    ];
    let planner = agent_step(
        "planner",
        "Planner",
        "planner",
        CandidateAuthority::ReadOnly,
        review_tools(),
        vec![initial_candidate_input()],
        vec![assistant_output(), output("plan", OutputKind::Plan)],
        next("implementer"),
    );
    let implementer = agent_step(
        "implementer",
        "Implementer",
        "implementer",
        CandidateAuthority::Edit,
        ToolId::ALL.to_vec(),
        vec![
            initial_candidate_input(),
            input("plan", ArtefactKind::Plan, "planner", "plan"),
        ],
        vec![assistant_output(), candidate_revision_output()],
        next("reviewer"),
    );
    let reviewer = agent_step(
        "reviewer",
        "Reviewer",
        "reviewer",
        CandidateAuthority::ReadOnly,
        review_tools(),
        vec![
            input(
                "candidate",
                ArtefactKind::CandidateRevision,
                "implementer",
                "candidate",
            ),
            input("plan", ArtefactKind::Plan, "planner", "plan"),
        ],
        vec![assistant_output(), review_output()],
        next("commit"),
    );
    let commit = commit_step("implementer", "reviewer");
    definition(
        "Sequential team",
        default_environment,
        roles,
        "planner",
        vec![planner, implementer, reviewer, commit],
    )
}

pub(crate) fn read_only_review_definition(
    default_environment: EnvironmentId,
) -> WorkflowDefinition {
    let roles = vec![
        role(
            "implementer",
            "Implementer",
            "Implements the requested change.",
            "Implement the task and submit the complete candidate.",
        ),
        role(
            "reviewer",
            "Reviewer",
            "Checks correctness, security, regressions, and scope.",
            "Assess this exact candidate. Submit a structured review verdict.",
        ),
    ];
    let implementer = agent_step(
        "implementer",
        "Implementer",
        "implementer",
        CandidateAuthority::Edit,
        ToolId::ALL.to_vec(),
        vec![initial_candidate_input()],
        vec![assistant_output(), candidate_revision_output()],
        next("reviewer"),
    );
    let reviewer = agent_step(
        "reviewer",
        "Reviewer",
        "reviewer",
        CandidateAuthority::ReadOnly,
        review_tools(),
        vec![input(
            "candidate",
            ArtefactKind::CandidateRevision,
            "implementer",
            "candidate",
        )],
        vec![assistant_output(), review_output()],
        next("commit"),
    );
    definition(
        "Read-only review",
        default_environment,
        roles,
        "implementer",
        vec![
            implementer,
            reviewer,
            commit_step("implementer", "reviewer"),
        ],
    )
}

pub(crate) fn review_with_fixes_definition(
    default_environment: EnvironmentId,
) -> WorkflowDefinition {
    let roles = vec![
        role(
            "implementer",
            "Implementer",
            "Implements the requested change.",
            "Implement the task and submit the complete candidate.",
        ),
        role(
            "fixing-reviewer",
            "Fixing reviewer",
            "Reviews the candidate and fixes safe issues.",
            "Fix every safe issue that you find. Submit a structured verdict for your output candidate.",
        ),
        role(
            "independent-reviewer",
            "Independent reviewer",
            "Assesses the fixed candidate without trust in the prior verdict.",
            "Review the fixed candidate independently. Treat the prior review as context only.",
        ),
    ];
    let implementer = agent_step(
        "implementer",
        "Implementer",
        "implementer",
        CandidateAuthority::Edit,
        ToolId::ALL.to_vec(),
        vec![initial_candidate_input()],
        vec![assistant_output(), candidate_revision_output()],
        next("fixing-reviewer"),
    );
    let fixing = agent_step(
        "fixing-reviewer",
        "Fixing reviewer",
        "fixing-reviewer",
        CandidateAuthority::Edit,
        ToolId::ALL.to_vec(),
        vec![input(
            "candidate",
            ArtefactKind::CandidateRevision,
            "implementer",
            "candidate",
        )],
        vec![
            assistant_output(),
            candidate_revision_output(),
            review_output(),
        ],
        next("independent-reviewer"),
    );
    let independent = agent_step(
        "independent-reviewer",
        "Independent reviewer",
        "independent-reviewer",
        CandidateAuthority::ReadOnly,
        review_tools(),
        vec![
            input(
                "candidate",
                ArtefactKind::CandidateRevision,
                "fixing-reviewer",
                "candidate",
            ),
            input(
                "prior-review",
                ArtefactKind::ReviewReport,
                "fixing-reviewer",
                "review",
            ),
        ],
        vec![assistant_output(), review_output()],
        next("commit"),
    );
    definition(
        "Review with fixes",
        default_environment,
        roles,
        "implementer",
        vec![
            implementer,
            fixing,
            independent,
            commit_step("fixing-reviewer", "independent-reviewer"),
        ],
    )
}

fn definition(
    name: &str,
    environment: EnvironmentId,
    roles: Vec<RoleDefinition>,
    first: &str,
    steps: Vec<StepDefinition>,
) -> WorkflowDefinition {
    WorkflowDefinition::from_parts(
        name.to_owned(),
        environment,
        roles,
        StepKey::parse(first).expect("first step"),
        steps,
    )
    .expect("seed definition")
}

fn role(key: &str, name: &str, expertise: &str, prompt: &str) -> RoleDefinition {
    RoleDefinition::new(
        RoleKey::parse(key).expect("role"),
        name.to_owned(),
        expertise.to_owned(),
        prompt.to_owned(),
    )
    .expect("role definition")
}

#[allow(clippy::too_many_arguments)]
fn agent_step(
    key: &str,
    name: &str,
    role_key: &str,
    candidate_authority: CandidateAuthority,
    tools: Vec<ToolId>,
    inputs: Vec<RequiredInput>,
    outputs: Vec<RequiredOutput>,
    on_success: SuccessTransition,
) -> StepDefinition {
    StepDefinition {
        key: StepKey::parse(key).expect("step"),
        name: name.to_owned(),
        inputs,
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse(role_key).expect("role"),
            environment: StepEnvironment::WorkflowDefault,
            candidate_authority,
            authority: AgentAuthority::new(tools, Vec::new()).expect("authority"),
            required_outputs: outputs,
        }),
        on_success,
    }
}

fn commit_step(candidate_step: &str, review_step: &str) -> StepDefinition {
    StepDefinition {
        key: StepKey::parse("commit").expect("step"),
        name: "Commit".to_owned(),
        inputs: vec![
            input(
                "candidate",
                ArtefactKind::CandidateRevision,
                candidate_step,
                "candidate",
            ),
            input("review", ArtefactKind::ReviewReport, review_step, "review"),
        ],
        action: StepAction::SystemCommand(SystemCommandStep {
            command: SystemCommandId::CommitCandidate,
            environment: StepEnvironment::WorkflowDefault,
            required_outputs: vec![output("committed-candidate", OutputKind::CandidateRevision)],
        }),
        on_success: SuccessTransition::CompleteRun,
    }
}

fn input(key: &str, kind: ArtefactKind, step: &str, output_key: &str) -> RequiredInput {
    RequiredInput {
        key: InputKey::parse(key).expect("input"),
        kind,
        source: ArtefactSource::StepOutput {
            step: StepKey::parse(step).expect("source step"),
            output: OutputKey::parse(output_key).expect("source output"),
        },
    }
}

fn output(key: &str, kind: OutputKind) -> RequiredOutput {
    RequiredOutput {
        key: OutputKey::parse(key).expect("output"),
        kind,
    }
}

fn assistant_output() -> RequiredOutput {
    output(ASSISTANT_REPLY, OutputKind::AssistantReply)
}
fn review_output() -> RequiredOutput {
    output("review", OutputKind::ReviewReport)
}
fn review_tools() -> Vec<ToolId> {
    vec![ToolId::List, ToolId::Read, ToolId::Run]
}
fn next(step: &str) -> SuccessTransition {
    SuccessTransition::Next(StepKey::parse(step).expect("next"))
}

#[cfg(test)]
mod tests;
