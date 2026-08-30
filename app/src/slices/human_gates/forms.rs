use crate::workflows::gates::{GateRevision, normalise_revision_note};

#[derive(Clone, Debug)]
pub(super) struct DecisionForm {
    pub(super) revision: GateRevision,
    pub(super) candidate: String,
    pub(super) note: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FormError {
    Invalid,
    Note,
}

impl DecisionForm {
    pub(super) fn parse(
        pairs: Vec<(String, String)>,
        requires_note: bool,
    ) -> Result<Self, FormError> {
        let mut revision = None;
        let mut candidate = None;
        let mut note = None;
        let mut seen = Vec::new();
        for (key, value) in pairs {
            if seen.contains(&key) {
                return Err(FormError::Invalid);
            }
            seen.push(key.clone());
            match key.as_str() {
                "gate-revision" => revision = GateRevision::parse(&value),
                "candidate" => candidate = Some(value),
                "note" if requires_note => note = normalise_revision_note(&value),
                _ => return Err(FormError::Invalid),
            }
        }
        if requires_note && note.is_none() {
            return Err(FormError::Note);
        }
        let candidate = candidate
            .filter(|value| !value.is_empty())
            .ok_or(FormError::Invalid)?;
        Ok(Self {
            revision: revision.ok_or(FormError::Invalid)?,
            candidate,
            note,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DiffQuery {
    pub(super) page: usize,
    pub(super) change: Option<usize>,
    pub(super) line: usize,
}

impl DiffQuery {
    pub(super) fn parse(
        page: Option<&str>,
        change: Option<&str>,
        line: Option<&str>,
    ) -> Option<Self> {
        Some(Self {
            page: parse_number(page.unwrap_or("0"))?,
            change: match change {
                Some(value) => Some(parse_number(value)?),
                None => None,
            },
            line: parse_number(line.unwrap_or("0"))?,
        })
    }
}

fn parse_number(raw: &str) -> Option<usize> {
    if raw.is_empty()
        || raw.len() > 10
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    raw.parse().ok()
}
