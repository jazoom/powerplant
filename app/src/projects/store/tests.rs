use std::path::{Path, PathBuf};

use super::{MAXIMUM_CATALOGUE_BYTES, ProjectStore};
use crate::projects::id::ProjectId;
use crate::projects::record::{MAXIMUM_PROJECTS, ProjectError};

fn write_catalogue(path: &Path, json: &str) {
    std::fs::write(path, json).expect("write");
}

fn project_value(id: &ProjectId, name: &str, host_path: &str, revision: u32) -> serde_json::Value {
    serde_json::json!({
        "id": id.as_hex(),
        "revision": revision,
        "name": name,
        "host-path": host_path,
        "created-at-ms": 1
    })
}

fn catalogue_json(projects: Vec<serde_json::Value>) -> String {
    serde_json::json!({
        "file-version": 1,
        "projects": projects
    })
    .to_string()
}

#[test]
fn missing_file_opens_empty_without_import_or_write() {
    let dir = tempfile::tempdir().expect("dir");
    write_catalogue(
        &dir.path().join("project.json"),
        r#"{"version":1,"path":"/srv/legacy"}"#,
    );
    std::fs::create_dir(dir.path().join("agents")).expect("agents");
    write_catalogue(&dir.path().join("agents").join("agent.json"), "{}");
    let path = dir.path().join("projects.json");
    let store = ProjectStore::open(path.clone()).expect("open");
    assert!(store.list().is_empty());
    assert!(!path.exists());
}

#[test]
fn create_persists_revision_one_and_permits_duplicate_names() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("projects.json");
    let first;
    let second;
    {
        let store = ProjectStore::open(path.clone()).expect("open");
        first = store
            .create("Desk".to_owned(), PathBuf::from("/srv/one"))
            .expect("first");
        second = store
            .create("Desk".to_owned(), PathBuf::from("/srv/two"))
            .expect("second");
        assert_eq!(first.revision, 1);
        assert_eq!(second.revision, 1);
        assert_eq!(first.name, second.name);
        assert_ne!(first.id, second.id);
    }
    let store = ProjectStore::open(path).expect("reopen");
    assert_eq!(
        store.get(&first.id).expect("first").host_path,
        first.host_path
    );
    assert_eq!(store.get(&second.id).expect("second").name, "Desk");
}

#[test]
fn create_rejects_duplicate_canonical_paths() {
    let store = ProjectStore::in_memory();
    store
        .create("One".to_owned(), PathBuf::from("/srv/app"))
        .expect("first");
    assert_eq!(
        store
            .create("Two".to_owned(), PathBuf::from("/srv/app"))
            .err(),
        Some(ProjectError::DuplicatePath)
    );
}

#[test]
fn stored_duplicate_paths_and_identifiers_are_corrupt() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("projects.json");
    let left = ProjectId::generate().expect("left");
    let right = ProjectId::generate().expect("right");
    write_catalogue(
        &path,
        &catalogue_json(vec![
            project_value(&left, "One", "/srv/app", 1),
            project_value(&right, "Two", "/srv/app", 1),
        ]),
    );
    let original = std::fs::read(&path).expect("original");
    assert_eq!(
        ProjectStore::open(path.clone()).err(),
        Some(ProjectError::Corrupt)
    );
    assert_eq!(std::fs::read(&path).expect("unchanged"), original);

    write_catalogue(
        &path,
        &catalogue_json(vec![
            project_value(&left, "One", "/srv/one", 1),
            project_value(&left, "Two", "/srv/two", 1),
        ]),
    );
    let original = std::fs::read(&path).expect("original");
    assert_eq!(
        ProjectStore::open(path.clone()).err(),
        Some(ProjectError::Corrupt)
    );
    assert_eq!(std::fs::read(&path).expect("unchanged"), original);
}

#[test]
fn corrupt_and_unknown_fields_fail_open_without_replacement() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("projects.json");
    write_catalogue(&path, "{");
    let original = std::fs::read(&path).expect("original");
    assert_eq!(
        ProjectStore::open(path.clone()).err(),
        Some(ProjectError::Corrupt)
    );
    assert_eq!(std::fs::read(&path).expect("unchanged"), original);

    write_catalogue(&path, r#"{"file-version":1,"projects":[],"extra":true}"#);
    let original = std::fs::read(&path).expect("original");
    assert_eq!(
        ProjectStore::open(path.clone()).err(),
        Some(ProjectError::Corrupt)
    );
    assert_eq!(std::fs::read(&path).expect("unchanged"), original);
}

#[cfg(unix)]
#[test]
fn catalogue_symlinks_are_rejected_without_changing_the_target() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("dir");
    let target = dir.path().join("target.json");
    let path = dir.path().join("projects.json");
    let original = catalogue_json(Vec::new());
    write_catalogue(&target, &original);
    symlink(&target, &path).expect("symlink");

    assert_eq!(
        ProjectStore::open(path.clone()).err(),
        Some(ProjectError::Corrupt)
    );
    assert_eq!(std::fs::read_to_string(target).expect("target"), original);
    assert!(path.is_symlink());
}

#[test]
fn file_bounds_and_catalogue_limits_are_enforced() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("projects.json");
    write_catalogue(&path, &"x".repeat(MAXIMUM_CATALOGUE_BYTES + 1));
    let original = std::fs::read(&path).expect("original");
    assert_eq!(
        ProjectStore::open(path.clone()).err(),
        Some(ProjectError::Corrupt)
    );
    assert_eq!(std::fs::read(&path).expect("unchanged"), original);

    let mut projects = Vec::new();
    for index in 0..=MAXIMUM_PROJECTS {
        let id = ProjectId::generate().expect("id");
        projects.push(project_value(
            &id,
            "Desk",
            &format!("/srv/project-{index}"),
            1,
        ));
    }
    write_catalogue(&path, &catalogue_json(projects));
    let original = std::fs::read(&path).expect("original");
    assert_eq!(
        ProjectStore::open(path.clone()).err(),
        Some(ProjectError::Corrupt)
    );
    assert_eq!(std::fs::read(&path).expect("unchanged"), original);

    let store = ProjectStore::in_memory();
    for index in 0..MAXIMUM_PROJECTS {
        store
            .create(
                format!("Desk {index}"),
                PathBuf::from(format!("/srv/project-{index}")),
            )
            .expect("create");
    }
    assert_eq!(
        store
            .create("Overflow".to_owned(), PathBuf::from("/srv/overflow"))
            .err(),
        Some(ProjectError::Full)
    );
}
