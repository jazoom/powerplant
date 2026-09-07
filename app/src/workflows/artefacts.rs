pub(crate) mod apply;
pub(crate) mod assurance;
pub(crate) mod candidate;
mod confine;
pub(crate) mod diff;
mod id;
pub(crate) mod materialise;
pub(crate) mod output;
pub(crate) mod payload;
mod store;

pub(crate) use apply::CandidateApply;
pub(crate) use assurance::status_against;
pub(crate) use candidate::{
    CANDIDATE_SCHEMA, CandidateCapture, CandidateEntryKind, CandidatePayload,
    inspect_supported_worktree,
};
pub(crate) use diff::CandidateDiff;
pub(crate) use id::{ArtefactHash, CandidateHash, ObjectHash};
pub(crate) use materialise::CandidateMaterialise;
pub(crate) use payload::{
    ReviewVerdict, TestOutcome, TypedPayload, artefact_hash_for, encode_human_decision,
    encode_plan_decision, parse_typed_payload,
};
pub(crate) use store::{ArtefactStoreError, WorkflowArtefactRepository};

use super::definition::ArtefactKind;
use super::definition::{LaunchInputSource, OutputKey, StepKey};
use super::id::{ArtefactId, AttemptId, GateId, RunId};
use crate::conversations::{ConversationId, DocumentId, PlanRevisionReference};

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
    HumanGate {
        gate_id: GateId,
        step: StepKey,
        output: OutputKey,
    },
    LaunchInput {
        source: LaunchInputSource,
        conversation_id: ConversationId,
        document_id: DocumentId,
        revision: u32,
        content_hash: ObjectHash,
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
    HumanDecision {
        candidate: CandidateHash,
        diff_base: CandidateHash,
        decision: crate::workflows::gates::HumanDecisionKind,
    },
    PlanDecision {
        plan: ArtefactHash,
        decision: crate::workflows::gates::PlanDecisionKind,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlanImportError {
    Payload(payload::PayloadError),
    Random,
    Reference,
}

pub(crate) fn import_saved_plan(
    run_id: RunId,
    created_at_ms: u64,
    conversation_id: ConversationId,
    document_id: DocumentId,
    revision: &PlanRevisionReference,
    markdown: &str,
    store: &WorkflowArtefactRepository,
) -> Result<ArtefactRecord, PlanImportError> {
    if revision.document_id != document_id
        || revision.revision == 0
        || revision.content_hash != ObjectHash::of(markdown.as_bytes())
    {
        return Err(PlanImportError::Reference);
    }
    let (bytes, object_hash, artefact_hash) =
        payload::encode_plan(markdown, None).map_err(PlanImportError::Payload)?;
    if object_hash != revision.object_hash || artefact_hash != revision.artefact_hash {
        return Err(PlanImportError::Reference);
    }
    store
        .publish(&bytes)
        .map_err(|_| PlanImportError::Reference)?;
    let id = ArtefactId::generate().map_err(|_| PlanImportError::Random)?;
    Ok(ArtefactRecord {
        id,
        kind: ArtefactKind::Plan,
        artefact_hash,
        object_hash,
        payload_bytes: bytes.len() as u64,
        created_at_ms,
        provenance: ArtefactProvenance {
            run_id,
            producer: ArtefactProducer::LaunchInput {
                source: LaunchInputSource::SavedPlan,
                conversation_id,
                document_id,
                revision: revision.revision,
                content_hash: revision.content_hash,
            },
            inputs: Vec::new(),
        },
        summary: ArtefactSummary::Plan {
            markdown_bytes: markdown.len() as u64,
        },
    })
}

impl ArtefactRecord {
    pub(crate) fn candidate_hash(&self) -> Option<CandidateHash> {
        match &self.summary {
            ArtefactSummary::Review { candidate, .. }
            | ArtefactSummary::Test { candidate, .. }
            | ArtefactSummary::Candidate { candidate, .. }
            | ArtefactSummary::HumanDecision { candidate, .. } => Some(*candidate),
            ArtefactSummary::Plan { .. } | ArtefactSummary::PlanDecision { .. } => None,
        }
    }

    pub(crate) fn assurance_label(&self, observed: Option<CandidateHash>) -> &'static str {
        match &self.summary {
            ArtefactSummary::Review { candidate, .. }
            | ArtefactSummary::Test { candidate, .. }
            | ArtefactSummary::HumanDecision { candidate, .. } => {
                let status = status_against(Some(*candidate), observed);
                if matches!(self.summary, ArtefactSummary::HumanDecision { .. })
                    && status != assurance::AssuranceStatus::Current
                {
                    "Stale"
                } else {
                    status.as_label()
                }
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
            ArtefactSummary::HumanDecision {
                candidate,
                decision,
                ..
            } => format!("{} · {}", decision.as_label(), candidate.short()),
            ArtefactSummary::PlanDecision { plan, decision } => {
                format!("{} · {}", decision.as_label(), plan.short())
            }
            ArtefactSummary::Plan { .. } => String::new(),
        }
    }
}

impl ArtefactProducer {
    pub(crate) fn as_label(&self) -> &'static str {
        match self {
            Self::RunSourceCapture => "Source capture",
            Self::StepAttempt { .. } => "Step attempt",
            Self::HumanGate { .. } => "Human gate",
            Self::LaunchInput { source, .. } => source.label(),
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
