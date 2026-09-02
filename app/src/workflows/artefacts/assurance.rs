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
        TypedPayload::HumanDecision(decision) => CandidateHash::parse(&decision.candidate),
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
mod tests;
