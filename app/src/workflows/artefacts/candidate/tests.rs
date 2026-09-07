use super::*;

impl CandidateCapture {
    pub(crate) fn capture_worktree(
        worktree: &Path,
        git_dir: &Path,
        expected_git: &GitAdministrativeFingerprint,
        store: &WorkflowArtefactRepository,
    ) -> Result<CandidateRevisionArtefact, CaptureError> {
        capture_twice(worktree, git_dir, Some(expected_git), store)
    }
}

fn git_init(dir: &Path) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .expect("git")
            .success()
    );
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "dev@example.com"])
        .current_dir(dir)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Dev"])
        .current_dir(dir)
        .status();
}

#[test]
fn inspect_supported_worktree_requires_a_real_git_directory() {
    let dir = tempfile::tempdir().expect("dir");
    assert_eq!(
        inspect_supported_worktree(dir.path()).err(),
        Some(CaptureError::SourceNotGit)
    );
    std::fs::write(dir.path().join(".git"), b"gitdir: /tmp/other").expect("gitfile");
    assert_eq!(
        inspect_supported_worktree(dir.path()).err(),
        Some(CaptureError::SourceUnsupported)
    );
    std::fs::remove_file(dir.path().join(".git")).expect("remove gitfile");
    std::fs::create_dir(dir.path().join(".git")).expect("git directory");
    std::fs::write(dir.path().join(".git/HEAD"), b"ref: refs/heads/main\n").expect("head");
    std::fs::write(
        dir.path().join(".git/config"),
        b"[core]\nrepositoryformatversion = 0\n",
    )
    .expect("config");
    assert_eq!(
        inspect_supported_worktree(dir.path()).err(),
        Some(CaptureError::SourceNotGit)
    );
    std::fs::remove_dir_all(dir.path().join(".git")).expect("remove invalid repository");
    git_init(dir.path());
    inspect_supported_worktree(dir.path()).expect("supported");
    let config = dir.path().join(".git/config");
    let mut text = std::fs::read_to_string(&config).expect("config");
    text.push_str("\n[include]\n\tpath = /tmp/other\n");
    std::fs::write(&config, text).expect("include");
    assert_eq!(
        inspect_supported_worktree(dir.path()).err(),
        Some(CaptureError::SourceUnsupported)
    );
}

#[test]
fn inspect_supported_worktree_rejects_unmerged_entries() {
    let dir = tempfile::tempdir().expect("dir");
    git_init(dir.path());
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("git")
    };
    std::fs::write(dir.path().join("conflict.txt"), b"base\n").expect("base");
    assert!(run(&["add", "conflict.txt"]).status.success());
    assert!(run(&["commit", "-qm", "base"]).status.success());
    assert!(run(&["checkout", "-qb", "other"]).status.success());
    std::fs::write(dir.path().join("conflict.txt"), b"other\n").expect("other");
    assert!(run(&["commit", "-qam", "other"]).status.success());
    assert!(
        run(&["checkout", "-q", "--detach", "HEAD~1"])
            .status
            .success()
    );
    std::fs::write(dir.path().join("conflict.txt"), b"main\n").expect("main");
    assert!(run(&["commit", "-qam", "main"]).status.success());
    assert!(!run(&["merge", "other"]).status.success());
    assert_eq!(
        inspect_supported_worktree(dir.path()).err(),
        Some(CaptureError::SourceUnsupported)
    );
}

#[cfg(unix)]
#[test]
fn inspect_supported_worktree_rejects_a_git_symlink() {
    let dir = tempfile::tempdir().expect("dir");
    let target = tempfile::tempdir().expect("target");
    git_init(target.path());
    std::os::unix::fs::symlink(target.path().join(".git"), dir.path().join(".git"))
        .expect("symlink");
    assert_eq!(
        inspect_supported_worktree(dir.path()).err(),
        Some(CaptureError::SourceUnsupported)
    );
}

