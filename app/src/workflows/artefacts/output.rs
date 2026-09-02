use super::payload::{ReviewVerdict, TestOutcome};
use crate::workflows::definition::{OutputKey, OutputKind, RequiredOutput};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OutputDraft {
    Plan {
        markdown: String,
    },
    Review {
        verdict: ReviewVerdict,
        markdown: String,
    },
    Test {
        outcome: TestOutcome,
        markdown: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutputDraftError {
    UnknownKey,
    Kind,
    CandidateHash,
    HumanDecision,
}

impl OutputDraftError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::UnknownKey => "That output key is not declared.",
            Self::Kind => "Those output fields do not match the declared kind.",
            Self::CandidateHash => "The model cannot supply a candidate hash.",
            Self::HumanDecision => "Agents cannot submit a human decision.",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OutputDrafts {
    drafts: Vec<(OutputKey, OutputDraft)>,
}

impl OutputDrafts {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn submit(
        &mut self,
        outputs: &[RequiredOutput],
        key: &str,
        kind: OutputKind,
        markdown: Option<String>,
        verdict: Option<&str>,
        outcome: Option<&str>,
        candidate: bool,
        human: bool,
    ) -> Result<(), OutputDraftError> {
        if candidate {
            return Err(OutputDraftError::CandidateHash);
        }
        if human {
            return Err(OutputDraftError::HumanDecision);
        }
        let declared = outputs
            .iter()
            .find(|output| output.key.as_str() == key)
            .ok_or(OutputDraftError::UnknownKey)?;
        if declared.kind != kind {
            return Err(OutputDraftError::Kind);
        }
        let draft = match kind {
            OutputKind::Plan => OutputDraft::Plan {
                markdown: markdown.ok_or(OutputDraftError::Kind)?,
            },
            OutputKind::ReviewReport => OutputDraft::Review {
                verdict: parse_verdict(verdict.ok_or(OutputDraftError::Kind)?)?,
                markdown: markdown.ok_or(OutputDraftError::Kind)?,
            },
            OutputKind::TestReport => OutputDraft::Test {
                outcome: parse_outcome(outcome.ok_or(OutputDraftError::Kind)?)?,
                markdown: markdown.ok_or(OutputDraftError::Kind)?,
            },
            OutputKind::AssistantReply
            | OutputKind::CandidateRevision
            | OutputKind::HumanDecision => {
                return Err(OutputDraftError::Kind);
            }
        };
        if let Some(existing) = self
            .drafts
            .iter_mut()
            .find(|(item, _)| item.as_str() == key)
        {
            existing.1 = draft;
        } else {
            self.drafts.push((declared.key.clone(), draft));
        }
        Ok(())
    }

    pub(crate) fn take(&mut self, key: &OutputKey) -> Option<OutputDraft> {
        self.drafts
            .iter()
            .position(|(item, _)| item == key)
            .map(|index| self.drafts.remove(index).1)
    }
}

fn parse_verdict(value: &str) -> Result<ReviewVerdict, OutputDraftError> {
    match value {
        "approved" => Ok(ReviewVerdict::Approved),
        "revision-required" => Ok(ReviewVerdict::RevisionRequired),
        "blocked" => Ok(ReviewVerdict::Blocked),
        _ => Err(OutputDraftError::Kind),
    }
}

fn parse_outcome(value: &str) -> Result<TestOutcome, OutputDraftError> {
    match value {
        "passed" => Ok(TestOutcome::Passed),
        "failed" => Ok(TestOutcome::Failed),
        "not-run" => Ok(TestOutcome::NotRun),
        _ => Err(OutputDraftError::Kind),
    }
}

#[cfg(test)]
mod tests;
