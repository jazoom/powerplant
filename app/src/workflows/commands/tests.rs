use super::{SystemCommandId, kinds_match};
use crate::workflows::definition::{ArtefactKind, OutputKind};

#[test]
fn command_contract_table() {
    let status = SystemCommandId::parse("repository-status").expect("status");
    let status_contract = status.contract();
    assert_eq!(status.as_str(), "repository-status");
    assert_eq!(status.label(), "Repository status");
    assert_eq!(SystemCommandId::all().len(), 2);
    assert_eq!(
        status_contract.required_inputs,
        &[ArtefactKind::CandidateRevision]
    );
    assert!(status_contract.required_outputs.is_empty());
    assert!(matches!(
        status_contract.source_effect,
        super::CommandSourceEffect::ReadOnly
    ));

    let commit = SystemCommandId::parse("commit-candidate").expect("commit");
    let commit_contract = commit.contract();
    assert_eq!(commit.as_str(), "commit-candidate");
    assert_eq!(
        commit_contract.required_inputs,
        &[ArtefactKind::CandidateRevision, ArtefactKind::ReviewReport]
    );
    assert_eq!(
        commit_contract.required_outputs,
        &[OutputKind::CandidateRevision]
    );
    assert!(matches!(
        commit_contract.source_effect,
        super::CommandSourceEffect::Commit
    ));
    assert_eq!(
        commit.consequence(),
        "Applies an approved candidate to the local project and creates a Git commit."
    );

    assert!(SystemCommandId::parse("git-commit").is_none());
    assert!(SystemCommandId::parse("repository-status ").is_none());
    assert!(!kinds_match(
        &[ArtefactKind::ReviewReport],
        &[ArtefactKind::CandidateRevision]
    ));
    assert!(!kinds_match(
        &[
            ArtefactKind::CandidateRevision,
            ArtefactKind::CandidateRevision
        ],
        &[ArtefactKind::CandidateRevision, ArtefactKind::ReviewReport]
    ));
    assert!(kinds_match(
        &[ArtefactKind::ReviewReport, ArtefactKind::CandidateRevision],
        commit_contract.required_inputs
    ));
}
