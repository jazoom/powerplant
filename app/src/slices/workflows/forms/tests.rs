use super::{FormError, FormIntent, WorkflowFormState, can_move_step, can_remove_step};

fn pair(key: &str, value: &str) -> (String, String) {
    (key.to_owned(), value.to_owned())
}

fn valid_pairs() -> Vec<(String, String)> {
    vec![
        pair("intent", "save"),
        pair("name", "One step"),
        pair(
            "default-environment",
            &crate::tests::test_environment_id().as_hex(),
        ),
        pair("role_0_key", "coding-agent"),
        pair("role_0_name", "Coding agent"),
        pair("role_0_expertise", ""),
        pair("role_0_prompt", ""),
        pair("step_0_key", "work-on-task"),
        pair("step_0_name", "Work on task"),
        pair("step_0_action", "agent"),
        pair("step_0_review-policy", "none"),
        pair("step_0_role", "coding-agent"),
        pair("step_0_candidate-access", "edit-candidate"),
        pair("step_0_tool_list", "on"),
        pair("step_0_input_0_key", "candidate"),
        pair("step_0_input_0_kind", "candidate-revision"),
        pair("step_0_input_0_source", "run-initial-candidate"),
        pair("step_0_output_0_key", "assistant-reply"),
        pair("step_0_output_0_kind", "assistant-reply"),
        pair("step_0_output_1_key", "candidate"),
        pair("step_0_output_1_kind", "candidate-revision"),
    ]
}

fn review_pairs() -> Vec<(String, String)> {
    let mut pairs = valid_pairs();
    pairs
        .iter_mut()
        .find(|(key, _)| key == "step_0_input_0_source")
        .expect("candidate source")
        .1 = "run-current-candidate".to_owned();
    pairs.extend([
        pair("role_1_key", "reviewer"),
        pair("role_1_name", "Reviewer"),
        pair("role_1_expertise", ""),
        pair("role_1_prompt", ""),
        pair("step_1_key", "review"),
        pair("step_1_name", "Review"),
        pair("step_1_action", "agent"),
        pair("step_1_review-policy", "review-verdict"),
        pair("step_1_report-output", "review"),
        pair("step_1_revision-target", "work-on-task"),
        pair("step_1_attempt-limit", "3"),
        pair("step_1_role", "reviewer"),
        pair("step_1_candidate-access", "read-only"),
        pair("step_1_tool_list", "on"),
        pair("step_1_input_0_key", "candidate"),
        pair("step_1_input_0_kind", "candidate-revision"),
        pair("step_1_input_0_source", "run-current-candidate"),
        pair("step_1_output_0_key", "assistant-reply"),
        pair("step_1_output_0_kind", "assistant-reply"),
        pair("step_1_output_1_key", "review"),
        pair("step_1_output_1_kind", "review-report"),
    ]);
    pairs
}

#[test]
fn unknown_intents_are_rejected() {
    let mut pairs = valid_pairs();
    pairs[0] = pair("intent", "explode");
    assert_eq!(
        WorkflowFormState::parse(pairs).err(),
        Some(FormError::Intent)
    );
}

#[test]
fn unknown_fields_are_rejected() {
    let mut pairs = valid_pairs();
    pairs.push(pair("shell", "rm -rf /"));
    assert_eq!(
        WorkflowFormState::parse(pairs).err(),
        Some(FormError::UnknownField)
    );
}

#[test]
fn duplicate_fields_are_rejected() {
    let mut pairs = valid_pairs();
    pairs.push(pair("name", "Other"));
    assert_eq!(
        WorkflowFormState::parse(pairs).err(),
        Some(FormError::DuplicateField)
    );
}

#[test]
fn sparse_role_indices_are_rejected() {
    let mut pairs = valid_pairs();
    pairs.push(pair("role_2_key", "extra"));
    pairs.push(pair("role_2_name", "Extra"));
    pairs.push(pair("role_2_expertise", ""));
    pairs.push(pair("role_2_prompt", ""));
    assert_eq!(
        WorkflowFormState::parse(pairs).err(),
        Some(FormError::Sparse)
    );
}