#[test]
fn path_kind_mode_and_blob_changes_create_different_hashes() {
    let left = vec![CandidateEntry {
        path: "a.txt".to_owned(),
        kind: CandidateEntryKind::Regular {
            executable: false,
            mode: 0o644,
            bytes: 1,
            blob: ObjectHash::of(b"a"),
        },
    }];
    let right = vec![CandidateEntry {
        path: "a.txt".to_owned(),
        kind: CandidateEntryKind::Regular {
            executable: true,
            mode: 0o755,
            bytes: 1,
            blob: ObjectHash::of(b"a"),
        },
    }];
    assert_ne!(hash_entries(&left), hash_entries(&right));
    let renamed = vec![CandidateEntry {
        path: "b.txt".to_owned(),
        kind: left[0].kind.clone(),
    }];
    assert_ne!(hash_entries(&left), hash_entries(&renamed));
}

#[test]
fn capture_includes_tracked_and_permitted_untracked_files() {
    let dir = tempfile::tempdir().expect("dir");
    git_init(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), b"one").expect("write");
    assert!(
        std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(dir.path())
            .status()
            .expect("add")
            .success()
    );
    std::fs::write(dir.path().join("loose.txt"), b"two").expect("loose");
    std::fs::write(dir.path().join(".gitignore"), b"ignored.txt\n").expect("ignore");
    std::fs::write(dir.path().join("ignored.txt"), b"nope").expect("ignored");
    let store = WorkflowArtefactRepository::in_memory();
    let candidate = CandidateCapture::capture_host(dir.path(), &store).expect("capture");
    let paths: Vec<_> = candidate
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert!(paths.contains(&"tracked.txt"));
    assert!(paths.contains(&"loose.txt"));
    assert!(paths.contains(&".gitignore"));
    assert!(!paths.contains(&"ignored.txt"));
    let again = hash_entries(&candidate.entries);
    assert_eq!(again, candidate.candidate_hash);
}

#[test]
fn capture_disables_external_exclude_files() {
    let dir = tempfile::tempdir().expect("dir");
    let external = tempfile::NamedTempFile::new().expect("exclude");
    std::fs::write(external.path(), b"external.txt\n").expect("pattern");
    git_init(dir.path());
    assert!(
        std::process::Command::new("git")
            .args(["config", "core.excludesFile"])
            .arg(external.path())
            .current_dir(dir.path())
            .status()
            .expect("config")
            .success()
    );
    std::fs::write(dir.path().join("external.txt"), b"capture").expect("file");
    let store = WorkflowArtefactRepository::in_memory();

    let candidate = CandidateCapture::capture_host(dir.path(), &store).expect("capture");

    assert!(
        candidate
            .entries
            .iter()
            .any(|entry| entry.path == "external.txt")
    );
}

#[test]
fn gitlink_placeholders_reject_kind_and_content_changes() {
    let dir = tempfile::tempdir().expect("dir");
    git_init(dir.path());
    let commit = "1".repeat(40);
    assert!(
        std::process::Command::new("git")
            .args(["update-index", "--add", "--cacheinfo"])
            .arg(format!("160000,{commit},module"))
            .current_dir(dir.path())
            .status()
            .expect("index")
            .success()
    );
    let store = WorkflowArtefactRepository::in_memory();
    let module = dir.path().join("module");
    std::fs::create_dir(&module).expect("placeholder");
    let captured = CandidateCapture::capture_host(dir.path(), &store).expect("gitlink");
    assert!(matches!(
        captured.entries[0].kind,
        CandidateEntryKind::Gitlink { .. }
    ));

    std::fs::remove_dir(&module).expect("remove placeholder");
    std::fs::write(&module, b"file").expect("replace file");
    assert_eq!(
        CandidateCapture::capture_host(dir.path(), &store).err(),
        Some(CaptureError::SourceUnsupported)
    );
    std::fs::remove_file(&module).expect("remove file");
    std::os::unix::fs::symlink("target", &module).expect("replace link");
    assert_eq!(
        CandidateCapture::capture_host(dir.path(), &store).err(),
        Some(CaptureError::SourceUnsupported)
    );
    std::fs::remove_file(&module).expect("remove link");
    std::fs::create_dir(&module).expect("placeholder");
    std::fs::write(module.join("nested"), b"content").expect("nested");
    assert_eq!(
        CandidateCapture::capture_host(dir.path(), &store).err(),
        Some(CaptureError::SourceUnsupported)
    );
    std::fs::remove_dir_all(&module).expect("remove module");
    let deleted = CandidateCapture::capture_host(dir.path(), &store).expect("deletion");
    assert!(deleted.entries.is_empty());
}

