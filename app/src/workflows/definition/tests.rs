use super::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, DefinitionError, GuestDirectoryAccess, OutputKey,
    OutputKind, RequiredOutput, RoleDefinition, RoleKey, StepAction, StepDefinition, StepKey,
    SuccessTransition, SystemCommandId, SystemCommandStep, WorkflowDefinition,
};
use crate::agents::{AccessMode, ToolId};

fn role() -> RoleDefinition {
    RoleDefinition::new(
        RoleKey::parse("agent").expect("role"),
        "Maintainer".to_owned(),
        String::new(),
        "Keep public interfaces stable.".to_owned(),
    )
    .expect("role")
}

fn authority() -> AgentAuthority {
    AgentAuthority::new(
        vec![ToolId::List],
        vec![GuestDirectoryAccess {
            alias: "project".to_owned(),
            access: AccessMode::ReadWrite,
        }],
    )
    .expect("authority")
}

fn agent_step(key: &str, next: SuccessTransition) -> StepDefinition {
    StepDefinition {
        key: StepKey::parse(key).expect("step"),
        name: "Reply".to_owned(),
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse("agent").expect("role"),
            authority: authority(),
            required_outputs: vec![RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                kind: OutputKind::AssistantReply,
            }],
        }),
        on_success: next,
    }
}

fn command_step(key: &str, next: SuccessTransition) -> StepDefinition {
    StepDefinition {
        key: StepKey::parse(key).expect("step"),
        name: "Status".to_owned(),
        action: StepAction::SystemCommand(SystemCommandStep {
            command: SystemCommandId::RepositoryStatus,
            required_outputs: Vec::new(),
        }),
        on_success: next,
    }
}

fn one_agent() -> WorkflowDefinition {
    WorkflowDefinition::from_parts(
        "Maintainer".to_owned(),
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![agent_step("reply", SuccessTransition::CompleteRun)],
    )
    .expect("definition")
}

#[test]
fn duplicate_role_keys_are_rejected() {
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        vec![role(), role()],
        StepKey::parse("reply").expect("first"),
        vec![agent_step("reply", SuccessTransition::CompleteRun)],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::DuplicateRole));
}

#[test]
fn duplicate_step_keys_are_rejected() {
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![
            agent_step(
                "reply",
                SuccessTransition::Next(StepKey::parse("reply").expect("next")),
            ),
            agent_step("reply", SuccessTransition::CompleteRun),
        ],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::DuplicateStep));
}

#[test]
fn unknown_roles_are_rejected() {
    let mut step = agent_step("reply", SuccessTransition::CompleteRun);
    if let StepAction::Agent(action) = &mut step.action {
        action.role = RoleKey::parse("reviewer").expect("role");
    }
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![step],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::UnknownRole));
}

#[test]
fn unused_roles_are_rejected() {
    let extra = RoleDefinition::new(
        RoleKey::parse("reviewer").expect("role"),
        "Reviewer".to_owned(),
        String::new(),
        String::new(),
    )
    .expect("role");
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        vec![role(), extra],
        StepKey::parse("reply").expect("first"),
        vec![agent_step("reply", SuccessTransition::CompleteRun)],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::UnusedRole));
}

#[test]
fn unknown_successors_are_rejected() {
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![agent_step(
            "reply",
            SuccessTransition::Next(StepKey::parse("missing").expect("next")),
        )],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::UnknownStep));
}

#[test]
fn cycles_are_rejected() {
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        vec![role()],
        StepKey::parse("one").expect("first"),
        vec![
            agent_step(
                "one",
                SuccessTransition::Next(StepKey::parse("two").expect("two")),
            ),
            command_step(
                "two",
                SuccessTransition::Next(StepKey::parse("one").expect("one")),
            ),
        ],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::Cycle));
}

#[test]
fn joins_are_rejected() {
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        vec![role()],
        StepKey::parse("one").expect("first"),
        vec![
            agent_step(
                "one",
                SuccessTransition::Next(StepKey::parse("end").expect("end")),
            ),
            command_step(
                "two",
                SuccessTransition::Next(StepKey::parse("end").expect("end")),
            ),
            command_step("end", SuccessTransition::CompleteRun),
        ],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::Join));
}

