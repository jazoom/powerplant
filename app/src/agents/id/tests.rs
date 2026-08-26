use super::{AGENT_ID_LENGTH, AgentId};

#[test]
fn generated_ids_are_opaque_hex() {
    let id = AgentId::generate().expect("id");
    let hex = id.as_hex();
    assert_eq!(hex.len(), AGENT_ID_LENGTH);
    assert!(hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(hex, hex.to_ascii_lowercase());
    assert_eq!(AgentId::parse(&hex), Some(id));
}

#[test]
fn parse_rejects_invalid_identifiers() {
    assert!(AgentId::parse("").is_none());
    assert!(AgentId::parse("abc").is_none());
    assert!(AgentId::parse(&"g".repeat(AGENT_ID_LENGTH)).is_none());
    assert!(AgentId::parse(&"A".repeat(AGENT_ID_LENGTH)).is_none());
    assert!(AgentId::parse(&"ab".repeat(AGENT_ID_LENGTH)).is_none());
    assert!(AgentId::parse("repository-maintainer").is_none());
}
