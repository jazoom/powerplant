use std::path::{Path, PathBuf};

use super::{
    MAXIMUM_NAME_BYTES, MAXIMUM_PATH_BYTES, ProjectError, ProjectFile, ProjectRecord,
    normalise_name, stored_host_path, submitted_host_path,
};
use crate::projects::id::ProjectId;

fn stored_file(name: &str, host_path: &str, revision: u32) -> ProjectFile {
    ProjectFile {
        id: ProjectId::generate().expect("id").as_hex(),
        revision,
        name: name.to_owned(),
        host_path: host_path.to_owned(),
        created_at_ms: 1,
    }
}

#[test]
fn names_reject_empty_controls_and_bounds() {
    assert_eq!(normalise_name("  ").err(), Some(ProjectError::Name));
    assert_eq!(
        normalise_name("a".repeat(MAXIMUM_NAME_BYTES + 1).as_str()).err(),
        Some(ProjectError::Name)
    );
    assert_eq!(normalise_name("bad\nname").err(), Some(ProjectError::Name));
    assert_eq!(normalise_name("\nDesk\t").err(), Some(ProjectError::Name));
    assert_eq!(normalise_name("  Desk  ").as_deref(), Ok("Desk"));
}

#[test]
fn stored_paths_must_be_absolute_and_bounded() {
    assert_eq!(
        stored_host_path(Path::new("relative/project")).err(),
        Some(ProjectError::Path)
    );
    assert_eq!(
        stored_host_path(Path::new("/tmp/\nproject")).err(),
        Some(ProjectError::Path)
    );
    let oversized = format!("/{}", "a".repeat(MAXIMUM_PATH_BYTES));
    assert_eq!(
        stored_host_path(Path::new(&oversized)).err(),
        Some(ProjectError::Path)
    );
    assert_eq!(
        stored_host_path(Path::new("/srv/powerplant")),
        Ok(PathBuf::from("/srv/powerplant"))
    );

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;

        let invalid_utf8 = PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', 0xff]));
        assert_eq!(
            stored_host_path(&invalid_utf8).err(),
            Some(ProjectError::Path)
        );
    }
}

#[test]
fn stored_files_reject_zero_revisions_and_relative_paths() {
    assert_eq!(
        ProjectRecord::from_file(stored_file("Desk", "/srv/app", 0)).err(),
        Some(ProjectError::Corrupt)
    );
    assert_eq!(
        ProjectRecord::from_file(stored_file("Desk", "srv/app", 1)).err(),
        Some(ProjectError::Corrupt)
    );
}

#[test]
fn new_records_start_at_revision_one() {
    let id = ProjectId::generate().expect("id");
    let record = ProjectRecord::create(id, "Desk".to_owned(), PathBuf::from("/srv/app"), 10)
        .expect("create");
    assert_eq!(record.id, id);
    assert_eq!(record.revision, 1);
    assert_eq!(record.name, "Desk");
    assert_eq!(record.host_path, PathBuf::from("/srv/app"));
    assert_eq!(record.created_at_ms, 10);
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
}

#[test]
fn submitted_paths_require_an_accessible_git_worktree() {
    assert_eq!(
        submitted_host_path(Path::new("relative/project")).err(),
        Some(ProjectError::Path)
    );
    let missing = PathBuf::from("/no-such-powerplant-project");
    assert_eq!(
        submitted_host_path(&missing).err(),
        Some(ProjectError::NotADirectory)
    );
    let dir = tempfile::tempdir().expect("dir");
    assert_eq!(
        submitted_host_path(dir.path()).err(),
        Some(ProjectError::Worktree)
    );
    git_init(dir.path());
    let canonical = dir.path().canonicalize().expect("canonical");
    assert_eq!(submitted_host_path(dir.path()).expect("path"), canonical);
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, b"notes").expect("file");
    assert_eq!(
        submitted_host_path(&file).err(),
        Some(ProjectError::NotADirectory)
    );
}

#[test]
fn availability_follows_the_stored_canonical_path() {
    let dir = tempfile::tempdir().expect("dir");
    git_init(dir.path());
    let host_path = dir.path().canonicalize().expect("canonical");
    let record = ProjectRecord::create(
        ProjectId::generate().expect("id"),
        "Desk".to_owned(),
        host_path.clone(),
        1,
    )
    .expect("record");
    assert!(record.host_path_is_available());
    drop(dir);
    assert!(!record.host_path_is_available());

    std::fs::write(&host_path, b"not a directory").expect("replacement file");
    assert!(!record.host_path_is_available());
    std::fs::remove_file(&host_path).expect("remove replacement file");
    std::fs::create_dir(&host_path).expect("replacement directory");
    assert!(record.host_path_is_available());
    std::fs::remove_dir(&host_path).expect("remove replacement directory");
    assert_eq!(record.host_path, host_path);
}