#[cfg(unix)]
#[test]
fn ordinary_capture_keeps_binary_links_modes_empty_directories_and_exclusions() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().expect("root");
    std::fs::write(root.path().join("binary.bin"), [0, 1, 255]).expect("binary");
    std::fs::set_permissions(
        root.path().join("binary.bin"),
        std::fs::Permissions::from_mode(0o640),
    )
    .expect("mode");
    std::fs::create_dir(root.path().join("empty")).expect("empty directory");
    std::fs::set_permissions(
        root.path().join("empty"),
        std::fs::Permissions::from_mode(0o750),
    )
    .expect("directory mode");
    let outside = tempfile::tempdir().expect("outside");
    std::fs::write(outside.path().join("secret"), b"do not capture").expect("secret");
    std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("link"))
        .expect("link");
    std::fs::create_dir(root.path().join("workflow-workspaces")).expect("excluded");
    std::fs::write(root.path().join("workflow-workspaces/live"), b"live").expect("live");
    let store = WorkflowArtefactRepository::in_memory();

    let captured = CandidateCapture::capture_directory(
        root.path(),
        &["workflow-workspaces".to_owned()],
        &store,
    )
    .expect("ordinary capture");

    assert!(captured.ordinary);
    assert!(captured.repository.is_none());
    assert_eq!(captured.exclusions, vec!["workflow-workspaces"]);
    assert!(captured.entries.iter().any(|entry| {
        entry.path == "binary.bin"
            && matches!(entry.kind, CandidateEntryKind::Regular { mode: 0o640, .. })
    }));
    assert!(captured.entries.iter().any(|entry| {
        entry.path == "empty" && matches!(entry.kind, CandidateEntryKind::Directory { mode: 0o750 })
    }));
    assert!(captured.entries.iter().any(|entry| {
        entry.path == "link" && matches!(entry.kind, CandidateEntryKind::Symlink { .. })
    }));
    assert!(
        !captured
            .entries
            .iter()
            .any(|entry| { entry.path.starts_with("workflow-workspaces") })
    );
    assert!(!captured.entries.iter().any(|entry| {
        match &entry.kind {
            CandidateEntryKind::Regular { blob, .. } => store
                .get(blob)
                .ok()
                .is_some_and(|bytes| bytes == b"do not capture"),
            _ => false,
        }
    }));
}

#[cfg(unix)]
#[test]
fn ordinary_capture_rejects_special_files() {
    use std::os::unix::net::UnixListener;

    let root = tempfile::tempdir().expect("root");
    let _socket = UnixListener::bind(root.path().join("socket")).expect("socket");
    let store = WorkflowArtefactRepository::in_memory();
    assert_eq!(
        CandidateCapture::capture_directory(root.path(), &[], &store).err(),
        Some(CaptureError::SourceUnsupported)
    );
}

#[test]
fn ordinary_capture_rejects_oversized_files_and_new_git_administration() {
    let store = WorkflowArtefactRepository::in_memory();
    let root = tempfile::tempdir().expect("root");
    let baseline = CandidateCapture::capture_directory(root.path(), &[], &store).expect("capture");
    std::fs::create_dir(root.path().join(".git")).expect("git directory");
    assert_eq!(
        CandidateCapture::capture_isolated(
            root.path(),
            &baseline,
            &root.path().join(".git"),
            &store
        )
        .err(),
        Some(CaptureError::SourceUnsupported)
    );
    std::fs::remove_dir(root.path().join(".git")).expect("remove git directory");
    let file = std::fs::File::create(root.path().join("large")).expect("file");
    file.set_len(MAXIMUM_FILE_BYTES + 1).expect("size");
    assert!(CandidateCapture::capture_directory(root.path(), &[], &store).is_err());
}

#[test]
fn ordinary_manifest_rejects_entries_inside_exclusions() {
    let store = WorkflowArtefactRepository::in_memory();
    let root = tempfile::tempdir().expect("root");
    std::fs::create_dir(root.path().join("engine")).expect("directory");
    std::fs::write(root.path().join("engine/live"), b"x").expect("file");
    let mut captured =
        CandidateCapture::capture_directory(root.path(), &[], &store).expect("capture");
    captured.exclusions.push("engine".to_owned());
    captured.candidate_hash = hash_candidate(&captured.entries, &captured.exclusions);
    assert!(
        CandidateRevisionArtefact::from_manifest_bytes(
            &captured.manifest_bytes().expect("manifest")
        )
        .is_none()
    );
}

