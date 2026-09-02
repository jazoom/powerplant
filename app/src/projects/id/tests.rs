const PROJECT_ID_LENGTH: usize = 32;

use super::ProjectId;

#[test]
fn generated_ids_are_opaque_lowercase_hex() {
    let id = ProjectId::generate().expect("id");
    let hex = id.as_hex();
    assert_eq!(hex.len(), PROJECT_ID_LENGTH);
    assert!(
        hex.bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    );
    assert_eq!(ProjectId::parse(&hex), Some(id));
    assert_eq!(id.to_string(), hex);
}

#[test]
fn parse_rejects_uppercase_and_named_identifiers() {
    assert!(ProjectId::parse("abcdef0123456789abcdef0123456789").is_some());
    assert!(ProjectId::parse("ABCDEF0123456789ABCDEF0123456789").is_none());
    assert!(ProjectId::parse("abcdef0123456789abcdef012345678G").is_none());
    assert!(ProjectId::parse("repository-root").is_none());
}
