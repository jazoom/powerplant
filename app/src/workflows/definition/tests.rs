use super::*;

impl super::WorkflowDefinition {
    pub(crate) fn from_file_bytes(bytes: &[u8]) -> Result<Self, DefinitionError> {
        let file: DefinitionFile =
            serde_json::from_slice(bytes).map_err(|_| DefinitionError::Format)?;
        Self::from_file(file)
    }
}

pub(crate) fn test_environment_id() -> EnvironmentId {
    EnvironmentId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("test env")
}
pub(crate) fn test_named_definition(name: &str) -> WorkflowDefinition {
    let role = RoleDefinition::new(
        RoleKey::parse("agent").expect("role"),
        "Coding agent".to_owned(),
        String::new(),
        String::new(),
    )
    .expect("role");
    let authority =
        AgentAuthority::new(vec![crate::agents::ToolId::List], Vec::new()).expect("authority");
    WorkflowDefinition::from_parts(
        name.to_owned(),
        test_environment_id(),
        vec![role],
        vec![StepDefinition {
            key: StepKey::parse("work").expect("step"),
            name: "Work on task".to_owned(),
            inputs: vec![initial_candidate_input()],
            action: StepAction::Agent(AgentStep {
                environment: StepEnvironment::WorkflowDefault,
                role: RoleKey::parse("agent").expect("role"),
                candidate_authority: CandidateAuthority::Edit,
                authority,
                required_outputs: vec![
                    RequiredOutput {
                        key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                        kind: OutputKind::AssistantReply,
                    },
                    candidate_revision_output(),
                ],
            }),
            review: None,
        }],
    )
    .expect("definition")
}

use super::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, ArtefactKind, ArtefactSource, CandidateAuthority,
    DefinitionError, GuestDirectoryAccess, InputKey, OutputKey, OutputKind, RequiredInput,
    RequiredOutput, ReviewPolicy, RoleDefinition, RoleKey, StepAction, StepDefinition,
    StepEnvironment, StepKey, SystemCommandId, SystemCommandStep, WorkflowDefinition,
    candidate_revision_output, initial_candidate_input,
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

fn agent_step(key: &str) -> StepDefinition {
    write_agent_step(
        key,
        ArtefactSource::RunInitialCandidate,
        vec![
            RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                kind: OutputKind::AssistantReply,
            },
            candidate_revision_output(),
        ],
        None,
    )
}

fn write_agent_step(
    key: &str,
    candidate: ArtefactSource,
    outputs: Vec<RequiredOutput>,
    review: Option<ReviewPolicy>,
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
        review,
    }
}

fn command_step(key: &str) -> StepDefinition {
    StepDefinition {
        key: StepKey::parse(key).expect("step"),
        name: "Status".to_owned(),
        inputs: vec![initial_candidate_input()],
        action: StepAction::SystemCommand(SystemCommandStep {
            command: SystemCommandId::RepositoryStatus,
            environment: StepEnvironment::WorkflowDefault,
            required_outputs: Vec::new(),
        }),
        review: None,
    }
}

fn one_agent() -> WorkflowDefinition {
    WorkflowDefinition::from_parts(
        "Maintainer".to_owned(),
        test_environment_id(),
        vec![role()],
        vec![agent_step("reply")],
    )
    .expect("definition")
}

#[test]
fn duplicate_role_keys_are_rejected() {
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        test_environment_id(),
        vec![role(), role()],
        vec![agent_step("reply")],
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
        vec![agent_step("reply"), agent_step("reply")],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::DuplicateStep));
}

#[test]
fn unknown_roles_are_rejected() {
    let mut step = agent_step("reply");
    if let StepAction::Agent(action) = &mut step.action {
        action.role = RoleKey::parse("reviewer").expect("role");
    }
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        test_environment_id(),
        vec![role()],
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
        vec![agent_step("reply")],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::UnusedRole));
}

