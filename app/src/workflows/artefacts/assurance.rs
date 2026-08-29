use super::id::CandidateHash;
use super::payload::TypedPayload;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AssuranceStatus {
    Current,
    Stale,
    Unknown,
}

impl AssuranceStatus {
    pub(crate) fn as_label(self) -> &'static str {
        match self {
            Self::Current => "Current",
            Self::Stale => "Stale",
            Self::Unknown => "Unknown",
        }
    }
}

pub(crate) fn candidate_constraint(payload: &TypedPayload) -> Option<CandidateHash> {
    match payload {
        TypedPayload::Plan(_) => None,
        TypedPayload::Review(report) => CandidateHash::parse(&report.candidate),
        TypedPayload::Test(report) => CandidateHash::parse(&report.candidate),
    }
}

pub(crate) fn status_against(
    bound: Option<CandidateHash>,
    observed: Option<CandidateHash>,
) -> AssuranceStatus {
    match (bound, observed) {
        (Some(bound), Some(observed)) if bound == observed => AssuranceStatus::Current,
        (Some(_), Some(_)) => AssuranceStatus::Stale,
        _ => AssuranceStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
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
}
