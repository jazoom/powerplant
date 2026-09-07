use super::{ApplyJournals, ApplyRoot, ApplyRootOutcome, ApplyTransaction, ApplyTransactionState};
use crate::execution::{CanonicalDirectoryIdentity, DirectoryGrantId};
use crate::workflows::artefacts::{ArtefactHash, ArtefactReference, CandidateHash};
use crate::workflows::definition::ArtefactKind;
use crate::workflows::{ArtefactId, AttemptId, RunId};

use crate::workflows::artefacts::{
    CandidateCapture, WorkflowArtefactRepository, candidate::CandidateSetArtefact,
};

struct ApplicationFixture {
    _root: tempfile::TempDir,
    store: WorkflowArtefactRepository,
    baseline: CandidateSetArtefact,
    candidate: CandidateSetArtefact,
    transaction: ApplyTransaction,
    _journals: ApplyJournals,
    journal: super::ApplyJournal,
}

impl ApplicationFixture {
    fn new(second_changes: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = WorkflowArtefactRepository::in_memory();
        let mut grants = Vec::new();
        for alias in ["first", "second"] {
            let path = root.path().join(alias);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("same.txt"), alias).unwrap();
            let mut grant =
                crate::execution::DirectoryGrant::from_selected(&path, &grants).unwrap();
            grant.access = crate::execution::DirectoryAccess::ReviewBeforeApply;
            grants.push(grant);
        }
        let baseline =
            CandidateCapture::capture_set(&grants, &root.path().join("data"), &store).unwrap();
        for (index, grant) in grants.iter().enumerate() {
            if index == 0 || second_changes {
                std::fs::write(
                    grant.host_path.join("same.txt"),
                    format!("new {}", grant.alias),
                )
                .unwrap();
            }
        }
        let candidate =
            CandidateCapture::capture_set(&grants, &root.path().join("data"), &store).unwrap();
        for grant in &grants {
            std::fs::write(grant.host_path.join("same.txt"), &grant.alias).unwrap();
        }
        let mut transaction = transaction();
        transaction.roots = grants
            .iter()
            .zip(&baseline.roots)
            .zip(&candidate.roots)
            .map(|((grant, before), after)| ApplyRoot {
                grant_id: grant.id,
                alias: grant.alias.clone(),
                host_path: grant.host_path.clone(),
                identity: grant.identity,
                baseline_candidate: before.candidate.candidate_hash,
                candidate_hash: after.candidate.candidate_hash,
                exclusions: before.candidate.exclusions.clone(),
                outcome: if before.candidate == after.candidate {
                    ApplyRootOutcome::Unchanged
                } else {
                    ApplyRootOutcome::Pending
                },
            })
            .collect();
        let manifest = baseline.manifest_bytes().unwrap();
        transaction.baseline.artefact_hash = crate::workflows::artefacts::artefact_hash_for(
            ArtefactKind::CandidateRevision,
            1,
            &manifest,
        );
        let journals = ApplyJournals::in_memory();
        let journal = journals
            .create(
                RunId::generate().unwrap(),
                AttemptId::generate().unwrap(),
                &transaction,
                &manifest,
                transaction.baseline.artefact_hash,
            )
            .unwrap();
        Self {
            _root: root,
            store,
            baseline,
            candidate,
            transaction,
            _journals: journals,
            journal,
        }
    }

    fn path(&self, index: usize) -> std::path::PathBuf {
        self.transaction.roots[index].host_path.join("same.txt")
    }
}

