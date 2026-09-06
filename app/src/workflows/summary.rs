use super::commands::SystemCommandId;
use super::definition::{StepAction, WorkflowDefinition};

pub(crate) const REQUIRED_INPUTS: &str = "Task brief · Target project";

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
