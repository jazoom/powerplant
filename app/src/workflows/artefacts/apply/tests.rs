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
            mode: 0o644,
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
            mode: 0o644,
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
        mode: 0o644,
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
        Some(ApplyError::Integrity)
    );

    std::fs::write(dir.path().join("clash.txt"), b"host").expect("clash");
    let mut clash = initial.clone();
    clash.entries.push(CandidateEntry {
        path: "clash.txt".to_owned(),
        kind: CandidateEntryKind::Regular {
            executable: false,
            mode: 0o644,
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

#[test]
fn ordinary_files_apply_without_git_and_keep_exclusions() {
    let dir = tempfile::tempdir().expect("directory");
    std::fs::write(dir.path().join("file.txt"), b"before").expect("file");
    std::fs::create_dir(dir.path().join("engine")).expect("engine");
    std::fs::write(dir.path().join("engine/live"), b"retained").expect("excluded");
    let store = WorkflowArtefactRepository::in_memory();
    let exclusions = vec!["engine".to_owned()];
    let initial =
        CandidateCapture::capture_directory(dir.path(), &exclusions, &store).expect("initial");
    let mut target = initial.clone();
    let blob = store.publish(b"after").expect("blob");
    target.entries[0].kind = CandidateEntryKind::Regular {
        executable: false,
        mode: 0o644,
        bytes: 5,
        blob,
    };
    target.candidate_hash =
        crate::workflows::artefacts::candidate::hash_candidate(&target.entries, &target.exclusions);

    std::fs::write(dir.path().join("file.txt"), b"external").unwrap();
    assert_eq!(
        CandidateApply::apply_bound(
            dir.path(),
            &initial,
            hash_of(&initial),
            &target,
            hash_of(&target),
            &exclusions,
            &store,
        ),
        Err(ApplyError::Drift)
    );
    assert_eq!(
        std::fs::read(dir.path().join("file.txt")).unwrap(),
        b"external"
    );
    std::fs::write(dir.path().join("file.txt"), b"before").unwrap();
    CandidateApply::apply_bound(
        dir.path(),
        &initial,
        hash_of(&initial),
        &target,
        hash_of(&target),
        &exclusions,
        &store,
    )
    .expect("apply");

    assert_eq!(
        std::fs::read(dir.path().join("file.txt")).unwrap(),
        b"after"
    );
    assert_eq!(
        std::fs::read(dir.path().join("engine/live")).unwrap(),
        b"retained"
    );
    assert!(!dir.path().join(".git").exists());
}

#[test]
fn recovery_restores_only_known_partial_application_states() {
    let dir = tempfile::tempdir().expect("directory");
    std::fs::write(dir.path().join("a.txt"), b"a0").expect("a");
    std::fs::write(dir.path().join("b.txt"), b"b0").expect("b");
    std::fs::write(dir.path().join("unrelated.txt"), b"original").expect("unrelated");
    let store = WorkflowArtefactRepository::in_memory();
    let initial = CandidateCapture::capture_directory(dir.path(), &[], &store).expect("initial");
    let mut target = initial.clone();
    for (path, bytes) in [("a.txt", b"a1".as_slice()), ("b.txt", b"b1".as_slice())] {
        let entry = target
            .entries
            .iter_mut()
            .find(|entry| entry.path == path)
            .expect("entry");
        entry.kind = CandidateEntryKind::Regular {
            executable: false,
            mode: 0o644,
            bytes: 2,
            blob: store.publish(bytes).expect("blob"),
        };
    }
    target.candidate_hash =
        crate::workflows::artefacts::candidate::hash_candidate(&target.entries, &target.exclusions);
    let mut started = Vec::new();
    assert_eq!(
        CandidateApply::apply_journalled(
            dir.path(),
            &initial,
            &target,
            super::CandidateApplicationBinding {
                initial_hash: hash_of(&initial),
                target_hash: hash_of(&target),
                exclusions: &[],
            },
            &store,
            |_, path, applied| {
                if path == "b.txt" {
                    return Err(ApplyError::Write);
                }
                if !applied {
                    started.push(path.to_owned());
                }
                Ok(())
            },
        ),
        Err(ApplyError::Write)
    );
    assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"a1");
    assert_eq!(std::fs::read(dir.path().join("b.txt")).unwrap(), b"b0");
    std::fs::write(dir.path().join("unrelated.txt"), b"external").expect("external edit");
    assert_eq!(
        CandidateApply::recover_to_initial(dir.path(), &initial, &target, &[], &started, &store),
        Err(ApplyError::Conflict)
    );
    assert_eq!(
        std::fs::read(dir.path().join("unrelated.txt")).unwrap(),
        b"external"
    );
    assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"a1");
    std::fs::write(dir.path().join("unrelated.txt"), b"original").expect("restore fixture");
    assert_eq!(
        CandidateApply::recover_to_initial(dir.path(), &initial, &target, &[], &[], &store),
        Err(ApplyError::Conflict)
    );
    CandidateApply::recover_to_initial(dir.path(), &initial, &target, &[], &started, &store)
        .expect("recover");
    assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"a0");
    assert_eq!(std::fs::read(dir.path().join("b.txt")).unwrap(), b"b0");

    std::fs::write(dir.path().join("a.txt"), b"external").expect("external");
    assert_eq!(
        CandidateApply::recover_to_initial(dir.path(), &initial, &target, &[], &started, &store)
            .err(),
        Some(ApplyError::Conflict)
    );
    assert_eq!(
        std::fs::read(dir.path().join("a.txt")).unwrap(),
        b"external"
    );
}