#[test]
fn stale_later_root_blocks_the_entire_application_and_retains_its_outcome() {
    let mut fixture = ApplicationFixture::new(true);
    std::fs::write(fixture.path(1), b"external").unwrap();
    let mut recorded = fixture.transaction.clone();
    let result = fixture.transaction.apply_roots(
        &fixture.baseline,
        &fixture.candidate,
        &fixture.store,
        &fixture.journal,
        |next| {
            assert!(recorded.can_advance_to(next));
            recorded = next.clone();
            Ok(())
        },
    );
    assert_eq!(result, Err(super::ApplyExecutionError::Conflict));
    assert_eq!(std::fs::read(fixture.path(0)).unwrap(), b"first");
    assert_eq!(std::fs::read(fixture.path(1)).unwrap(), b"external");
    assert_eq!(recorded.roots[1].outcome, ApplyRootOutcome::Conflicted);
    assert!(recorded.is_settled());
    assert!(!recorded.is_verified());
    assert!(
        fixture
            .journal
            .started_paths(&["first/same.txt".to_owned(), "second/same.txt".to_owned()])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn later_root_drift_retains_the_earlier_write_and_never_overwrites_external_edits() {
    let mut fixture = ApplicationFixture::new(true);
    let second = fixture.path(1);
    let mut recorded = fixture.transaction.clone();
    let result = fixture.transaction.apply_roots(
        &fixture.baseline,
        &fixture.candidate,
        &fixture.store,
        &fixture.journal,
        |next| {
            assert!(recorded.can_advance_to(next));
            recorded = next.clone();
            if next.roots[0].outcome == ApplyRootOutcome::Applied {
                std::fs::write(&second, b"external").unwrap();
            }
            Ok(())
        },
    );
    assert_eq!(result, Err(super::ApplyExecutionError::Conflict));
    let started = fixture
        .journal
        .started_paths(&["first/same.txt".to_owned(), "second/same.txt".to_owned()])
        .unwrap();
    assert_eq!(started, ["first/same.txt"]);
    fixture.transaction.recover_roots(
        &fixture.baseline,
        &fixture.candidate,
        &started,
        &fixture.store,
    );
    assert!(recorded.can_advance_to(&fixture.transaction));
    assert_eq!(
        fixture.transaction.roots[0].outcome,
        ApplyRootOutcome::Applied
    );
    assert_eq!(
        fixture.transaction.roots[1].outcome,
        ApplyRootOutcome::Uncertain
    );
    assert!(!fixture.transaction.is_settled());
    assert_eq!(std::fs::read(fixture.path(0)).unwrap(), b"new first");
    assert_eq!(std::fs::read(fixture.path(1)).unwrap(), b"external");
}

#[test]
fn application_keeps_duplicate_paths_separate_and_recovery_rejects_a_replaced_root() {
    let mut fixture = ApplicationFixture::new(true);
    let mut recorded = fixture.transaction.clone();
    fixture
        .transaction
        .apply_roots(
            &fixture.baseline,
            &fixture.candidate,
            &fixture.store,
            &fixture.journal,
            |next| {
                assert!(recorded.can_advance_to(next));
                recorded = next.clone();
                Ok(())
            },
        )
        .unwrap();
    assert!(recorded.is_verified());
    assert_eq!(std::fs::read(fixture.path(0)).unwrap(), b"new first");
    assert_eq!(std::fs::read(fixture.path(1)).unwrap(), b"new second");
    let operations = ["first/same.txt".to_owned(), "second/same.txt".to_owned()];
    assert_eq!(
        fixture.journal.started_paths(&operations).unwrap(),
        operations
    );

    let root = &fixture.transaction.roots[1].host_path;
    std::fs::rename(root, root.with_extension("original")).unwrap();
    std::fs::create_dir(root).unwrap();
    std::fs::write(root.join("same.txt"), b"replacement").unwrap();
    fixture.transaction.recover_roots(
        &fixture.baseline,
        &fixture.candidate,
        &operations,
        &fixture.store,
    );
    assert!(recorded.can_advance_to(&fixture.transaction));
    assert_eq!(
        fixture.transaction.roots[1].outcome,
        ApplyRootOutcome::Uncertain
    );
    assert!(!fixture.transaction.is_settled());
    assert_eq!(std::fs::read(fixture.path(1)).unwrap(), b"replacement");
}

#[test]
fn recovery_settles_a_known_partial_result_without_claiming_completion() {
    let mut fixture = ApplicationFixture::new(true);
    std::fs::write(fixture.path(0), b"new first").unwrap();
    fixture.transaction.state = ApplyTransactionState::Applying {
        completed: 0,
        path: "first/same.txt".to_owned(),
    };
    let previous = fixture.transaction.clone();
    fixture.transaction.recover_roots(
        &fixture.baseline,
        &fixture.candidate,
        &["first/same.txt".to_owned()],
        &fixture.store,
    );
    assert!(previous.can_advance_to(&fixture.transaction));
    assert_eq!(fixture.transaction.state, ApplyTransactionState::Recovered);
    assert_eq!(
        fixture.transaction.roots[0].outcome,
        ApplyRootOutcome::Applied
    );
    assert_eq!(
        fixture.transaction.roots[1].outcome,
        ApplyRootOutcome::Unchanged
    );
    assert!(!fixture.transaction.is_verified());
}

#[test]
fn recovery_never_attributes_an_unstarted_root_to_the_transaction() {
    let mut fixture = ApplicationFixture::new(true);
    std::fs::write(fixture.path(0), b"new first").unwrap();
    std::fs::write(fixture.path(1), b"new second").unwrap();
    fixture.transaction.recover_roots(
        &fixture.baseline,
        &fixture.candidate,
        &["first/same.txt".to_owned()],
        &fixture.store,
    );
    assert_eq!(
        fixture.transaction.roots[0].outcome,
        ApplyRootOutcome::Applied
    );
    assert_eq!(
        fixture.transaction.roots[1].outcome,
        ApplyRootOutcome::Uncertain
    );
    assert!(!fixture.transaction.is_verified());
    assert_eq!(std::fs::read(fixture.path(1)).unwrap(), b"new second");
}

#[test]
fn recovery_keeps_unchanged_roots_unchanged_after_complete_application() {
    let mut fixture = ApplicationFixture::new(false);
    let mut recorded = fixture.transaction.clone();
    fixture
        .transaction
        .apply_roots(
            &fixture.baseline,
            &fixture.candidate,
            &fixture.store,
            &fixture.journal,
            |next| {
                assert!(recorded.can_advance_to(next));
                recorded = next.clone();
                Ok(())
            },
        )
        .unwrap();
    fixture.transaction.recover_roots(
        &fixture.baseline,
        &fixture.candidate,
        &["first/same.txt".to_owned()],
        &fixture.store,
    );
    assert!(recorded.can_advance_to(&fixture.transaction));
    assert!(fixture.transaction.is_verified());
    assert_eq!(
        fixture.transaction.roots[1].outcome,
        ApplyRootOutcome::Unchanged
    );
    assert_eq!(std::fs::read(fixture.path(0)).unwrap(), b"new first");
    assert_eq!(std::fs::read(fixture.path(1)).unwrap(), b"second");
}

fn reference(byte: u8, kind: ArtefactKind) -> ArtefactReference {
    ArtefactReference {
        id: ArtefactId::generate().expect("artefact"),
        kind,
        artefact_hash: ArtefactHash::of(b"test", &[byte]),
    }
}

fn transaction() -> ApplyTransaction {
    ApplyTransaction {
        state: ApplyTransactionState::Prepared,
        roots: vec![ApplyRoot {
            grant_id: DirectoryGrantId::generate().expect("grant"),
            alias: "root".to_owned(),
            host_path: std::path::PathBuf::from("/tmp/root"),
            identity: CanonicalDirectoryIdentity {
                device: 1,
                inode: 2,
            },
            baseline_candidate: CandidateHash::of(b"baseline"),
            candidate_hash: CandidateHash::of(b"candidate"),
            exclusions: vec!["engine".to_owned()],
            outcome: ApplyRootOutcome::Pending,
        }],
        baseline: reference(1, ArtefactKind::CandidateRevision),
        candidate: reference(2, ArtefactKind::CandidateRevision),
        approval: reference(3, ArtefactKind::HumanDecision),
    }
}

#[test]
fn transaction_progress_cannot_change_approval_or_destination() {
    let prepared = transaction();
    let mut applying = prepared.clone();
    applying.state = ApplyTransactionState::Applying {
        completed: 0,
        path: "file.txt".to_owned(),
    };
    assert!(prepared.can_advance_to(&applying));

    let mut tampered = applying.clone();
    tampered.roots[0].identity.inode += 1;
    assert!(!applying.can_advance_to(&tampered));
    tampered = applying.clone();
    tampered.approval = reference(4, ArtefactKind::HumanDecision);
    assert!(!applying.can_advance_to(&tampered));

    let mut applied = applying.clone();
    applied.state = ApplyTransactionState::Applied { completed: 1 };
    assert!(applying.can_advance_to(&applied));
}

#[test]
fn journal_retains_the_exact_baseline_and_progress() {
    let journals = ApplyJournals::in_memory();
    let run = RunId::generate().expect("run");
    let attempt = AttemptId::generate().expect("attempt");
    let hash = ArtefactHash::of(b"candidate", b"baseline");
    let transaction = transaction();
    let journal = journals
        .create(run, attempt, &transaction, b"baseline", hash)
        .expect("journal");
    journal.make_sure_binding(&transaction).expect("binding");
    assert!(
        journals
            .create(run, attempt, &transaction, b"baseline", hash)
            .is_err()
    );
    let mut replaced = transaction.clone();
    replaced.roots[0].identity.inode += 1;
    assert!(journal.make_sure_binding(&replaced).is_err());
    journal
        .make_sure_baseline(b"baseline", hash)
        .expect("baseline");
    journal
        .record_progress(0, "a/file.txt", false)
        .expect("next");
    journal
        .record_progress(1, "a/file.txt", true)
        .expect("applied");
    let operations = vec!["a/file.txt".to_owned(), "b.txt".to_owned()];
    assert_eq!(journal.started_paths(&operations).unwrap(), operations[..1]);
    journal
        .record_progress(1, "b.txt", false)
        .expect("interrupted write");
    assert_eq!(journal.started_paths(&operations).unwrap(), operations);
    journal
        .record_progress(3, "b.txt", true)
        .expect("invalid progress");
    assert!(journal.started_paths(&operations).is_err());
    assert!(journal.make_sure_baseline(b"changed", hash).is_err());
    journals.remove(run, attempt).expect("remove");
    assert!(journals.load(run, attempt).is_err());
}
