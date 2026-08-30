use super::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, ArtefactKind, ArtefactSource, CandidateAuthority,
    DefinitionError, GuestDirectoryAccess, InputKey, OutputKey, OutputKind, RequiredInput,
    RequiredOutput, RoleDefinition, RoleKey, StepAction, StepDefinition, StepEnvironment, StepKey,
    SuccessTransition, SystemCommandId, SystemCommandStep, WorkflowDefinition,
    candidate_revision_output, initial_candidate_input, test_environment_id,
};
use crate::agents::{AccessMode, ToolId};
use crate::environments::EnvironmentId;

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
    AgentAuthority::new(vec![ToolId::List], Vec::new()).expect("authority")
}

fn agent_step(key: &str, next: SuccessTransition) -> StepDefinition {
    write_agent_step(
        key,
        next,
        ArtefactSource::RunInitialCandidate,
        vec![
            RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                kind: OutputKind::AssistantReply,
            },
            candidate_revision_output(),
        ],
    )
}

fn write_agent_step(
    key: &str,
    next: SuccessTransition,
    candidate: ArtefactSource,
    outputs: Vec<RequiredOutput>,
) -> StepDefinition {
    StepDefinition {
        key: StepKey::parse(key).expect("step"),
        name: "Reply".to_owned(),
        inputs: vec![RequiredInput {
            key: InputKey::parse("candidate").expect("input"),
            kind: ArtefactKind::CandidateRevision,
            source: candidate,
        }],
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse("agent").expect("role"),
            environment: StepEnvironment::WorkflowDefault,
            candidate_authority: CandidateAuthority::Edit,
            authority: authority(),
            required_outputs: outputs,
        }),
        on_success: next,
    }
}

fn command_step(key: &str, next: SuccessTransition) -> StepDefinition {
    StepDefinition {
        key: StepKey::parse(key).expect("step"),
        name: "Status".to_owned(),
        inputs: vec![initial_candidate_input()],
        action: StepAction::SystemCommand(SystemCommandStep {
            command: SystemCommandId::RepositoryStatus,
            environment: StepEnvironment::WorkflowDefault,
            required_outputs: Vec::new(),
        }),
        on_success: next,
    }
}

fn one_agent() -> WorkflowDefinition {
    WorkflowDefinition::from_parts(
        "Maintainer".to_owned(),
        test_environment_id(),
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
        test_environment_id(),
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
        test_environment_id(),
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
        test_environment_id(),
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
        test_environment_id(),
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
        test_environment_id(),
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
        test_environment_id(),
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
        test_environment_id(),
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
        test_environment_id(),
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
        test_environment_id(),
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
    let json = format!(
        r#"{{
        "format-version": 3,
        "name": "Status",
        "default-environment": "{}",
        "roles": [],
        "first-step": "status",
        "steps": [
            {{
                "key": "status",
                "name": "Status",
                "inputs": [{{
                    "key": "candidate",
                    "kind": "candidate-revision",
                    "source": {{ "source": "run-initial-candidate" }}
                }}],
                "action": {{
                    "type": "system-command",
                    "command": "rm -rf /",
                    "environment": {{ "source": "workflow-default" }},
                    "required-outputs": []
                }},
                "on-success": {{ "type": "complete-run" }}
            }}
        ]
    }}"#,
        test_environment_id().as_hex()
    );
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
        test_environment_id(),
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
        test_environment_id(),
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
        test_environment_id(),
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
        test_environment_id(),
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

#[test]
fn malformed_environment_identifiers_are_rejected_before_graph_checks() {
    let mut file = one_agent().to_file();
    file.default_environment = "not-an-id".to_owned();
    let bytes = serde_json::to_vec(&file).expect("json");
    assert_eq!(
        WorkflowDefinition::from_file_bytes(&bytes).err(),
        Some(DefinitionError::Environment)
    );
}