#[test]
fn malformed_indices_are_rejected() {
    let mut pairs = valid_pairs();
    pairs.push(pair("role_01_key", "padded"));
    assert_eq!(
        WorkflowFormState::parse(pairs).err(),
        Some(FormError::Index)
    );
}

#[test]
fn unknown_role_is_related_to_the_step_control() {
    let mut pairs = valid_pairs();
    pairs
        .iter_mut()
        .find(|(key, _)| key == "step_0_role")
        .expect("role")
        .1 = "missing".to_owned();
    let (form, _) = WorkflowFormState::parse(pairs).expect("parse");
    let errors = form.to_definition().expect_err("invalid");
    assert_eq!(errors.steps[0].role, "An agent step names an unknown role.");
}

#[test]
fn system_command_rejects_arbitrary_command_values() {
    let mut pairs = valid_pairs();
    pairs
        .iter_mut()
        .find(|(key, _)| key == "step_0_action")
        .expect("action")
        .1 = "system-command".to_owned();
    pairs.push(pair("step_0_command", "rm"));
    let (form, _) = WorkflowFormState::parse(pairs).expect("parse");
    let errors = form.to_definition().expect_err("invalid");
    assert_eq!(
        errors.steps[0].command,
        "Choose a registered system command."
    );
}

#[test]
fn a_row_action_normalises_a_step_after_a_system_command_switch() {
    let mut pairs = valid_pairs();
    pairs[0] = pair("intent", "add-role");
    pairs
        .iter_mut()
        .find(|(key, _)| key == "step_0_action")
        .expect("action")
        .1 = "system-command".to_owned();
    pairs.push(pair("step_0_command", "commit-candidate"));

    let (form, _) = WorkflowFormState::parse(pairs).expect("parse");

    assert!(form.steps[0].tools.is_empty());
    assert!(form.steps[0].directories.is_empty());
    assert_eq!(
        form.steps[0]
            .inputs
            .iter()
            .map(|input| input.kind.as_str())
            .collect::<Vec<_>>(),
        ["candidate-revision", "review-report"]
    );
    assert_eq!(form.steps[0].outputs.len(), 1);
    assert_eq!(form.steps[0].outputs[0].kind, "candidate-revision");
}

#[test]
fn a_human_decision_commit_does_not_force_a_review() {
    let mut pairs = valid_pairs();
    pairs[0] = pair("intent", "add-role");
    pairs
        .iter_mut()
        .find(|(key, _)| key == "step_0_action")
        .expect("action")
        .1 = "system-command".to_owned();
    pairs.extend([
        pair("step_0_command", "commit-candidate"),
        pair("step_0_input_1_key", "decision"),
        pair("step_0_input_1_kind", "human-decision"),
        pair("step_0_input_1_source", "step-output:approve:decision"),
    ]);

    let (form, _) = WorkflowFormState::parse(pairs).expect("parse");

    assert_eq!(
        form.steps[0]
            .inputs
            .iter()
            .map(|input| input.kind.as_str())
            .collect::<Vec<_>>(),
        ["candidate-revision", "human-decision"]
    );
}

#[test]
fn candidate_access_conflicts_have_field_errors() {
    let mut read_only = valid_pairs();
    read_only
        .iter_mut()
        .find(|(key, _)| key == "step_0_candidate-access")
        .expect("access")
        .1 = "read-only".to_owned();
    let (form, _) = WorkflowFormState::parse(read_only).expect("parse read-only");
    assert_eq!(
        form.to_definition().expect_err("candidate conflict").steps[0].candidate_access,
        "A read-only step cannot produce a candidate revision."
    );

    let mut edit = valid_pairs();
    edit.retain(|(key, _)| !key.starts_with("step_0_output_1_"));
    let (form, _) = WorkflowFormState::parse(edit).expect("parse edit");
    assert_eq!(
        form.to_definition().expect_err("missing candidate").steps[0].candidate_access,
        "An edit step needs one candidate revision output."
    );
}

