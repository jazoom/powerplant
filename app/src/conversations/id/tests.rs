use super::ConversationId;

#[test]
fn identifiers_are_opaque_and_round_trip() {
    let id = ConversationId::generate().expect("identifier");
    let encoded = id.as_hex();
    assert_eq!(encoded.len(), 32);
    assert_eq!(ConversationId::parse(&encoded), Some(id));
    assert!(ConversationId::parse("project-name").is_none());
}