#[test]
fn ordinary_manifest_rejects_an_unbounded_or_inconsistent_mode() {
    let store = WorkflowArtefactRepository::in_memory();
    let root = tempfile::tempdir().expect("root");
    std::fs::write(root.path().join("file"), b"x").expect("file");
    let mut captured =
        CandidateCapture::capture_directory(root.path(), &[], &store).expect("capture");
    let CandidateEntryKind::Regular { mode, .. } = &mut captured.entries[0].kind else {
        panic!("regular file");
    };
    *mode = 0o1777;
    captured.candidate_hash = hash_candidate(&captured.entries, &captured.exclusions);
    let bytes = captured.manifest_bytes().expect("manifest");
    assert!(CandidateRevisionArtefact::from_manifest_bytes(&bytes).is_none());
}

#[test]
fn comparison_reports_additions_and_mode_changes() {
    let blob = ObjectHash::of(b"x");
    let before = vec![CandidateEntry {
        path: "keep.txt".to_owned(),
        kind: CandidateEntryKind::Regular {
            executable: false,
            mode: 0o644,
            bytes: 1,
            blob,
        },
    }];
    let after = vec![
        CandidateEntry {
            path: "keep.txt".to_owned(),
            kind: CandidateEntryKind::Regular {
                executable: true,
                mode: 0o755,
                bytes: 1,
                blob,
            },
        },
        CandidateEntry {
            path: "new.txt".to_owned(),
            kind: CandidateEntryKind::Regular {
                executable: false,
                mode: 0o644,
                bytes: 1,
                blob,
            },
        },
    ];
    let changes = compare_candidates(&before, &after);
    assert!(
        changes
            .iter()
            .any(|item| item.1 == CandidateChange::ModeChanged)
    );
    assert!(changes.iter().any(|item| item.1 == CandidateChange::Added));
}

#[test]
fn git_fingerprint_detects_admin_drift() {
    let dir = tempfile::tempdir().expect("dir");
    git_init(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), b"one").expect("write");
    assert!(
        std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(dir.path())
            .status()
            .expect("add")
            .success()
    );
    let store = WorkflowArtefactRepository::in_memory();
    let first = CandidateCapture::capture_host(dir.path(), &store).expect("capture");
    let first_git = first.git_admin.clone().expect("git admin");
    let git = dir.path().join(".git");
    let original_head = std::fs::read(git.join("HEAD")).expect("head");
    std::fs::write(git.join("HEAD"), b"ref: refs/heads/other\n").expect("head");
    assert_ne!(git_fingerprint(&git).expect("fp"), first_git);
    std::fs::write(git.join("HEAD"), original_head).expect("restore");
    let original_exclude = std::fs::read(git.join("info/exclude")).unwrap_or_default();
    std::fs::create_dir_all(git.join("info")).expect("info");
    std::fs::write(git.join("info/exclude"), b"secret\n").expect("exclude");
    assert_ne!(git_fingerprint(&git).expect("fp"), first_git);
    std::fs::write(git.join("info/exclude"), original_exclude).expect("restore exclude");
    let original_config = std::fs::read(git.join("config")).expect("config");
    let mut config = original_config.clone();
    config.extend_from_slice(b"\n[user]\n\tname = Drift\n");
    std::fs::write(git.join("config"), config).expect("config");
    assert_ne!(git_fingerprint(&git).expect("fp"), first_git);
    std::fs::write(git.join("config"), original_config).expect("restore config");
    assert_eq!(git_fingerprint(&git).expect("fp"), first_git);
}