#[test]
fn changing_to_edit_access_adds_a_candidate_output() {
    let mut previous_pairs = valid_pairs();
    previous_pairs
        .iter_mut()
        .find(|(key, _)| key == "step_0_candidate-access")
        .expect("access")
        .1 = "read-only".to_owned();
    previous_pairs.retain(|(key, _)| !key.starts_with("step_0_output_1_"));
    let (previous, _) = WorkflowFormState::parse(previous_pairs).expect("previous");

    let mut changed = previous.clone();
    changed.steps[0].candidate_access = "edit-candidate".to_owned();
    changed.maintain_candidate_outputs_from(&previous);

    assert_eq!(
        changed.steps[0]
            .outputs
            .iter()
            .filter(|output| output.kind == "candidate-revision")
            .count(),
        1
    );
}

#[test]
fn a_malformed_default_environment_is_rejected() {
    let mut pairs = valid_pairs();
    pairs
        .iter_mut()
        .find(|(key, _)| key == "default-environment")
        .expect("default")
        .1 = "not-an-id".to_owned();
    let (form, _) = WorkflowFormState::parse(pairs).expect("parse");
    let errors = form.to_definition().expect_err("invalid");
    assert_eq!(
        errors.default_environment,
        "Enter a valid environment identifier."
    );
}

#[test]
fn add_role_preserves_incomplete_fields() {
    let mut pairs = valid_pairs();
    pairs[0] = pair("intent", "add-role");
    pairs
        .iter_mut()
        .find(|(key, _)| key == "name")
        .expect("name")
        .1 = String::new();
    let (mut form, intent) = WorkflowFormState::parse(pairs).expect("parse");
    form.apply(intent).expect("apply");
    assert_eq!(form.roles.len(), 2);
    assert!(form.name.is_empty());
}

#[test]
fn review_policy_values_are_strict_and_bounded() {
    let mut missing_policy = valid_pairs();
    missing_policy.retain(|(key, _)| key != "step_0_review-policy");
    assert_eq!(
        WorkflowFormState::parse(missing_policy).err(),
        Some(FormError::MissingField)
    );

    for value in ["normal", "conditional", "graph", ""] {
        let mut pairs = valid_pairs();
        pairs
            .iter_mut()
            .find(|(key, _)| key == "step_0_review-policy")
            .expect("policy")
            .1 = value.to_owned();
        assert_eq!(
            WorkflowFormState::parse(pairs).err(),
            Some(FormError::ReviewPolicy)
        );
    }

    let mut stale_details_on_none = valid_pairs();
    stale_details_on_none.extend([
        pair("step_0_report-output", "review"),
        pair("step_0_revision-target", "work-on-task"),
        pair("step_0_attempt-limit", "3"),
    ]);
    let (form, _) = WorkflowFormState::parse(stale_details_on_none).expect("remove policy");
    assert!(form.steps[0].review_policy.is_none());

    let mut partial_review = review_pairs();
    partial_review.retain(|(key, _)| key != "step_1_attempt-limit");
    assert_eq!(
        WorkflowFormState::parse(partial_review).err(),
        Some(FormError::MissingField)
    );

    let mut omitted_review_details = review_pairs();
    omitted_review_details.retain(|(key, _)| {
        key != "step_1_report-output"
            && key != "step_1_revision-target"
            && key != "step_1_attempt-limit"
    });
    let (form, _) = WorkflowFormState::parse(omitted_review_details).expect("parse");
    let errors = form.to_definition().expect_err("details");
    assert!(!errors.steps[1].report_output.is_empty());

    let mut update_policy = review_pairs();
    update_policy[0] = pair("intent", "update-review-policy:1");
    update_policy.retain(|(key, _)| {
        key != "step_1_report-output"
            && key != "step_1_revision-target"
            && key != "step_1_attempt-limit"
    });
    let (mut form, intent) = WorkflowFormState::parse(update_policy).expect("update policy");
    assert_eq!(intent, FormIntent::UpdateReviewPolicy(1));
    form.apply(intent).expect("apply policy");
    let policy = form.steps[1].review_policy.as_ref().expect("review policy");
    assert!(policy.report_output.is_empty());
    assert!(policy.revision_target.is_empty());
    assert_eq!(policy.attempt_limit, "3");

    for limit in ["0", "9", "-1", "many"] {
        let mut pairs = review_pairs();
        pairs
            .iter_mut()
            .find(|(key, _)| key == "step_1_attempt-limit")
            .expect("attempt limit")
            .1 = limit.to_owned();
        let (form, _) = WorkflowFormState::parse(pairs).expect("parse");
        assert!(
            !form.to_definition().expect_err("limit").steps[1]
                .attempt_limit
                .is_empty()
        );
    }

    let mut stale_target = review_pairs();
    stale_target
        .iter_mut()
        .find(|(key, _)| key == "step_1_revision-target")
        .expect("revision target")
        .1 = "deleted-step".to_owned();
    let (form, _) = WorkflowFormState::parse(stale_target).expect("parse");
    assert_eq!(
        form.to_definition().expect_err("stale target").summary,
        "A review policy names an unknown step."
    );
}