#[test]
fn journalled_directory_removal_tracks_each_entry_without_implicit_pruning() {
    let dir = tempfile::tempdir().expect("directory");
    std::fs::create_dir_all(dir.path().join("parent/child")).unwrap();
    std::fs::write(dir.path().join("parent/child/file.txt"), b"before").unwrap();
    let store = WorkflowArtefactRepository::in_memory();
    let initial = CandidateCapture::capture_directory(dir.path(), &[], &store).unwrap();
    let mut target = initial.clone();
    target.entries.clear();
    target.candidate_hash = crate::workflows::artefacts::candidate::hash_candidate(&[], &[]);
    let mut completed = Vec::new();
    CandidateApply::apply_journalled(
        dir.path(),
        &initial,
        &target,
        super::CandidateApplicationBinding {
            initial_hash: hash_of(&initial),
            target_hash: hash_of(&target),
            exclusions: &[],
        },
        &store,
        |_, path, applied| {
            if applied {
                completed.push(path.to_owned());
            }
            Ok(())
        },
    )
    .expect("apply nested removals");
    assert_eq!(
        completed,
        ["parent/child/file.txt", "parent/child", "parent"]
    );
    assert!(!dir.path().join("parent").exists());
}

#[test]
fn malicious_manifest_cannot_write_inside_a_pinned_exclusion() {
    let dir = tempfile::tempdir().expect("directory");
    std::fs::create_dir(dir.path().join("engine")).expect("engine");
    std::fs::write(dir.path().join("engine/live"), b"host").expect("file");
    let store = WorkflowArtefactRepository::in_memory();
    let exclusions = vec!["engine".to_owned()];
    let initial =
        CandidateCapture::capture_directory(dir.path(), &exclusions, &store).expect("initial");
    let mut target = initial.clone();
    let blob = store.publish(b"attack").expect("blob");
    target.entries.push(CandidateEntry {
        path: "engine/live".to_owned(),
        kind: CandidateEntryKind::Regular {
            executable: false,
            mode: 0o644,
            bytes: 6,
            blob,
        },
    });
    target.candidate_hash =
        crate::workflows::artefacts::candidate::hash_candidate(&target.entries, &target.exclusions);

    assert_eq!(
        CandidateApply::apply_bound(
            dir.path(),
            &initial,
            hash_of(&initial),
            &target,
            hash_of(&target),
            &exclusions,
            &store,
        )
        .err(),
        Some(ApplyError::Integrity)
    );
    assert_eq!(
        std::fs::read(dir.path().join("engine/live")).unwrap(),
        b"host"
    );
}
