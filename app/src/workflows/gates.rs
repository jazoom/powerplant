use super::artefacts::{ArtefactReference, CandidateHash};
use super::definition::{OutputKey, StepKey};
use super::id::GateId;

pub(crate) const MAXIMUM_REVISION_NOTE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GateRevision(u64);

impl GateRevision {
    pub(crate) fn new(sequence: u64) -> Option<Self> {
        (sequence > 0).then_some(Self(sequence))
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        if value.is_empty()
            || value.len() > 20
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        Self::new(value.parse().ok()?)
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HumanGateState {
    AwaitingDecision,
    Approved,
    RevisionRequested,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HumanGateRecord {
    pub(crate) id: GateId,
    pub(crate) step: StepKey,
    pub(crate) sequence: u32,
    pub(crate) revision: GateRevision,
    pub(crate) opened_at_ms: u64,
    pub(crate) closed_at_ms: Option<u64>,
    pub(crate) candidate: ArtefactReference,
    pub(crate) diff_base: ArtefactReference,
    pub(crate) state: HumanGateState,
    pub(crate) decision: Option<ArtefactReference>,
    pub(crate) output: OutputKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum HumanDecisionKind {
    Approved,
    RevisionRequested,
}

impl HumanDecisionKind {
    pub(crate) fn as_label(self) -> &'static str {
        match self {
            Self::Approved => "Approved",
            Self::RevisionRequested => "Revision requested",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct HumanDecisionPayload {
    pub(crate) format_version: u32,
    pub(crate) candidate: String,
    pub(crate) diff_base: String,
    pub(crate) decision: HumanDecisionKind,
    pub(crate) note: Option<String>,
    pub(crate) decided_at_ms: u64,
}

pub(crate) fn normalise_revision_note(raw: &str) -> Option<String> {
    let note = raw.replace("\r\n", "\n").replace('\r', "\n");
    let note = note.trim();
    if note.is_empty()
        || note.len() > MAXIMUM_REVISION_NOTE_BYTES
        || note
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return None;
    }
    Some(note.to_owned())
}

pub(crate) fn hashes(payload: &HumanDecisionPayload) -> Option<(CandidateHash, CandidateHash)> {
    Some((
        CandidateHash::parse(&payload.candidate)?,
        CandidateHash::parse(&payload.diff_base)?,
    ))
}
