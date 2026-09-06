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
        .filter(|step| matches!(step.action, StepAction::HumanGate(_)))
        .map(|step| step.name.as_str())
        .collect();
    if human_stops.is_empty() {
        "No approval stop".to_owned()
    } else {
        format!("Human approval at {}", human_stops.join(", "))
    }
}