#[test]
fn an_override_equal_to_the_default_normalises_to_workflow_default() {
    let mut step = agent_step("reply", SuccessTransition::CompleteRun);
    if let StepAction::Agent(action) = &mut step.action {
        action.environment = StepEnvironment::Override {
            environment_id: test_environment_id(),
        };
    }
    let definition = WorkflowDefinition::from_parts(
        "Maintainer".to_owned(),
        test_environment_id(),
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![step],
    )
    .expect("definition");
    assert_eq!(
        definition.steps()[0].environment(),
        Some(StepEnvironment::WorkflowDefault)
    );
}

#[test]
fn environment_identifiers_change_the_content_version() {
    let original = one_agent();
    let other = EnvironmentId::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").expect("other");
    let changed = WorkflowDefinition::from_parts(
        "Maintainer".to_owned(),
        other,
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![agent_step("reply", SuccessTransition::CompleteRun)],
    )
    .expect("changed");
    assert_ne!(original.version(), changed.version());
}

#[test]
fn earlier_formats_are_rejected() {
    let json = serde_json::json!({
        "format-version": 0,
        "name": "Maintainer",
        "default-environment": test_environment_id().as_hex(),
        "roles": [{
            "key": "agent",
            "name": "Maintainer",
            "expertise": "",
            "prompt-defaults": ""
        }],
        "first-step": "reply",
        "steps": [{
            "key": "reply",
            "name": "Reply",
            "action": {
                "type": "agent",
                "role": "agent",
                "environment": { "source": "workflow-default" },
                "authority": {
                    "tools": ["list"],
                    "directories": [{
                        "alias": "project",
                        "access": "read-write"
                    }]
                },
                "required-outputs": [{
                    "key": "assistant-reply",
                    "kind": "assistant-reply"
                }]
            },
            "on-success": { "type": "complete-run" }
        }]
    });
    let bytes = serde_json::to_vec(&json).expect("bytes");
    assert_eq!(
        WorkflowDefinition::from_file_bytes(&bytes).err(),
        Some(DefinitionError::Format)
    );
}

fn from_step(step: &str, output: &str) -> ArtefactSource {
    ArtefactSource::StepOutput {
        step: StepKey::parse(step).expect("step"),
        output: OutputKey::parse(output).expect("output"),
    }
}

#[test]
fn unknown_step_outputs_are_rejected() {
    let later = write_agent_step(
        "reply",
        SuccessTransition::Next(StepKey::parse("status").expect("next")),
        ArtefactSource::RunInitialCandidate,
        vec![
            RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                kind: OutputKind::AssistantReply,
            },
            candidate_revision_output(),
        ],
    );
    let mut status = command_step("status", SuccessTransition::CompleteRun);
    status.inputs = vec![
        RequiredInput {
            key: InputKey::parse("candidate").expect("input"),
            kind: ArtefactKind::CandidateRevision,
            source: from_step("reply", "candidate"),
        },
        RequiredInput {
            key: InputKey::parse("plan").expect("input"),
            kind: ArtefactKind::Plan,
            source: from_step("reply", "plan"),
        },
    ];
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        test_environment_id(),
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![later, status],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::UnsupportedOutput));
}

#[test]
fn input_kind_mismatch_is_rejected() {
    let planner = write_agent_step(
        "plan",
        SuccessTransition::Next(StepKey::parse("review").expect("next")),
        ArtefactSource::RunInitialCandidate,
        vec![
            RequiredOutput {
                key: OutputKey::parse("plan").expect("output"),
                kind: OutputKind::Plan,
            },
            candidate_revision_output(),
        ],
    );
    let mut review = write_agent_step(
        "review",
        SuccessTransition::CompleteRun,
        from_step("plan", "candidate"),
        vec![
            RequiredOutput {
                key: OutputKey::parse("report").expect("output"),
                kind: OutputKind::ReviewReport,
            },
            candidate_revision_output(),
        ],
    );
    review.inputs.push(RequiredInput {
        key: InputKey::parse("plan").expect("input"),
        kind: ArtefactKind::ReviewReport,
        source: from_step("plan", "plan"),
    });
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        test_environment_id(),
        vec![role()],
        StepKey::parse("plan").expect("first"),
        vec![planner, review],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::InputKind));
}