#[test]
fn arbitrary_command_values_are_rejected() {
    let json = format!(
        r#"{{
        "format-version": 1,
        "name": "Status",
        "default-environment": "{}",
        "roles": [],
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
                "review": null
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
    let mut step = command_step("status");
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
        vec![step],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::UnsupportedOutput));
}

#[test]
fn duplicate_output_keys_are_rejected() {
    let mut step = agent_step("reply");
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
        vec![command_step("status")],
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
        vec![agent_step("reply")],
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
fn malformed_environment_identifiers_are_rejected_before_other_definition_checks() {
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
    let mut step = agent_step("reply");
    if let StepAction::Agent(action) = &mut step.action {
        action.environment = StepEnvironment::Override {
            environment_id: test_environment_id(),
        };
    }
    let definition = WorkflowDefinition::from_parts(
        "Maintainer".to_owned(),
        test_environment_id(),
        vec![role()],
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
        vec![agent_step("reply")],
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
        ArtefactSource::RunInitialCandidate,
        vec![
            RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                kind: OutputKind::AssistantReply,
            },
            candidate_revision_output(),
        ],
        None,
    );
    let mut status = command_step("status");
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
        vec![later, status],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::UnsupportedOutput));
}

#[test]
fn input_kind_mismatch_is_rejected() {
    let planner = write_agent_step(
        "plan",
        ArtefactSource::RunInitialCandidate,
        vec![
            RequiredOutput {
                key: OutputKey::parse("plan").expect("output"),
                kind: OutputKind::Plan,
            },
            candidate_revision_output(),
        ],
        None,
    );
    let mut review = write_agent_step(
        "review",
        from_step("plan", "candidate"),
        vec![
            RequiredOutput {
                key: OutputKey::parse("report").expect("output"),
                kind: OutputKind::ReviewReport,
            },
            candidate_revision_output(),
        ],
        None,
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
        vec![planner, review],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::InputKind));
}

#[test]
fn stale_candidate_sources_are_rejected() {
    let first = agent_step("one");
    let second = write_agent_step(
        "two",
        ArtefactSource::RunInitialCandidate,
        vec![
            RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                kind: OutputKind::AssistantReply,
            },
            candidate_revision_output(),
        ],
        None,
    );
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        test_environment_id(),
        vec![role()],
        vec![first, second],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::CandidateInput));
}

#[test]
fn sandbox_steps_without_candidate_inputs_are_rejected() {
    let mut step = agent_step("reply");
    step.inputs.clear();
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        test_environment_id(),
        vec![role()],
        vec![step],
    )
    .err();
    assert_eq!(error, Some(DefinitionError::CandidateInput));
}

