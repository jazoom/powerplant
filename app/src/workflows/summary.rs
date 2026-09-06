use super::commands::SystemCommandId;
use super::definition::{CandidateAuthority, StepAction, WorkflowDefinition};

pub(crate) fn required_inputs(definition: &WorkflowDefinition) -> &'static str {
    if definition.launch_input_sources().is_empty() {
        "Task brief · Target project"
    } else {
        "Task brief · Target project · Saved plan"
    }
}

pub(crate) struct ProcessPhase {
    pub(crate) position: usize,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) purpose: String,
    pub(crate) effects: String,
    pub(crate) context: String,
    pub(crate) approval: String,
}

pub(crate) enum ProcessAction {
    Model(CandidateAuthority),
    Command(SystemCommandId),
    Approval,
    Invalid,
}

impl ProcessPhase {
    pub(crate) fn new(position: usize, name: String, action: ProcessAction) -> Self {
        let (kind, purpose, effects, context) = match action {
            ProcessAction::Model(access) => (
                "Model phase",
                match access {
                    CandidateAuthority::ReadOnly => {
                        "A model inspects the candidate and produces the declared outputs."
                            .to_owned()
                    }
                    CandidateAuthority::Edit => {
                        "A model works on the candidate and proposes changes.".to_owned()
                    }
                },
                match access {
                    CandidateAuthority::ReadOnly => {
                        "Reads the candidate. Does not change project files."
                    }
                    CandidateAuthority::Edit => {
                        "Can edit an isolated candidate. Does not change the project yet."
                    }
                },
                "Each attempt starts with fresh model context: the brief, declared artefacts and authorised root project instructions. Conversation and earlier worker transcripts stay excluded.",
            ),
            ProcessAction::Command(command) => (
                "System action",
                format!("Power Plant runs {}.", command.label()),
                match command {
                    SystemCommandId::RepositoryStatus => {
                        "Reads repository status. Does not change project files."
                    }
                    SystemCommandId::CommitCandidate => command.consequence(),
                },
                "The current candidate and validated artefacts continue to the next phase. No model request occurs.",
            ),
            ProcessAction::Approval => (
                "Approval stop",
                "A person reviews the exact candidate and records a decision.".to_owned(),
                "Changes no project files by itself.",
                "A safe pause. The candidate remains available for inspection.",
            ),
            ProcessAction::Invalid => (
                "Incomplete phase",
                "This phase needs a valid action.".to_owned(),
                "Unknown until the action is valid.",
                "Unknown until the action is valid.",
            ),
        };
        Self {
            position,
            name,
            kind: kind.to_owned(),
            purpose,
            effects: effects.to_owned(),
            context: context.to_owned(),
            approval: if matches!(action, ProcessAction::Approval) {
                "Approval stop. The exact candidate needs a human decision."
            } else {
                "No approval stop."
            }
            .to_owned(),
        }
    }
}

pub(crate) fn revision_summary(human: bool, target: &str, attempt_limit: &str) -> String {
    let prefix = if human {
        "Approval stop. Request changes returns to"
    } else {
        "Review route. A changes-requested verdict returns to"
    };
    format!("{prefix} {target}. Maximum {attempt_limit} attempts, including the first attempt.")
}

pub(crate) fn process_overview(definition: &WorkflowDefinition) -> Vec<ProcessPhase> {
    definition
        .steps()
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let action = match &step.action {
                StepAction::Agent(agent) => ProcessAction::Model(agent.candidate_authority),
                StepAction::SystemCommand(command) => ProcessAction::Command(command.command),
                StepAction::HumanGate(_) => ProcessAction::Approval,
            };
            let mut phase = ProcessPhase::new(index + 1, step.name.clone(), action);
            phase.approval = match &step.action {
                StepAction::HumanGate(action) => match &action.revision {
                    Some(policy) => format_revision(
                        true,
                        definition,
                        &policy.revision_target,
                        policy.attempt_limit,
                    ),
                    None => "Approval stop. The exact candidate needs a human decision.".to_owned(),
                },
                _ => match &step.review {
                    Some(policy) => format_revision(
                        false,
                        definition,
                        &policy.revision_target,
                        policy.attempt_limit,
                    ),
                    None => "No approval stop.".to_owned(),
                },
            };
            phase
        })
        .collect()
}

fn format_revision(
    human: bool,
    definition: &WorkflowDefinition,
    target: &super::definition::StepKey,
    attempt_limit: u8,
) -> String {
    let target = definition
        .step(target)
        .map(|step| step.name.as_str())
        .unwrap_or("the earlier implementation");
    revision_summary(human, target, &attempt_limit.to_string())
}

pub(crate) fn process_summary(definition: &WorkflowDefinition) -> String {
    definition
        .steps()
        .iter()
        .map(|step| {
            let action = match &step.action {
                StepAction::Agent(_) => "model phase",
                StepAction::SystemCommand(_) => "system action",
                StepAction::HumanGate(_) => "human approval",
            };
            format!("{} ({action})", step.name)
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

pub(crate) fn code_effects(definition: &WorkflowDefinition) -> String {
    let edits_candidate = definition
        .steps()
        .iter()
        .any(|step| step.writes_primary_source());
    let commits = definition.steps().iter().any(|step| {
        matches!(
            &step.action,
            StepAction::SystemCommand(command) if command.command == SystemCommandId::CommitCandidate
        )
    });

    match (edits_candidate, commits) {
        (false, false) => "Read-only. Does not change project files.".to_owned(),
        (true, false) => {
            "Can propose candidate changes. Does not commit project changes.".to_owned()
        }
        (true, true) => {
            "Can propose and commit candidate changes after the commit step's required assurance."
                .to_owned()
        }
        (false, true) => "Can commit a candidate after its required assurance.".to_owned(),
    }
}

pub(crate) fn approval_stops(definition: &WorkflowDefinition) -> String {
    let human_stops: Vec<_> = definition
        .steps()
        .iter()
        .filter_map(|step| {
            let StepAction::HumanGate(action) = &step.action else {
                return None;
            };
            Some(match &action.revision {
                Some(policy) => format!(
                    "{} with request changes to {}",
                    step.name,
                    definition
                        .step(&policy.revision_target)
                        .map(|target| target.name.as_str())
                        .unwrap_or("the earlier implementation")
                ),
                None => step.name.to_owned(),
            })
        })
        .collect();
    if !human_stops.is_empty() {
        format!("Human approval at {}", human_stops.join(", "))
    } else {
        match definition.commit_policy() {
            crate::workflows::definition::CommitPolicy::AutomaticAfterReview => {
                "Automatic commit after approved review".to_owned()
            }
            _ => "No approval stop".to_owned(),
        }
    }
}
