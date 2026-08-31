use super::{ENVIRONMENT_ID_LENGTH, EnvironmentId, PreparationId};

#[test]
fn generated_ids_are_opaque_hex() {
    let id = EnvironmentId::generate().expect("id");
    let hex = id.as_hex();
    assert_eq!(hex.len(), ENVIRONMENT_ID_LENGTH);
    assert_eq!(EnvironmentId::parse(&hex), Some(id));
    assert!(PreparationId::parse(&hex).is_some());
}

#[test]
fn parse_rejects_named_identifiers() {
    assert!(PreparationId::parse("alpine-git-v1").is_none());
}
