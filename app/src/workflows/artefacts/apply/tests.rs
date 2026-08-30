use super::{ApplyError, CandidateApply};
use crate::workflows::artefacts::candidate::{
    CandidateCapture, CandidateEntry, CandidateEntryKind, hash_entries,
};
use crate::workflows::artefacts::payload::artefact_hash_for;
use crate::workflows::artefacts::store::WorkflowArtefactRepository;
use crate::workflows::definition::ArtefactKind;

fn git_init(dir: &std::path::Path) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .expect("git")
            .success()
    );
}

fn hash_of(
    artefact: &crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
) -> crate::workflows::artefacts::ArtefactHash {
    let bytes = artefact.manifest_bytes().expect("bytes");
    artefact_hash_for(
        ArtefactKind::CandidateRevision,
        crate::workflows::artefacts::CANDIDATE_SCHEMA,
        &bytes,
    )
}

#[test]
fn mixed_tree_reaches_the_target_and_keeps_ignored_files() {
    let dir = tempfile::tempdir().expect("project");
    git_init(dir.path());
    std::fs::write(dir.path().join("keep.txt"), b"keep").expect("keep");
    std::fs::write(dir.path().join("gone.txt"), b"gone").expect("gone");
    std::fs::write(dir.path().join("ignored.log"), b"noise").expect("ignored");
    std::fs::write(dir.path().join(".gitignore"), b"ignored.log\n").expect("gitignore");
    assert!(
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .status()
            .expect("add")
            .success()
    );
    let store = WorkflowArtefactRepository::in_memory();
    let initial = CandidateCapture::capture_host(dir.path(), &store).expect("initial");
    let added = store.publish(b"new").expect("blob");
    let mut target = initial.clone();
    target.entries.retain(|entry| entry.path != "gone.txt");
    target.entries.push(CandidateEntry {
        path: "added.txt".to_owned(),
        kind: CandidateEntryKind::Regular {
            executable: false,
            bytes: 3,
            blob: added,
        },
    });
    if let Some(entry) = target
        .entries
        .iter_mut()
        .find(|entry| entry.path == "keep.txt")
    {
        let blob = store.publish(b"changed").expect("changed");
        entry.kind = CandidateEntryKind::Regular {
            executable: false,
            bytes: 7,
            blob,
        };
    }
    target
        .entries
        .sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    target.candidate_hash = hash_entries(&target.entries);
    CandidateApply::apply(dir.path(), &initial, &target, hash_of(&target), &store).expect("apply");
    assert_eq!(
        std::fs::read(dir.path().join("keep.txt")).expect("keep"),
        b"changed"
    );
    assert_eq!(
        std::fs::read(dir.path().join("added.txt")).expect("added"),
        b"new"
    );
    assert!(!dir.path().join("gone.txt").exists());
    assert_eq!(
        std::fs::read(dir.path().join("ignored.log")).expect("ignored"),
        b"noise"
    );
    CandidateApply::rollback(dir.path(), &initial, &target, &store).expect("rollback");
    assert_eq!(
        std::fs::read(dir.path().join("keep.txt")).expect("restored"),
        b"keep"
    );
    assert!(dir.path().join("gone.txt").exists());
    assert!(!dir.path().join("added.txt").exists());
    assert_eq!(
        std::fs::read(dir.path().join("ignored.log")).expect("ignored"),
        b"noise"
    );
}

#[test]
fn reconciliation_rejects_source_drift_before_mutation() {
    let dir = tempfile::tempdir().expect("project");
    git_init(dir.path());
    std::fs::write(dir.path().join("file.txt"), b"initial").expect("file");
    assert!(
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .status()
            .expect("add")
            .success()
    );
    let store = WorkflowArtefactRepository::in_memory();
    let initial = CandidateCapture::capture_host(dir.path(), &store).expect("initial");
    let mut target = initial.clone();
    let target_blob = store.publish(b"target").expect("target blob");
    target.entries[0].kind = CandidateEntryKind::Regular {
        executable: false,
        bytes: 6,
        blob: target_blob,
    };
    target.candidate_hash = hash_entries(&target.entries);

    std::fs::write(dir.path().join("file.txt"), b"external").expect("drift");
    let before = std::fs::read(dir.path().join("file.txt")).expect("before");
    assert_eq!(
        CandidateApply::apply(dir.path(), &initial, &target, hash_of(&target), &store).err(),
        Some(ApplyError::Drift)
    );
    assert_eq!(
        std::fs::read(dir.path().join("file.txt")).expect("after"),
        before
    );
}

#[test]
fn reconciliation_rejects_escape_conflicts_and_git_paths() {
    let dir = tempfile::tempdir().expect("project");
    git_init(dir.path());
    std::fs::write(dir.path().join("file.txt"), b"ok").expect("file");
    std::fs::write(dir.path().join(".gitignore"), b"clash.txt\n").expect("ignore");
    assert!(
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .status()
            .expect("add")
            .success()
    );
    let store = WorkflowArtefactRepository::in_memory();
    let initial = CandidateCapture::capture_host(dir.path(), &store).expect("initial");
    let file_index = initial
        .entries
        .iter()
        .position(|entry| entry.path == "file.txt")
        .expect("file entry");
    let blob = match &initial.entries[file_index].kind {
        CandidateEntryKind::Regular { blob, .. } => *blob,
        _ => panic!("file"),
    };

    let mut git_path = initial.clone();
    git_path.entries[file_index].path = ".git/hooks".to_owned();
    git_path.candidate_hash = hash_entries(&git_path.entries);
    assert_eq!(
        CandidateApply::apply(dir.path(), &initial, &git_path, hash_of(&git_path), &store).err(),
        Some(ApplyError::Escape)
    );

    std::fs::write(dir.path().join("clash.txt"), b"host").expect("clash");
    let mut clash = initial.clone();
    clash.entries.push(CandidateEntry {
        path: "clash.txt".to_owned(),
        kind: CandidateEntryKind::Regular {
            executable: false,
            bytes: 2,
            blob,
        },
    });
    clash
        .entries
        .sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    clash.candidate_hash = hash_entries(&clash.entries);
    assert_eq!(
        CandidateApply::apply(dir.path(), &initial, &clash, hash_of(&clash), &store).err(),
        Some(ApplyError::Conflict)
    );
}
