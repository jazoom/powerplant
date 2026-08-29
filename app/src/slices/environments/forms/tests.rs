use super::{EnvironmentFormState, FormError, parse_delete, parse_retry};

#[test]
fn parse_rejects_unknown_and_duplicate_fields() {
    assert_eq!(
        EnvironmentFormState::parse(vec![("extra".to_owned(), "1".to_owned())]).err(),
        Some(FormError::UnknownField)
    );
    assert_eq!(
        EnvironmentFormState::parse(vec![
            ("name".to_owned(), "A".to_owned()),
            ("name".to_owned(), "B".to_owned())
        ])
        .err(),
        Some(FormError::DuplicateField)
    );
}

#[test]
fn delete_requires_a_revision() {
    assert_eq!(
        parse_delete(&[("confirm".to_owned(), "on".to_owned())]).err(),
        Some(FormError::Revision)
    );
    let (revision, confirmed) = parse_delete(&[
        ("revision".to_owned(), "3".to_owned()),
        ("confirm".to_owned(), "on".to_owned()),
    ])
    .expect("ok");
    assert_eq!(revision, 3);
    assert!(confirmed);
}

#[test]
fn retry_requires_revision_and_recipe() {
    assert_eq!(
        parse_retry(&[("revision".to_owned(), "1".to_owned())]).err(),
        Some(FormError::Recipe)
    );
}