#[test]
fn candidate_set_keeps_duplicate_relative_paths_bound_to_distinct_roots() {
    let parent = tempfile::tempdir().expect("parent");
    let first_path = parent.path().join("first");
    let second_path = parent.path().join("second");
    std::fs::create_dir(&first_path).expect("first");
    std::fs::create_dir(&second_path).expect("second");
    std::fs::write(first_path.join("same.txt"), b"first").expect("first file");
    std::fs::write(second_path.join("same.txt"), b"second").expect("second file");
    let mut first =
        crate::execution::DirectoryGrant::from_selected(&first_path, &[]).expect("first grant");
    first.access = crate::execution::DirectoryAccess::ReviewBeforeApply;
    let mut second =
        crate::execution::DirectoryGrant::from_selected(&second_path, std::slice::from_ref(&first))
            .expect("second grant");
    second.access = crate::execution::DirectoryAccess::ReviewBeforeApply;
    let store = WorkflowArtefactRepository::in_memory();
    let set = CandidateCapture::capture_set(&[first, second], &parent.path().join("data"), &store)
        .expect("candidate set");

    assert_eq!(set.roots.len(), 2);
    assert_eq!(
        set.entries()
            .filter(|(_, entry)| entry.path == "same.txt")
            .count(),
        2
    );
    let bytes = set.manifest_bytes().expect("manifest");
    assert_eq!(CandidateSetArtefact::from_manifest_bytes(&bytes), Some(set));
}

#[test]
fn candidate_set_rejects_tampered_root_identity() {
    let parent = tempfile::tempdir().expect("parent");
    let path = parent.path().join("root");
    std::fs::create_dir(&path).expect("root");
    let mut grant = crate::execution::DirectoryGrant::from_selected(&path, &[]).expect("grant");
    grant.access = crate::execution::DirectoryAccess::ReviewBeforeApply;
    let store = WorkflowArtefactRepository::in_memory();
    let set = CandidateCapture::capture_set(&[grant], &parent.path().join("data"), &store)
        .expect("candidate set");
    let mut value: serde_json::Value =
        serde_json::from_slice(&set.manifest_bytes().expect("manifest")).expect("json");
    value["roots"][0]["identity"]["inode"] = serde_json::json!(999_999_u64);
    let bytes = serde_json::to_vec(&value).expect("tampered manifest");

    assert!(CandidateSetArtefact::from_manifest_bytes(&bytes).is_none());

    let mut roots = set.roots.clone();
    roots[0].alias = "../escape".to_owned();
    assert!(CandidateSetArtefact::from_roots(roots).is_err());
    let mut duplicate = set.roots[0].clone();
    duplicate.grant_id = crate::execution::DirectoryGrantId::generate().unwrap();
    duplicate.alias = "another".to_owned();
    assert!(CandidateSetArtefact::from_roots(vec![set.roots[0].clone(), duplicate]).is_err());
}

#[test]
fn capture_does_not_follow_a_workspace_symlink_to_a_sentinel() {
    let project = tempfile::tempdir().expect("project");
    git_init(project.path());
    std::fs::write(project.path().join("tracked.txt"), b"inside").expect("tracked");
    assert!(
        std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(project.path())
            .status()
            .expect("add")
            .success()
    );
    let store = WorkflowArtefactRepository::in_memory();
    let captured = CandidateCapture::capture_host(project.path(), &store).expect("capture");
    let workspace = tempfile::tempdir().expect("workspace");
    let dest = workspace.path().join("project");
    crate::workflows::artefacts::CandidateMaterialise::into_workspace(
        &dest,
        &captured,
        crate::workflows::artefacts::artefact_hash_for(
            crate::workflows::definition::ArtefactKind::CandidateRevision,
            CANDIDATE_SCHEMA,
            &captured.manifest_bytes().expect("bytes"),
        ),
        &store,
    )
    .expect("materialise");
    let outside = tempfile::tempdir().expect("outside");
    let sentinel = outside.path().join("secret.txt");
    std::fs::write(&sentinel, b"SENTINEL").expect("sentinel");
    std::os::unix::fs::symlink(&sentinel, dest.join("escape")).expect("link");
    let recaptured = CandidateCapture::capture_worktree(
        &dest,
        &project.path().join(".git"),
        captured.git_admin.as_ref().expect("git admin"),
        &store,
    );
    if let Ok(recaptured) = recaptured {
        let leaked = recaptured.entries.iter().any(|entry| match &entry.kind {
            CandidateEntryKind::Regular { blob, .. } => store
                .get(blob)
                .ok()
                .is_some_and(|bytes| bytes == b"SENTINEL"),
            _ => false,
        });
        assert!(!leaked);
    }
}
