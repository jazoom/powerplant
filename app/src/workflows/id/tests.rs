use super::{AttemptId, RunId, WORKFLOW_ID_LENGTH, WorkflowId};

#[test]
fn generated_ids_are_opaque_hex() {
    let id = RunId::generate().expect("id");
    let hex = id.as_hex();
    assert_eq!(hex.len(), WORKFLOW_ID_LENGTH);
    assert!(hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(hex, hex.to_ascii_lowercase());
    assert_eq!(RunId::parse(&hex), Some(id));
    assert!(WorkflowId::parse(&hex).is_some());
    assert!(AttemptId::parse(&hex).is_some());
}

#[test]
fn parse_rejects_invalid_identifiers() {
    assert!(RunId::parse("").is_none());
    assert!(RunId::parse("abc").is_none());
    assert!(RunId::parse(&"g".repeat(WORKFLOW_ID_LENGTH)).is_none());
    assert!(RunId::parse(&"A".repeat(WORKFLOW_ID_LENGTH)).is_none());
    assert!(RunId::parse("repository-status").is_none());
}