#[test]
fn step_moves_preserve_review_targets_or_fail() {
    let (mut form, _) = WorkflowFormState::parse(review_pairs()).expect("parse");
    form.to_definition().expect("valid review loop");
    let original_keys: Vec<_> = form.steps.iter().map(|step| step.key.clone()).collect();
    let revision_target = form.steps[1]
        .review_policy
        .as_ref()
        .expect("review policy")
        .revision_target
        .clone();

    assert!(!can_move_step(&form.steps, 1, true));
    assert_eq!(
        form.apply(FormIntent::MoveStepUp(1)),
        Err(FormError::ReviewTarget)
    );
    assert_eq!(
        form.steps
            .iter()
            .map(|step| step.key.clone())
            .collect::<Vec<_>>(),
        original_keys
    );
    assert_eq!(
        form.steps[1]
            .review_policy
            .as_ref()
            .expect("review policy")
            .revision_target,
        revision_target
    );

    let mut trailing = form.steps[0].clone();
    trailing.key = "trailing".to_owned();
    trailing.review_policy = None;
    form.steps.push(trailing);
    assert!(can_move_step(&form.steps, 1, false));
    form.apply(FormIntent::MoveStepDown(1)).expect("valid move");
    assert_eq!(
        form.steps[2]
            .review_policy
            .as_ref()
            .expect("review policy")
            .revision_target,
        "work-on-task"
    );
}

#[test]
fn step_removals_preserve_review_targets_or_fail() {
    let (mut form, _) = WorkflowFormState::parse(review_pairs()).expect("parse");
    let original_keys: Vec<_> = form.steps.iter().map(|step| step.key.clone()).collect();

    assert!(!can_remove_step(&form.steps, 0));
    assert_eq!(
        form.apply(FormIntent::RemoveStep(0)),
        Err(FormError::ReviewTarget)
    );
    assert_eq!(
        form.steps
            .iter()
            .map(|step| step.key.clone())
            .collect::<Vec<_>>(),
        original_keys
    );
    assert_eq!(
        form.steps[1]
            .review_policy
            .as_ref()
            .expect("review policy")
            .revision_target,
        "work-on-task"
    );

    assert!(can_remove_step(&form.steps, 1));
    form.apply(FormIntent::RemoveStep(1))
        .expect("remove review step");
    assert_eq!(form.steps.len(), 1);
    assert_eq!(form.steps[0].key, "work-on-task");
    assert!(form.steps[0].review_policy.is_none());
}
