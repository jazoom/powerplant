use super::{AttemptId, RunId, WORKFLOW_ID_LENGTH, WorkflowId};

#[test]
fn generated_ids_are_opaque_hex() {
    let id = RunId::generate().expect("id");
    let hex = id.as_hex();
    assert_eq!(hex.len(), WORKFLOW_ID_LENGTH);
    assert_eq!(RunId::parse(&hex), Some(id));
    assert!(WorkflowId::parse(&hex).is_some());
    assert!(AttemptId::parse(&hex).is_some());
}

#[test]
fn parse_rejects_named_identifiers() {
    assert!(RunId::parse("repository-status").is_none());
}