#[test]
fn stale_candidate_sources_are_rejected() {
    let first = agent_step(
        "one",
        SuccessTransition::Next(StepKey::parse("two").expect("two")),
    );
    let second = write_agent_step(
        "two",
        SuccessTransition::CompleteRun,
        ArtefactSource::RunInitialCandidate,
        vec![
            RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                kind: OutputKind::AssistantReply,
            },
            candidate_revision_output(),
        ],
    );
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        test_environment_id(),
        vec![role()],
        StepKey::parse("one").expect("first"),
        vec![first, second],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::CandidateInput));
}

#[test]
fn sandbox_steps_without_candidate_inputs_are_rejected() {
    let mut step = agent_step("reply", SuccessTransition::CompleteRun);
    step.inputs.clear();
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        test_environment_id(),
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![step],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::CandidateInput));
}

#[test]
fn candidate_authority_rejects_conflicting_outputs() {
    let mut read_only = agent_step("reply", SuccessTransition::CompleteRun);
    let StepAction::Agent(action) = &mut read_only.action else {
        panic!("agent step");
    };
    action.candidate_authority = CandidateAuthority::ReadOnly;
    assert_eq!(
        WorkflowDefinition::from_parts(
            "Read-only".to_owned(),
            test_environment_id(),
            vec![role()],
            StepKey::parse("reply").expect("first"),
            vec![read_only],
        )
        .err(),
        Some(DefinitionError::CandidateOutput)
    );

    let mut duplicate_reviews = agent_step("reply", SuccessTransition::CompleteRun);
    let StepAction::Agent(action) = &mut duplicate_reviews.action else {
        panic!("agent step");
    };
    action.required_outputs.extend([
        RequiredOutput {
            key: OutputKey::parse("review-one").expect("output"),
            kind: OutputKind::ReviewReport,
        },
        RequiredOutput {
            key: OutputKey::parse("review-two").expect("output"),
            kind: OutputKind::ReviewReport,
        },
    ]);
    assert_eq!(
        WorkflowDefinition::from_parts(
            "Fixing review".to_owned(),
            test_environment_id(),
            vec![role()],
            StepKey::parse("reply").expect("first"),
            vec![duplicate_reviews],
        )
        .err(),
        Some(DefinitionError::UnsupportedOutput)
    );
}

#[test]
fn unknown_or_absent_candidate_authority_is_rejected() {
    let definition = one_agent();
    let value = serde_json::to_value(definition.to_file()).expect("definition json");
    for replacement in [None, Some("unknown")] {
        let mut candidate = value.clone();
        let action = candidate["steps"][0]["action"]
            .as_object_mut()
            .expect("agent action");
        match replacement {
            Some(value) => {
                action.insert(
                    "candidate-authority".to_owned(),
                    serde_json::Value::String(value.to_owned()),
                );
            }
            None => {
                action.remove("candidate-authority");
            }
        }
        let bytes = serde_json::to_vec(&candidate).expect("bytes");
        assert_eq!(
            WorkflowDefinition::from_file_bytes(&bytes).err(),
            Some(DefinitionError::Format)
        );
    }
}

#[test]
fn write_steps_without_candidate_outputs_are_rejected() {
    let mut step = agent_step("reply", SuccessTransition::CompleteRun);
    if let StepAction::Agent(action) = &mut step.action {
        action
            .required_outputs
            .retain(|output| output.kind != OutputKind::CandidateRevision);
    }
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        test_environment_id(),
        vec![role()],
        StepKey::parse("reply").expect("first"),
        vec![step],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::CandidateOutput));
}

#[test]
fn secondary_write_grants_are_rejected() {
    let error = AgentAuthority::new(
        vec![ToolId::List],
        vec![GuestDirectoryAccess {
            alias: "docs".to_owned(),
            access: AccessMode::ReadWrite,
        }],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::SecondaryWrite));
}
