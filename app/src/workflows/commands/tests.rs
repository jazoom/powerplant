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

#[test]
fn commit_contract_accepts_review_and_decision_authority() {
    let commit = SystemCommandId::parse("commit-candidate")
        .expect("commit")
        .contract();
    let outputs = [OutputKind::CandidateRevision];
    let candidate = ArtefactKind::CandidateRevision;
    let review = ArtefactKind::ReviewReport;
    let decision = ArtefactKind::HumanDecision;

    assert!(commit.accepts(&[candidate, review], &outputs));
    assert!(commit.accepts(&[candidate, review, review], &outputs));
    assert!(commit.accepts(&[candidate, decision], &outputs));
    assert!(commit.accepts(&[candidate, review, decision], &outputs));
    assert!(commit.accepts(&[candidate, review, review, decision], &outputs));

    assert!(!commit.accepts(&[candidate], &outputs));
    assert!(!commit.accepts(&[review], &outputs));
    assert!(!commit.accepts(&[decision], &outputs));
    assert!(!commit.accepts(&[candidate, candidate, review], &outputs));
    assert!(!commit.accepts(&[candidate, decision, decision], &outputs));
    assert!(!commit.accepts(&[candidate, ArtefactKind::Plan], &outputs));
    assert!(!commit.accepts(&[candidate, ArtefactKind::TestReport], &outputs));
    assert!(!commit.accepts(&[candidate, review, ArtefactKind::Plan], &outputs));
    assert!(!commit.accepts(&[], &outputs));
}
