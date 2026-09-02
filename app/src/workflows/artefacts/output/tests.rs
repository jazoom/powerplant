use super::*;
use crate::workflows::definition::OutputKey;

fn plan_output() -> RequiredOutput {
    RequiredOutput {
        key: OutputKey::parse("plan").expect("key"),
        kind: OutputKind::Plan,
    }
}

#[test]
fn later_valid_submissions_replace_drafts() {
    let mut drafts = OutputDrafts::default();
    drafts
        .submit(
            &[plan_output()],
            "plan",
            OutputKind::Plan,
            Some("one".to_owned()),
            None,
            None,
            false,
            false,
        )
        .expect("first");
    drafts
        .submit(
            &[plan_output()],
            "plan",
            OutputKind::Plan,
            Some("two".to_owned()),
            None,
            None,
            false,
            false,
        )
        .expect("second");
    assert_eq!(
        drafts.take(&OutputKey::parse("plan").expect("key")),
        Some(OutputDraft::Plan {
            markdown: "two".to_owned()
        })
    );
}

#[test]
fn unknown_keys_and_model_candidate_hashes_are_rejected() {
    let mut drafts = OutputDrafts::default();
    assert_eq!(
        drafts
            .submit(
                &[plan_output()],
                "missing",
                OutputKind::Plan,
                Some("x".to_owned()),
                None,
                None,
                false,
                false
            )
            .err(),
        Some(OutputDraftError::UnknownKey)
    );
    assert_eq!(
        drafts
            .submit(
                &[plan_output()],
                "plan",
                OutputKind::Plan,
                Some("x".to_owned()),
                None,
                None,
                true,
                false
            )
            .err(),
        Some(OutputDraftError::CandidateHash)
    );
}