#[test]
fn candidate_authority_rejects_conflicting_outputs() {
    let mut read_only = agent_step("reply");
    let StepAction::Agent(action) = &mut read_only.action else {
        panic!("agent step");
    };
    action.candidate_authority = CandidateAuthority::ReadOnly;
    assert_eq!(
        WorkflowDefinition::from_parts(
            "Read-only".to_owned(),
            test_environment_id(),
            vec![role()],
            vec![read_only],
        )
        .err(),
        Some(DefinitionError::CandidateOutput)
    );

    let mut duplicate_reviews = agent_step("reply");
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
    let mut step = agent_step("reply");
    if let StepAction::Agent(action) = &mut step.action {
        action
            .required_outputs
            .retain(|output| output.kind != OutputKind::CandidateRevision);
    }
    let error = WorkflowDefinition::from_parts(
        "Team".to_owned(),
        test_environment_id(),
        vec![role()],
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

fn rebuild_review_definition(
    steps: Vec<StepDefinition>,
) -> Result<WorkflowDefinition, DefinitionError> {
    let source = crate::workflows::seeds::review_until_approved_definition(test_environment_id());
    WorkflowDefinition::from_parts(
        source.name().to_owned(),
        source.default_environment(),
        source.roles().to_vec(),
        steps,
    )
}

#[test]
fn review_policy_shape_and_attempt_limits_are_validated() {
    enum InvalidPolicy {
        MissingReport,
        ForwardTarget,
        UnknownTarget,
        AttemptLimit(u8),
    }

    for (invalid, expected) in [
        (InvalidPolicy::MissingReport, DefinitionError::ReviewPolicy),
        (InvalidPolicy::ForwardTarget, DefinitionError::ReviewPolicy),
        (InvalidPolicy::UnknownTarget, DefinitionError::UnknownStep),
        (
            InvalidPolicy::AttemptLimit(0),
            DefinitionError::AttemptLimit,
        ),
        (
            InvalidPolicy::AttemptLimit(9),
            DefinitionError::AttemptLimit,
        ),
    ] {
        let source =
            crate::workflows::seeds::review_until_approved_definition(test_environment_id());
        let mut steps = source.steps().to_vec();
        let reviewer = steps
            .iter_mut()
            .find(|step| step.key.as_str() == "reviewer")
            .expect("reviewer");
        let Some(policy) = reviewer.review.as_mut() else {
            panic!("review policy")
        };
        match invalid {
            InvalidPolicy::MissingReport => {
                policy.report_output = OutputKey::parse("missing").expect("output")
            }
            InvalidPolicy::ForwardTarget => {
                policy.revision_target = StepKey::parse("commit").expect("step")
            }
            InvalidPolicy::UnknownTarget => {
                policy.revision_target = StepKey::parse("missing").expect("step")
            }
            InvalidPolicy::AttemptLimit(limit) => policy.attempt_limit = limit,
        }
        assert_eq!(rebuild_review_definition(steps).err(), Some(expected));
    }
}

#[test]
fn repeatable_steps_require_the_current_candidate() {
    let source = crate::workflows::seeds::review_until_approved_definition(test_environment_id());
    let mut steps = source.steps().to_vec();
    let reviewer = steps
        .iter_mut()
        .find(|step| step.key.as_str() == "reviewer")
        .expect("reviewer");
    reviewer
        .inputs
        .iter_mut()
        .find(|input| input.kind == ArtefactKind::CandidateRevision)
        .expect("candidate")
        .source = from_step("implementer", "candidate");
    assert_eq!(
        rebuild_review_definition(steps).err(),
        Some(DefinitionError::CandidateInput)
    );
}

#[test]
fn review_loops_respect_the_conservative_run_bound() {
    let mut steps = Vec::new();
    for index in 0..10 {
        let key = StepKey::parse(&format!("step-{index}")).expect("step");
        let mut outputs = vec![
            RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY).expect("reply"),
                kind: OutputKind::AssistantReply,
            },
            candidate_revision_output(),
        ];
        let review = if index == 0 {
            None
        } else {
            outputs.push(RequiredOutput {
                key: OutputKey::parse("review").expect("review"),
                kind: OutputKind::ReviewReport,
            });
            Some(ReviewPolicy {
                report_output: OutputKey::parse("review").expect("review"),
                revision_target: StepKey::parse("step-0").expect("target"),
                attempt_limit: 8,
            })
        };
        steps.push(write_agent_step(
            key.as_str(),
            ArtefactSource::RunCurrentCandidate,
            outputs,
            review,
        ));
    }
    assert_eq!(
        WorkflowDefinition::from_parts(
            "Bounded".to_owned(),
            test_environment_id(),
            vec![role()],
            steps,
        )
        .err(),
        Some(DefinitionError::RunBound)
    );
}

