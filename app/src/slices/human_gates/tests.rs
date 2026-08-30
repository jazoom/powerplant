use super::forms::{DecisionForm, FormError};

#[test]
fn decision_forms_reject_duplicate_and_blank_revision_fields() {
    let duplicate = vec![
        ("gate-revision".to_owned(), "1".to_owned()),
        ("gate-revision".to_owned(), "1".to_owned()),
        ("candidate".to_owned(), "sha256:00".to_owned()),
    ];
    assert_eq!(
        DecisionForm::parse(duplicate, false).err(),
        Some(FormError::Invalid)
    );

    let blank_note = vec![
        ("gate-revision".to_owned(), "1".to_owned()),
        ("candidate".to_owned(), "sha256:00".to_owned()),
        ("note".to_owned(), "  ".to_owned()),
    ];
    assert_eq!(
        DecisionForm::parse(blank_note, true).err(),
        Some(FormError::Note)
    );
}