#[test]
fn extra_sources_are_rejected_as_branches() {
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        vec![role()],
        StepKey::parse("one").expect("first"),
        vec![
            agent_step("one", SuccessTransition::CompleteRun),
            command_step("two", SuccessTransition::CompleteRun),
        ],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::Unreachable));
}

#[test]
fn unreachable_steps_are_rejected() {
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        vec![role()],
        StepKey::parse("one").expect("first"),
        vec![
            agent_step("one", SuccessTransition::CompleteRun),
            command_step(
                "two",
                SuccessTransition::Next(StepKey::parse("one").expect("one")),
            ),
        ],
    )
    .err();
    assert!(matches!(
        error,
        Some(DefinitionError::Join | DefinitionError::Branch | DefinitionError::Unreachable)
    ));
}

#[test]
fn arbitrary_command_values_are_rejected() {
    let json = r#"{
        "format-version": 1,
        "name": "Status",
        "roles": [],
        "first-step": "status",
        "steps": [
            {
                "key": "status",
                "name": "Status",
                "action": {
                    "type": "system-command",
                    "command": "rm -rf /",
                    "required-outputs": []
                },
                "on-success": { "type": "complete-run" }
            }
        ]
    }"#;
    assert_eq!(
        WorkflowDefinition::from_file_bytes(json.as_bytes()).err(),
        Some(DefinitionError::Command)
    );
}

#[test]
fn command_outputs_that_the_command_cannot_produce_are_rejected() {
    let mut step = command_step("status", SuccessTransition::CompleteRun);
    if let StepAction::SystemCommand(action) = &mut step.action {
        action.required_outputs.push(RequiredOutput {
            key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
            kind: OutputKind::AssistantReply,
        });
    }
    let error = WorkflowDefinition::from_parts(
        "Status".to_owned(),
        Vec::new(),
        StepKey::parse("status").expect("first"),
        vec![step],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::UnsupportedOutput));
}

#[test]
fn duplicate_output_keys_are_rejected() {
    let mut step = agent_step("reply", SuccessTransition::CompleteRun);
    if let StepAction::Agent(action) = &mut step.action {
        action.required_outputs.push(RequiredOutput {
            key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
            kind: OutputKind::AssistantReply,
        });
    }
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![step],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::DuplicateOutput));
}

#[test]
fn content_versions_round_trip_both_action_kinds() {
    let agent = one_agent();
    let command = WorkflowDefinition::from_parts(
        "Status".to_owned(),
        Vec::new(),
        StepKey::parse("status").expect("first"),
        vec![command_step("status", SuccessTransition::CompleteRun)],
    )
    .expect("command");
    let agent_bytes = serde_json::to_vec(&agent.to_file()).expect("json");
    let command_bytes = serde_json::to_vec(&command.to_file()).expect("json");
    let agent_again = WorkflowDefinition::from_file_bytes(&agent_bytes).expect("agent");
    let command_again = WorkflowDefinition::from_file_bytes(&command_bytes).expect("command");
    assert_eq!(agent.version(), agent_again.version());
    assert_eq!(command.version(), command_again.version());
    assert_ne!(agent.version(), command.version());
}

#[test]
fn a_definition_change_creates_a_different_content_version() {
    let original = one_agent();
    let changed = WorkflowDefinition::from_parts(
        "Changed".to_owned(),
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![agent_step("reply", SuccessTransition::CompleteRun)],
    )
    .expect("changed");
    assert_ne!(original.version(), changed.version());
}

#[test]
fn pretty_printed_bytes_do_not_change_the_content_version() {
    let definition = one_agent();
    let compact = serde_json::to_vec(&definition.to_file()).expect("compact");
    let pretty = serde_json::to_vec_pretty(&definition.to_file()).expect("pretty");
    assert_ne!(compact, pretty);
    assert_eq!(
        WorkflowDefinition::from_file_bytes(&pretty)
            .expect("pretty")
            .version(),
        definition.version()
    );
}
