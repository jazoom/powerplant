use super::{ApplyJournals, ApplyRoot, ApplyTransaction, ApplyTransactionState};
use crate::execution::{CanonicalDirectoryIdentity, DirectoryGrantId};
use crate::workflows::artefacts::{ArtefactHash, ArtefactReference, CandidateHash};
use crate::workflows::definition::ArtefactKind;
use crate::workflows::{ArtefactId, AttemptId, RunId};

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
        root: ApplyRoot {
            grant_id: DirectoryGrantId::generate().expect("grant"),
            host_path: std::path::PathBuf::from("/tmp/root"),
            identity: CanonicalDirectoryIdentity {
                device: 1,
                inode: 2,
            },
        },
        baseline: reference(1, ArtefactKind::CandidateRevision),
        baseline_candidate: CandidateHash::of(b"baseline"),
        candidate: reference(2, ArtefactKind::CandidateRevision),
        candidate_hash: CandidateHash::of(b"candidate"),
        approval: reference(3, ArtefactKind::HumanDecision),
        exclusions: vec!["engine".to_owned()],
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
    tampered.root.identity.inode += 1;
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
    replaced.root.identity.inode += 1;
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
