use super::{AGENT_ID_LENGTH, AgentId};

#[test]
fn generated_ids_are_opaque_hex() {
    let id = AgentId::generate().expect("id");
    let hex = id.as_hex();
    assert_eq!(hex.len(), AGENT_ID_LENGTH);
    assert_eq!(AgentId::parse(&hex), Some(id));
}

#[test]
fn parse_rejects_named_identifiers() {
    assert!(AgentId::parse("repository-maintainer").is_none());
}
