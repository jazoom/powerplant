use serde::{Deserialize, Serialize};

use super::id::{ArtefactHash, CandidateHash, ObjectHash};
use crate::workflows::definition::ArtefactKind;

pub(crate) const PLAN_SCHEMA: u32 = 1;
pub(crate) const HUMAN_DECISION_SCHEMA: u32 = 1;
pub(crate) const MAXIMUM_PLAN_BYTES: usize = 256 * 1024;
const ARTEFACT_DOMAIN: &[u8] = b"powerplant.artefact.v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct PlanArtefact {
    pub(crate) format_version: u32,
    pub(crate) markdown: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct ReviewReportArtefact {
    pub(crate) format_version: u32,
    pub(crate) candidate: String,
    pub(crate) verdict: ReviewVerdict,
    pub(crate) markdown: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ReviewVerdict {
    Approved,
    RevisionRequired,
    Blocked,
}

impl ReviewVerdict {
    pub(crate) fn as_label(self) -> &'static str {
        match self {
            Self::Approved => "Approved",
            Self::RevisionRequired => "Revision required",
            Self::Blocked => "Blocked",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct TestReportArtefact {
    pub(crate) format_version: u32,
    pub(crate) candidate: String,
    pub(crate) outcome: TestOutcome,
    pub(crate) markdown: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TestOutcome {
    Passed,
    Failed,
    NotRun,
}

impl TestOutcome {
    pub(crate) fn as_label(self) -> &'static str {
        match self {
            Self::Passed => "Passed",
            Self::Failed => "Failed",
            Self::NotRun => "Not run",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TypedPayload {
    Plan(PlanArtefact),
    Review(ReviewReportArtefact),
    Test(TestReportArtefact),
    HumanDecision(crate::workflows::gates::HumanDecisionPayload),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PayloadError {
    Encoding,
    DuplicateField,
    Format,
    Text,
    Bound,
    Candidate,
    Credential,
}

pub(crate) fn encode_plan(
    markdown: &str,
    secret: Option<&str>,
) -> Result<(Vec<u8>, ObjectHash, ArtefactHash), PayloadError> {
    let markdown = normalise_text(markdown, MAXIMUM_PLAN_BYTES, secret)?;
    let payload = PlanArtefact {
        format_version: PLAN_SCHEMA,
        markdown,
    };
    encode(ArtefactKind::Plan, PLAN_SCHEMA, &payload)
}

pub(crate) fn encode_review(
    candidate: CandidateHash,
    verdict: ReviewVerdict,
    markdown: &str,
    secret: Option<&str>,
) -> Result<(Vec<u8>, ObjectHash, ArtefactHash), PayloadError> {
    let markdown = normalise_text(markdown, MAXIMUM_PLAN_BYTES, secret)?;
    let payload = ReviewReportArtefact {
        format_version: PLAN_SCHEMA,
        candidate: candidate.as_str(),
        verdict,
        markdown,
    };
    encode(ArtefactKind::ReviewReport, PLAN_SCHEMA, &payload)
}

pub(crate) fn encode_test(
    candidate: CandidateHash,
    outcome: TestOutcome,
    markdown: &str,
    secret: Option<&str>,
) -> Result<(Vec<u8>, ObjectHash, ArtefactHash), PayloadError> {
    let markdown = normalise_text(markdown, MAXIMUM_PLAN_BYTES, secret)?;
    let payload = TestReportArtefact {
        format_version: PLAN_SCHEMA,
        candidate: candidate.as_str(),
        outcome,
        markdown,
    };
    encode(ArtefactKind::TestReport, PLAN_SCHEMA, &payload)
}

pub(crate) fn encode_human_decision(
    candidate: CandidateHash,
    diff_base: CandidateHash,
    decision: crate::workflows::gates::HumanDecisionKind,
    note: Option<&str>,
    decided_at_ms: u64,
    secret: Option<&str>,
) -> Result<(Vec<u8>, ObjectHash, ArtefactHash), PayloadError> {
    let note = match (decision, note) {
        (crate::workflows::gates::HumanDecisionKind::Approved, None) => None,
        (crate::workflows::gates::HumanDecisionKind::RevisionRequested, Some(note)) => {
            Some(crate::workflows::gates::normalise_revision_note(note).ok_or(PayloadError::Text)?)
        }
        _ => return Err(PayloadError::Format),
    };
    if let Some(secret) = secret.filter(|value| !value.is_empty())
        && note.as_ref().is_some_and(|note| {
            note.as_bytes()
                .windows(secret.len())
                .any(|window| window == secret.as_bytes())
        })
    {
        return Err(PayloadError::Credential);
    }
    let payload = crate::workflows::gates::HumanDecisionPayload {
        format_version: HUMAN_DECISION_SCHEMA,
        candidate: candidate.as_str(),
        diff_base: diff_base.as_str(),
        decision,
        note,
        decided_at_ms,
    };
    encode(ArtefactKind::HumanDecision, HUMAN_DECISION_SCHEMA, &payload)
}

pub(crate) fn parse_typed_payload(
    kind: ArtefactKind,
    bytes: &[u8],
) -> Result<TypedPayload, PayloadError> {
    reject_duplicate_keys(bytes)?;
    match kind {
        ArtefactKind::Plan => {
            let payload: PlanArtefact =
                serde_json::from_slice(bytes).map_err(|_| PayloadError::Encoding)?;
            if payload.format_version != PLAN_SCHEMA {
                return Err(PayloadError::Format);
            }
            let _ = normalise_text(&payload.markdown, MAXIMUM_PLAN_BYTES, None)?;
            Ok(TypedPayload::Plan(payload))
        }
        ArtefactKind::ReviewReport => {
            let payload: ReviewReportArtefact =
                serde_json::from_slice(bytes).map_err(|_| PayloadError::Encoding)?;
            if payload.format_version != PLAN_SCHEMA {
                return Err(PayloadError::Format);
            }
            CandidateHash::parse(&payload.candidate).ok_or(PayloadError::Candidate)?;
            let _ = normalise_text(&payload.markdown, MAXIMUM_PLAN_BYTES, None)?;
            Ok(TypedPayload::Review(payload))
        }
        ArtefactKind::TestReport => {
            let payload: TestReportArtefact =
                serde_json::from_slice(bytes).map_err(|_| PayloadError::Encoding)?;
            if payload.format_version != PLAN_SCHEMA {
                return Err(PayloadError::Format);
            }
            CandidateHash::parse(&payload.candidate).ok_or(PayloadError::Candidate)?;
            let _ = normalise_text(&payload.markdown, MAXIMUM_PLAN_BYTES, None)?;
            Ok(TypedPayload::Test(payload))
        }
        ArtefactKind::HumanDecision => {
            let payload: crate::workflows::gates::HumanDecisionPayload =
                serde_json::from_slice(bytes).map_err(|_| PayloadError::Encoding)?;
            if payload.format_version != HUMAN_DECISION_SCHEMA
                || crate::workflows::gates::hashes(&payload).is_none()
                || payload.decided_at_ms == 0
            {
                return Err(PayloadError::Format);
            }
            match (payload.decision, payload.note.as_deref()) {
                (crate::workflows::gates::HumanDecisionKind::Approved, None) => {}
                (crate::workflows::gates::HumanDecisionKind::RevisionRequested, Some(note))
                    if crate::workflows::gates::normalise_revision_note(note).as_deref()
                        == Some(note) => {}
                _ => return Err(PayloadError::Text),
            }
            Ok(TypedPayload::HumanDecision(payload))
        }
        ArtefactKind::CandidateRevision => Err(PayloadError::Format),
    }
}

pub(crate) fn artefact_hash_for(kind: ArtefactKind, schema: u32, payload: &[u8]) -> ArtefactHash {
    let mut domain = Vec::from(ARTEFACT_DOMAIN);
    domain.push(0);
    domain.extend_from_slice(kind.as_str().as_bytes());
    domain.push(0);
    domain.extend_from_slice(&schema.to_be_bytes());
    ArtefactHash::of(&domain, payload)
}

fn encode<T: Serialize>(
    kind: ArtefactKind,
    schema: u32,
    value: &T,
) -> Result<(Vec<u8>, ObjectHash, ArtefactHash), PayloadError> {
    let bytes = canonical_json(value)?;
    let object = ObjectHash::of(&bytes);
    let artefact = artefact_hash_for(kind, schema, &bytes);
    Ok((bytes, object, artefact))
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, PayloadError> {
    serde_json::to_vec(value).map_err(|_| PayloadError::Encoding)
}

fn normalise_text(raw: &str, maximum: usize, secret: Option<&str>) -> Result<String, PayloadError> {
    if raw.as_bytes().contains(&0) {
        return Err(PayloadError::Text);
    }
    let normalised = raw.replace("\r\n", "\n").replace('\r', "\n");
    if normalised.len() > maximum {
        return Err(PayloadError::Bound);
    }
    if let Some(secret) = secret.filter(|value| !value.is_empty())
        && normalised
            .as_bytes()
            .windows(secret.len())
            .any(|window| window == secret.as_bytes())
    {
        return Err(PayloadError::Credential);
    }
    Ok(normalised)
}

fn reject_duplicate_keys(bytes: &[u8]) -> Result<(), PayloadError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| PayloadError::Encoding)?;
    let serde_json::Value::Object(map) = &value else {
        return Err(PayloadError::Encoding);
    };
    let decoded: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(bytes).map_err(|_| PayloadError::Encoding)?;
    if decoded.len() != map.len() {
        return Err(PayloadError::DuplicateField);
    }
    let raw = std::str::from_utf8(bytes).map_err(|_| PayloadError::Encoding)?;
    let mut seen = Vec::new();
    for key in object_keys(raw)? {
        if seen.iter().any(|item: &String| item == &key) {
            return Err(PayloadError::DuplicateField);
        }
        seen.push(key);
    }
    Ok(())
}

fn object_keys(raw: &str) -> Result<Vec<String>, PayloadError> {
    let trimmed = raw.trim();
    let Some(body) = trimmed
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    else {
        return Err(PayloadError::Encoding);
    };
    let mut keys = Vec::new();
    let mut chars = body.char_indices().peekable();
    while let Some((_, character)) = chars.next() {
        if character == '"' {
            let mut key = String::new();
            loop {
                let Some((_, next)) = chars.next() else {
                    return Err(PayloadError::Encoding);
                };
                if next == '"' {
                    break;
                }
                if next == '\\' {
                    let Some((_, escaped)) = chars.next() else {
                        return Err(PayloadError::Encoding);
                    };
                    key.push(escaped);
                } else {
                    key.push(next);
                }
            }
            keys.push(key);
            while let Some((_, next)) = chars.peek().copied() {
                if next == ',' || next == '{' || next == '[' {
                    chars.next();
                    if next != ',' {
                        skip_value(&mut chars)?;
                    }
                    break;
                }
                chars.next();
                if next == '{' || next == '[' {
                    skip_value(&mut chars)?;
                }
            }
        }
    }
    Ok(keys)
}

fn skip_value(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) -> Result<(), PayloadError> {
    let mut depth = 1usize;
    let mut in_string = false;
    while depth > 0 {
        let Some((_, character)) = chars.next() else {
            return Err(PayloadError::Encoding);
        };
        if in_string {
            if character == '\\' {
                let _ = chars.next();
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_and_payload_changes_create_different_artefact_hashes() {
        let (left, _, left_hash) = encode_plan("# Plan\n", None).expect("left");
        let (_right, _, right_hash) = encode_plan("# Other\n", None).expect("right");
        assert_ne!(left_hash, right_hash);
        let object = ObjectHash::of(&left);
        assert_ne!(object.as_str(), left_hash.as_str());
        let review = encode_review(
            CandidateHash::of(b"tree"),
            ReviewVerdict::Approved,
            "# Plan\n",
            None,
        )
        .expect("review");
        assert_ne!(review.2, left_hash);
        assert_ne!(review.1, ObjectHash::of(&left));
    }

    #[test]
    fn stable_order_creates_the_same_hash() {
        let (_, _, first) = encode_plan("Hello\n", None).expect("first");
        let (_, _, second) = encode_plan("Hello\r\n", None).expect("second");
        assert_eq!(first, second);
        let (bytes, _, _) = encode_plan("caf\u{e9}", None).expect("unicode");
        assert!(
            String::from_utf8(bytes)
                .expect("utf8")
                .contains("caf\u{e9}")
        );
    }

    #[test]
    fn payload_bounds_and_nul_text_are_rejected() {
        assert_eq!(
            encode_plan("bad\u{0000}text", None).err(),
            Some(PayloadError::Text)
        );
        let huge = "a".repeat(MAXIMUM_PLAN_BYTES + 1);
        assert_eq!(encode_plan(&huge, None).err(), Some(PayloadError::Bound));
        assert_eq!(
            encode_plan("use sk-secret", Some("sk-secret")).err(),
            Some(PayloadError::Credential)
        );
        assert!(CandidateHash::parse("sha256:zz").is_none());
    }

    #[test]
    fn duplicate_fields_are_rejected() {
        let bytes = br#"{"format-version":1,"markdown":"a","markdown":"b"}"#;
        assert_eq!(
            parse_typed_payload(ArtefactKind::Plan, bytes).err(),
            Some(PayloadError::DuplicateField)
        );
    }
}
