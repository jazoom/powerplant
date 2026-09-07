use crate::{
    agents::ToolId,
    providers::{ModelSelection, ProviderKind},
};

use super::ExecutionSettings;

fn model() -> ModelSelection {
    ModelSelection::new(ProviderKind::Xai, "test-model".to_owned(), None).unwrap()
}

#[test]
fn settings_reject_duplicate_tools_and_control_characters() {
    assert!(
        ExecutionSettings::new(
            model(),
            String::new(),
            vec![ToolId::Read, ToolId::Read],
            crate::tests::test_environment_id(),
        )
        .is_none()
    );
    assert!(
        ExecutionSettings::new(
            model(),
            "bad\0text".to_owned(),
            Vec::new(),
            crate::tests::test_environment_id(),
        )
        .is_none()
    );
}

#[test]
fn project_basename_does_not_use_the_reserved_workflow_alias() {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("project");
    std::fs::create_dir(&path).unwrap();
    let grant = super::DirectoryGrant::from_selected(&path, &[]).unwrap();
    assert!(
        crate::workflows::definition::AgentAuthority::new(
            vec![ToolId::Read],
            vec![crate::workflows::definition::GuestDirectoryAccess {
                alias: grant.alias,
                access: grant.access,
            }],
        )
        .is_ok()
    );
}

#[test]
fn duplicate_directory_identity_is_invalid_at_distinct_paths() {
    let parent = tempfile::tempdir().unwrap();
    let first = super::DirectoryGrant::from_selected(parent.path(), &[]).unwrap();
    let mut duplicate = first.clone();
    duplicate.id = super::DirectoryGrantId::generate().unwrap();
    duplicate.alias = "other".to_owned();
    duplicate.host_path = parent.path().with_extension("alias");
    assert!(
        ExecutionSettings::new(
            model(),
            String::new(),
            Vec::new(),
            crate::tests::test_environment_id(),
        )
        .unwrap()
        .with_directories(vec![first, duplicate])
        .is_none()
    );
}

#[test]
fn settings_reject_overlapping_directory_roots() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("root");
    let child = root.join("child");
    std::fs::create_dir_all(&child).unwrap();
    let first = crate::execution::DirectoryGrant::from_selected(&root, &[]).unwrap();

    assert_eq!(
        crate::execution::DirectoryGrant::from_selected(&child, &[first]),
        Err(crate::execution::DirectoryGrantError::Overlap)
    );
}
