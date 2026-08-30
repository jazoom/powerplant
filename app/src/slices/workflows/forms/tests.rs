use super::{FormError, WorkflowFormState};

fn pair(key: &str, value: &str) -> (String, String) {
    (key.to_owned(), value.to_owned())
}

fn valid_pairs() -> Vec<(String, String)> {
    vec![
        pair("intent", "save"),
        pair("name", "One step"),
        pair(
            "default-environment",
            &crate::workflows::definition::test_environment_id().as_hex(),
        ),
        pair("role_0_key", "coding-agent"),
        pair("role_0_name", "Coding agent"),
        pair("role_0_expertise", ""),
        pair("role_0_prompt", ""),
        pair("step_0_key", "work-on-task"),
        pair("step_0_name", "Work on task"),
        pair("step_0_action", "agent"),
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
