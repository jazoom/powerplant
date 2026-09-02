use super::*;
use crate::workflows::artefacts::payload::{ReviewReportArtefact, ReviewVerdict};

#[test]
fn approved_reports_are_current_until_the_candidate_changes() {
    let candidate = CandidateHash::of(b"same");
    let payload = TypedPayload::Review(ReviewReportArtefact {
        format_version: 1,
        candidate: candidate.as_str(),
        verdict: ReviewVerdict::Approved,
        markdown: "ok".to_owned(),
    });
    assert_eq!(
        status_against(candidate_constraint(&payload), Some(candidate)),
        AssuranceStatus::Current
    );
    assert_eq!(
        status_against(
            candidate_constraint(&payload),
            Some(CandidateHash::of(b"other"))
        ),
        AssuranceStatus::Stale
    );
    assert_eq!(
        status_against(candidate_constraint(&payload), None),
        AssuranceStatus::Unknown
    );
}
