use super::*;

#[test]
fn file_read_rejects_links_and_bounds_output_without_shell_interpolation() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("tasks.md"), "private").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("tasks.md"), root.path().join("link.md"))
        .unwrap();
    let read = |path: &str| {
        std::process::Command::new("sh")
            .current_dir(root.path())
            .args(["-c", IMPORT_SCRIPT, "task-import", path])
            .arg(root.path())
            .output()
            .unwrap()
    };
    for path in ["linked/tasks.md", "link.md", "missing.md"] {
        let result = read(path);
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
    }
    let filename = "$(touch injected).md";
    std::fs::write(root.path().join(filename), "# Tasks\n- [ ] Read one file\n").unwrap();
    let result = read(filename);
    assert!(result.status.success());
    assert!(!root.path().join("injected").exists());
    std::fs::write(root.path().join("large.md"), vec![b'x'; 70_000]).unwrap();
    let result = read("large.md");
    let encoded: Vec<_> = result
        .stdout
        .into_iter()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap()
            .len(),
        65_537
    );
    for path in ["../tasks.md", "/etc/passwd", "a/../../tasks.md", "a//b"] {
        assert!(!task_list::valid_project_path(path));
    }
}

#[tokio::test]
async fn import_requires_exact_live_directory_consent() {
    let state = super::super::tests::test_state();
    let token = super::super::tests::connected(&state);
    let session = super::super::tests::session_id(&token);
    let mut record = state.conversations.create("Import".to_owned()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut grant = crate::execution::DirectoryGrant::from_selected(root.path(), &[]).unwrap();
    grant.access = crate::execution::DirectoryAccess::ReviewBeforeApply;
    let settings = crate::execution::ExecutionSettings::new(
        crate::providers::ModelSelection::new(
            crate::providers::ProviderKind::Xai,
            "grok-4.6".to_owned(),
            None,
        )
        .unwrap(),
        String::new(),
        Vec::new(),
        crate::tests::test_environment_id(),
    )
    .unwrap()
    .with_directories(vec![grant.clone()])
    .unwrap();
    record.model = Some(ConversationModelConfiguration {
        settings: settings.clone(),
        preset: None,
    });
    let id = grant.id.as_hex();
    assert!(import_grant(&state, session, &record, &id).is_err());
    let request = state
        .access_consent
        .request_conversation(session, record.id, &settings, &grant)
        .unwrap();
    state
        .access_consent
        .approve_conversation(&request, session, record.id, &settings, &grant)
        .unwrap();
    assert!(import_grant(&state, session, &record, &id).is_ok());
    let other = super::super::tests::connected(&state);
    assert!(
        import_grant(
            &state,
            super::super::tests::session_id(&other),
            &record,
            &id
        )
        .is_err()
    );
    state.access_consent.invalidate_conversation(record.id);
    assert!(import_grant(&state, session, &record, &id).is_err());
    record.model.as_mut().unwrap().settings.directories[0].access =
        crate::execution::DirectoryAccess::ReadOnly;
    assert!(import_grant(&state, session, &record, &id).is_ok());
    let old = root.path().with_extension("old-import");
    std::fs::rename(root.path(), &old).unwrap();
    std::fs::create_dir(root.path()).unwrap();
    assert!(import_grant(&state, session, &record, &id).is_err());
    std::fs::remove_dir(old).unwrap();
}
