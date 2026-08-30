use super::CandidateMaterialise;
use crate::workflows::artefacts::candidate::{
    CANDIDATE_SCHEMA, CandidateCapture, CandidateEntry, CandidateEntryKind,
    CandidateRevisionArtefact, CaptureError, GitObjectFormat, hash_entries,
};
use crate::workflows::artefacts::id::ObjectHash;
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

fn hash_of(artefact: &CandidateRevisionArtefact) -> crate::workflows::artefacts::ArtefactHash {
    let bytes = artefact.manifest_bytes().expect("bytes");
    artefact_hash_for(ArtefactKind::CandidateRevision, CANDIDATE_SCHEMA, &bytes)
}

fn base_artefact(store: &WorkflowArtefactRepository) -> CandidateRevisionArtefact {
    let dir = tempfile::tempdir().expect("git");
    git_init(dir.path());
    CandidateCapture::capture_host(dir.path(), store).expect("capture")
}

fn with_entry(
    mut artefact: CandidateRevisionArtefact,
    entry: CandidateEntry,
) -> CandidateRevisionArtefact {
    artefact.entries.push(entry);
    artefact
        .entries
        .sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    artefact.candidate_hash = hash_entries(&artefact.entries);
    artefact
}

#[test]
fn mixed_manifest_round_trip_keeps_repository_context() {
    let dir = tempfile::tempdir().expect("dir");
    git_init(dir.path());
    std::fs::write(dir.path().join("readme.txt"), b"hi").expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(dir.path().join("tool.sh"), b"#!/bin/sh\n").expect("exec");
        let mut permissions = std::fs::metadata(dir.path().join("tool.sh"))
            .expect("meta")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(dir.path().join("tool.sh"), permissions).expect("mode");
        std::os::unix::fs::symlink("readme.txt", dir.path().join("link")).expect("link");
    }
    assert!(
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir.path())
            .status()
            .expect("add")
            .success()
    );
    assert!(
        std::process::Command::new("git")
            .args(["update-index", "--add", "--cacheinfo"])
            .arg(format!("160000,{},module", "1".repeat(40)))
            .current_dir(dir.path())
            .status()
            .expect("gitlink")
            .success()
    );
    std::fs::create_dir(dir.path().join("module")).expect("placeholder");
    let store = WorkflowArtefactRepository::in_memory();
    let captured = CandidateCapture::capture_host(dir.path(), &store).expect("capture");
    assert!(!captured.git_admin.as_str().is_empty());
    assert_eq!(captured.repository.object_format, GitObjectFormat::Sha1);
    let dest = tempfile::tempdir().expect("dest");
    let project = dest.path().join("project");
    CandidateMaterialise::into_workspace(&project, &captured, hash_of(&captured), &store)
        .expect("materialise");
    let again = CandidateCapture::capture_worktree(
        &project,
        &dir.path().join(".git"),
        &captured.git_admin,
        &store,
    )
    .expect("recapture");
    assert_eq!(again.candidate_hash, captured.candidate_hash);
    assert_eq!(again.repository, captured.repository);
    assert_eq!(again.git_admin, captured.git_admin);
}

#[test]
fn manifest_total_bytes_are_bounded_before_blob_reads() {
    let store = WorkflowArtefactRepository::in_memory();
    let mut artefact = base_artefact(&store);
    let blob = ObjectHash::of(b"absent");
    for index in 0..5 {
        artefact = with_entry(
            artefact,
            CandidateEntry {
                path: format!("{index}.bin"),
                kind: CandidateEntryKind::Regular {
                    executable: false,
                    bytes: super::super::candidate::MAXIMUM_FILE_BYTES,
                    blob,
                },
            },
        );
    }
    let destination = tempfile::tempdir().expect("destination");

    assert_eq!(
        CandidateMaterialise::into_workspace(
            &destination.path().join("project"),
            &artefact,
            hash_of(&artefact),
            &store,
        )
        .err(),
        Some(CaptureError::SourceTooLarge)
    );
}

#[test]
fn materialise_rejection_table() {
    let store = WorkflowArtefactRepository::in_memory();
    let blob = store.publish(b"x").expect("blob");
    let base = base_artefact(&store);
    let dest = tempfile::tempdir().expect("dest");

    let illegal = with_entry(
        base.clone(),
        CandidateEntry {
            path: "../escape".to_owned(),
            kind: CandidateEntryKind::Regular {
                executable: false,
                bytes: 1,
                blob,
            },
        },
    );
    assert_eq!(
        CandidateMaterialise::into_workspace(
            &dest.path().join("a"),
            &illegal,
            hash_of(&illegal),
            &store
        )
        .err(),
        Some(CaptureError::SourceUnsupported)
    );

    let git_path = with_entry(
        base.clone(),
        CandidateEntry {
            path: ".git/config".to_owned(),
            kind: CandidateEntryKind::Regular {
                executable: false,
                bytes: 1,
                blob,
            },
        },
    );
    assert_eq!(
        CandidateMaterialise::into_workspace(
            &dest.path().join("b"),
            &git_path,
            hash_of(&git_path),
            &store
        )
        .err(),
        Some(CaptureError::SourceUnsupported)
    );

    let absolute = with_entry(
        base.clone(),
        CandidateEntry {
            path: "/tmp/x".to_owned(),
            kind: CandidateEntryKind::Regular {
                executable: false,
                bytes: 1,
                blob,
            },
        },
    );
    assert_eq!(
        CandidateMaterialise::into_workspace(
            &dest.path().join("c"),
            &absolute,
            hash_of(&absolute),
            &store
        )
        .err(),
        Some(CaptureError::SourceUnsupported)
    );

    let duplicate = {
        let entry = CandidateEntry {
            path: "a.txt".to_owned(),
            kind: CandidateEntryKind::Regular {
                executable: false,
                bytes: 1,
                blob,
            },
        };
        let mut artefact = with_entry(base.clone(), entry.clone());
        artefact.entries.push(entry);
        artefact.candidate_hash = hash_entries(&artefact.entries);
        artefact
    };
    assert_eq!(
        CandidateMaterialise::into_workspace(
            &dest.path().join("d"),
            &duplicate,
            hash_of(&duplicate),
            &store
        )
        .err(),
        Some(CaptureError::SourceUnsupported)
    );

    let prefix = {
        let mut artefact = with_entry(
            base.clone(),
            CandidateEntry {
                path: "dir".to_owned(),
                kind: CandidateEntryKind::Regular {
                    executable: false,
                    bytes: 1,
                    blob,
                },
            },
        );
        artefact = with_entry(
            artefact,
            CandidateEntry {
                path: "dir/nested.txt".to_owned(),
                kind: CandidateEntryKind::Regular {
                    executable: false,
                    bytes: 1,
                    blob,
                },
            },
        );
        artefact
    };
    assert_eq!(
        CandidateMaterialise::into_workspace(
            &dest.path().join("e"),
            &prefix,
            hash_of(&prefix),
            &store
        )
        .err(),
        Some(CaptureError::SourceUnsupported)
    );

    let mismatch = with_entry(
        base.clone(),
        CandidateEntry {
            path: "bad.txt".to_owned(),
            kind: CandidateEntryKind::Regular {
                executable: false,
                bytes: 1,
                blob: ObjectHash::of(b"other"),
            },
        },
    );
    assert_eq!(
        CandidateMaterialise::into_workspace(
            &dest.path().join("f"),
            &mismatch,
            hash_of(&mismatch),
            &store
        )
        .err(),
        Some(CaptureError::ArtefactIntegrity)
    );
}
