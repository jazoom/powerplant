pub(crate) mod assurance;
pub(crate) mod candidate;
mod id;
pub(crate) mod output;
pub(crate) mod payload;
mod store;

pub(crate) use assurance::status_against;
pub(crate) use candidate::{CANDIDATE_SCHEMA, CandidateCapture, CandidateEntryKind};
pub(crate) use id::{ArtefactHash, CandidateHash, ObjectHash};
pub(crate) use payload::{
    ReviewVerdict, TestOutcome, TypedPayload, artefact_hash_for, parse_typed_payload,
};
pub(crate) use store::WorkflowArtefactRepository;

use super::definition::ArtefactKind;
use super::definition::{OutputKey, StepKey};
use super::id::{ArtefactId, AttemptId, RunId};

pub(crate) const MAXIMUM_ARTEFACTS: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtefactRecord {
    pub(crate) id: ArtefactId,
    pub(crate) kind: ArtefactKind,
    pub(crate) artefact_hash: ArtefactHash,
    pub(crate) object_hash: ObjectHash,
    pub(crate) payload_bytes: u64,
    pub(crate) created_at_ms: u64,
    pub(crate) provenance: ArtefactProvenance,
    pub(crate) summary: ArtefactSummary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtefactProvenance {
    pub(crate) run_id: RunId,
    pub(crate) producer: ArtefactProducer,
    pub(crate) inputs: Vec<ArtefactReference>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ArtefactProducer {
    RunSourceCapture,
    StepAttempt {
        attempt_id: AttemptId,
        step: StepKey,
        output: Option<OutputKey>,
        disposition: ProductionDisposition,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProductionDisposition {
    RequiredOutput,
    ObservedAfterFailure,
    SourceDrift,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtefactReference {
    pub(crate) id: ArtefactId,
    pub(crate) kind: ArtefactKind,
    pub(crate) artefact_hash: ArtefactHash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ArtefactSummary {
    Plan {
        markdown_bytes: u64,
    },
    Review {
        candidate: CandidateHash,
        verdict: ReviewVerdict,
    },
    Test {
        candidate: CandidateHash,
        outcome: TestOutcome,
    },
    Candidate {
        candidate: CandidateHash,
        entries: u64,
        bytes: u64,
        disposition: ProductionDisposition,
    },
}

impl ArtefactRecord {
    pub(crate) fn candidate_hash(&self) -> Option<CandidateHash> {
        match &self.summary {
            ArtefactSummary::Review { candidate, .. }
            | ArtefactSummary::Test { candidate, .. }
            | ArtefactSummary::Candidate { candidate, .. } => Some(*candidate),
            ArtefactSummary::Plan { .. } => None,
        }
    }

    pub(crate) fn assurance_label(&self, observed: Option<CandidateHash>) -> &'static str {
        match &self.summary {
            ArtefactSummary::Review { candidate, .. } | ArtefactSummary::Test { candidate, .. } => {
                status_against(Some(*candidate), observed).as_label()
            }
            _ => "",
        }
    }

    pub(crate) fn constraint_label(&self) -> String {
        match &self.summary {
            ArtefactSummary::Review { candidate, verdict } => {
                format!("{} · {}", verdict.as_label(), candidate.short())
            }
            ArtefactSummary::Test { candidate, outcome } => {
                format!("{} · {}", outcome.as_label(), candidate.short())
            }
            ArtefactSummary::Candidate {
                candidate,
                entries,
                bytes,
                ..
            } => format!(
                "{} · {} entries · {} bytes",
                candidate.short(),
                entries,
                bytes
            ),
            ArtefactSummary::Plan { .. } => String::new(),
        }
    }
}

impl ArtefactProducer {
    pub(crate) fn as_label(&self) -> &'static str {
        match self {
            Self::RunSourceCapture => "Source capture",
            Self::StepAttempt { .. } => "Step attempt",
        }
    }
}

impl ProductionDisposition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RequiredOutput => "required-output",
            Self::ObservedAfterFailure => "observed-after-failure",
            Self::SourceDrift => "source-drift",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "required-output" => Some(Self::RequiredOutput),
            "observed-after-failure" => Some(Self::ObservedAfterFailure),
            "source-drift" => Some(Self::SourceDrift),
            _ => None,
        }
    }
}