#[test]
fn version_one_linear_workflows_round_trip_from_vector_order() {
    let definition = crate::workflows::seeds::sequential_team_definition(test_environment_id());
    let value = serde_json::to_value(definition.to_file()).expect("json");
    assert!(value.get("first-step").is_none());
    for step in value["steps"].as_array().expect("steps") {
        assert!(step.get("on-success").is_none());
        assert!(step["review"].is_null());
    }

    let bytes = serde_json::to_vec(&value).expect("bytes");
    let loaded = WorkflowDefinition::from_file_bytes(&bytes).expect("round trip");
    assert_eq!(loaded, definition);
    assert_eq!(loaded.first_step().as_str(), "planner");
    assert_eq!(
        loaded.next_step(loaded.first_step()).map(StepKey::as_str),
        Some("implementer")
    );
    assert_eq!(
        loaded
            .next_step(&StepKey::parse("commit").expect("commit"))
            .map(StepKey::as_str),
        None
    );
}

#[test]
fn version_one_final_steps_round_trip_without_successors() {
    let definition = one_agent();
    let value = serde_json::to_value(definition.to_file()).expect("json");
    assert!(value["steps"][0]["review"].is_null());
    assert!(value["steps"][0].get("on-success").is_none());

    let bytes = serde_json::to_vec(&value).expect("bytes");
    let loaded = WorkflowDefinition::from_file_bytes(&bytes).expect("round trip");
    assert_eq!(loaded, definition);
    assert_eq!(loaded.first_step().as_str(), "reply");
    assert_eq!(loaded.next_step(loaded.first_step()), None);
    assert_eq!(loaded.step_position(loaded.first_step()), Some(0));
}

#[test]
fn version_one_keyed_review_loops_round_trip() {
    let definition =
        crate::workflows::seeds::review_until_approved_definition(test_environment_id());
    let value = serde_json::to_value(definition.to_file()).expect("json");
    let reviewer = value["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["key"] == "reviewer")
        .expect("reviewer");
    assert!(reviewer.get("on-success").is_none());
    assert_eq!(reviewer["review"]["revision-target"], "implementer");
    assert!(reviewer["review"].get("approved-target").is_none());

    let bytes = serde_json::to_vec(&value).expect("bytes");
    let loaded = WorkflowDefinition::from_file_bytes(&bytes).expect("round trip");
    assert_eq!(loaded, definition);
    let reviewer_key = StepKey::parse("reviewer").expect("reviewer");
    let policy = loaded
        .step(&reviewer_key)
        .and_then(|step| step.review.as_ref())
        .expect("review policy");
    assert_eq!(policy.revision_target.as_str(), "implementer");
    assert_eq!(
        loaded.next_step(&reviewer_key).map(StepKey::as_str),
        Some("commit")
    );
    assert!(loaded.step_position(&policy.revision_target) < loaded.step_position(&reviewer_key));
}

#[test]
fn current_step_fields_are_required() {
    for field in ["inputs", "review"] {
        let mut value = serde_json::to_value(one_agent().to_file()).expect("json");
        value["steps"][0]
            .as_object_mut()
            .expect("step")
            .remove(field);
        let bytes = serde_json::to_vec(&value).expect("bytes");
        assert_eq!(
            WorkflowDefinition::from_file_bytes(&bytes).err(),
            Some(DefinitionError::Format),
            "{field}"
        );
    }
}

#[test]
fn obsolete_first_step_fields_are_rejected() {
    let mut value = serde_json::to_value(one_agent().to_file()).expect("json");
    value.as_object_mut().expect("object").insert(
        "first-step".to_owned(),
        serde_json::Value::String("reply".to_owned()),
    );
    let bytes = serde_json::to_vec(&value).expect("bytes");
    assert_eq!(
        WorkflowDefinition::from_file_bytes(&bytes).err(),
        Some(DefinitionError::Format)
    );
}

#[test]
fn obsolete_on_success_fields_are_rejected() {
    let mut value = serde_json::to_value(one_agent().to_file()).expect("json");
    value["steps"][0].as_object_mut().expect("step").insert(
        "on-success".to_owned(),
        serde_json::json!({ "type": "complete-run" }),
    );
    let bytes = serde_json::to_vec(&value).expect("bytes");
    assert_eq!(
        WorkflowDefinition::from_file_bytes(&bytes).err(),
        Some(DefinitionError::Format)
    );
}
