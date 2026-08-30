use super::definition::{ArtefactKind, OutputKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SystemCommandId {
    RepositoryStatus,
    CommitCandidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandSourceEffect {
    ReadOnly,
    Commit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SystemCommandContract {
    pub(crate) id: SystemCommandId,
    pub(crate) source_effect: CommandSourceEffect,
    pub(crate) required_inputs: &'static [ArtefactKind],
    pub(crate) required_outputs: &'static [OutputKind],
}

impl SystemCommandContract {
    pub(crate) fn accepts(&self, inputs: &[ArtefactKind], outputs: &[OutputKind]) -> bool {
        if !kinds_match(outputs, self.required_outputs) {
            return false;
        }
        if self.id == SystemCommandId::CommitCandidate {
            return kinds_match(inputs, self.required_inputs)
                || kinds_match(
                    inputs,
                    &[
                        ArtefactKind::CandidateRevision,
                        ArtefactKind::ReviewReport,
                        ArtefactKind::HumanDecision,
                    ],
                );
        }
        kinds_match(inputs, self.required_inputs)
    }
}

impl SystemCommandId {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "repository-status" => Some(Self::RepositoryStatus),
            "commit-candidate" => Some(Self::CommitCandidate),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RepositoryStatus => "repository-status",
            Self::CommitCandidate => "commit-candidate",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::RepositoryStatus => "Repository status",
            Self::CommitCandidate => "Commit candidate",
        }
    }

    pub(crate) fn consequence(self) -> &'static str {
        match self {
            Self::RepositoryStatus => "",
            Self::CommitCandidate => {
                "Applies an approved candidate to the local project and creates a Git commit."
            }
        }
    }

    pub(crate) fn contract(self) -> SystemCommandContract {
        match self {
            Self::RepositoryStatus => SystemCommandContract {
                id: self,
                source_effect: CommandSourceEffect::ReadOnly,
                required_inputs: &[ArtefactKind::CandidateRevision],
                required_outputs: &[],
            },
            Self::CommitCandidate => SystemCommandContract {
                id: self,
                source_effect: CommandSourceEffect::Commit,
                required_inputs: &[ArtefactKind::CandidateRevision, ArtefactKind::ReviewReport],
                required_outputs: &[OutputKind::CandidateRevision],
            },
        }
    }

    pub(crate) fn all() -> [Self; 2] {
        [Self::RepositoryStatus, Self::CommitCandidate]
    }
}

pub(crate) fn kinds_match<T: Copy + Eq>(declared: &[T], required: &[T]) -> bool {
    if declared.len() != required.len() {
        return false;
    }
    let mut remaining: Vec<T> = required.to_vec();
    for item in declared {
        if let Some(index) = remaining.iter().position(|required| required == item) {
            remaining.remove(index);
        } else {
            return false;
        }
    }
    remaining.is_empty()
}

#[cfg(test)]
mod tests;
