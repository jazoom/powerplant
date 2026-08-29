use super::{ENVIRONMENT_ID_LENGTH, EnvironmentId, PreparationId};

#[test]
fn generated_ids_are_opaque_hex() {
    let id = EnvironmentId::generate().expect("id");
    let hex = id.as_hex();
    assert_eq!(hex.len(), ENVIRONMENT_ID_LENGTH);
    assert!(hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(hex, hex.to_ascii_lowercase());
    assert_eq!(EnvironmentId::parse(&hex), Some(id));
    assert!(PreparationId::parse(&hex).is_some());
}

#[test]
fn parse_rejects_invalid_identifiers() {
    assert!(EnvironmentId::parse("").is_none());
    assert!(EnvironmentId::parse("abc").is_none());
    assert!(EnvironmentId::parse(&"g".repeat(ENVIRONMENT_ID_LENGTH)).is_none());
    assert!(EnvironmentId::parse(&"A".repeat(ENVIRONMENT_ID_LENGTH)).is_none());
    assert!(PreparationId::parse("alpine-git-v1").is_none());
}
