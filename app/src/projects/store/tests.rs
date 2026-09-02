use super::*;

impl super::ProjectStore {
    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            inner: Mutex::new(BTreeMap::new()),
        }
    }
}

use std::path::Path;

use super::{MAXIMUM_CATALOGUE_BYTES, ProjectStore};
use crate::projects::id::ProjectId;
use crate::projects::record::{MAXIMUM_PROJECTS, ProjectError};

fn git_init(dir: &Path) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .expect("git")
            .success()
    );
}

fn git_worktree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("dir");
    git_init(dir.path());
    dir
}

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
    let one = git_worktree();
    let two = git_worktree();
    let first;
    let second;
    {
        let store = ProjectStore::open(path.clone()).expect("open");
        first = store
            .create("Desk".to_owned(), one.path().to_path_buf())
            .expect("first");
        second = store
            .create("Desk".to_owned(), two.path().to_path_buf())
            .expect("second");
        assert_eq!(first.revision, 1);
        assert_eq!(second.revision, 1);
        assert_eq!(first.name, second.name);
        assert_ne!(first.id, second.id);
        assert_eq!(
            first.host_path,
            one.path().canonicalize().expect("canonical")
        );
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
    let dir = git_worktree();
    store
        .create("One".to_owned(), dir.path().to_path_buf())
        .expect("first");
    assert_eq!(
        store
            .create("Two".to_owned(), dir.path().to_path_buf())
            .err(),
        Some(ProjectError::DuplicatePath)
    );
}

#[cfg(unix)]
#[test]
fn create_rejects_symlink_aliases_of_a_stored_path() {
    let store = ProjectStore::in_memory();
    let dir = git_worktree();
    store
        .create("One".to_owned(), dir.path().to_path_buf())
        .expect("first");
    let parent = tempfile::tempdir().expect("parent");
    let alias = parent.path().join("alias");
    std::os::unix::fs::symlink(dir.path(), &alias).expect("symlink");
    assert_eq!(
        store.create("Two".to_owned(), alias).err(),
        Some(ProjectError::DuplicatePath)
    );
}

#[test]
fn create_rejects_unsupported_worktrees() {
    let store = ProjectStore::in_memory();
    let dir = tempfile::tempdir().expect("dir");
    assert_eq!(
        store
            .create("Desk".to_owned(), dir.path().to_path_buf())
            .err(),
        Some(ProjectError::Worktree)
    );
}

#[test]
fn update_name_enforces_revision_and_keeps_the_path() {
    let store = ProjectStore::in_memory();
    let dir = git_worktree();
    let created = store
        .create("Desk".to_owned(), dir.path().to_path_buf())
        .expect("create");
    assert_eq!(
        store
            .update_name(&created.id, created.revision + 1, "Later".to_owned())
            .err(),
        Some(ProjectError::Conflict)
    );
    let updated = store
        .update_name(&created.id, created.revision, "Later".to_owned())
        .expect("rename");
    assert_eq!(updated.name, "Later");
    assert_eq!(updated.revision, created.revision + 1);
    assert_eq!(updated.host_path, created.host_path);
    assert_eq!(
        store
            .update_name(&created.id, created.revision, "Stale".to_owned())
            .err(),
        Some(ProjectError::Conflict)
    );
    assert_eq!(
        store
            .update_name(
                &crate::projects::ProjectId::generate().expect("missing"),
                1,
                "Gone".to_owned(),
            )
            .err(),
        Some(ProjectError::Missing)
    );
}

#[test]
fn failed_persistence_rolls_back_create_and_rename() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("projects.json");
    let store = ProjectStore::open(path.clone()).expect("open");
    let worktree = git_worktree();

    std::fs::create_dir(&path).expect("blocking directory");
    assert_eq!(
        store
            .create("Desk".to_owned(), worktree.path().to_path_buf())
            .err(),
        Some(ProjectError::Persist)
    );
    assert!(store.list().is_empty());

    std::fs::remove_dir(&path).expect("remove blocking directory");
    let created = store
        .create("Desk".to_owned(), worktree.path().to_path_buf())
        .expect("create");
    std::fs::remove_file(&path).expect("remove catalogue");
    std::fs::create_dir(&path).expect("blocking directory");
    assert_eq!(
        store
            .update_name(&created.id, created.revision, "Later".to_owned())
            .err(),
        Some(ProjectError::Persist)
    );
    assert_eq!(store.get(&created.id), Some(created));
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
    let root = tempfile::tempdir().expect("root");
    for index in 0..MAXIMUM_PROJECTS {
        let path = root.path().join(format!("project-{index}"));
        std::fs::create_dir(&path).expect("dir");
        git_init(&path);
        store.create(format!("Desk {index}"), path).expect("create");
    }
    let overflow = git_worktree();
    assert_eq!(
        store
            .create("Overflow".to_owned(), overflow.path().to_path_buf())
            .err(),
        Some(ProjectError::Full)
    );
}
