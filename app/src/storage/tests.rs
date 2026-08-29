use std::fs;

use super::{ensure_private_dir, write_private};

#[test]
fn write_private_replaces_bytes_and_leaves_no_temporary_files() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("record.json");
    write_private(&path, b"first").expect("first write");
    write_private(&path, b"second").expect("second write");
    assert_eq!(fs::read(&path).expect("read"), b"second");
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .expect("list")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn failed_write_preserves_the_prior_destination() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("record.json");
    write_private(&path, b"keep").expect("first write");
    let blocked = dir.path().join("missing").join("record.json");
    assert!(write_private(&blocked, b"new").is_err());
    assert_eq!(fs::read(&path).expect("read"), b"keep");
}

#[test]
fn failed_write_removes_its_temporary_file() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("not-a-dir");
    fs::write(&file, b"x").expect("file");
    let path = file.join("record.json");
    assert!(write_private(&path, b"new").is_err());
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .expect("list")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[cfg(unix)]
#[test]
fn owner_only_permissions_apply_to_files_and_directories() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("dir");
    let private = dir.path().join("workflow-runs");
    ensure_private_dir(&private).expect("dir");
    let path = private.join("record.json");
    write_private(&path, b"secret").expect("write");
    let dir_mode = fs::metadata(&private)
        .expect("dir meta")
        .permissions()
        .mode()
        & 0o777;
    let file_mode = fs::metadata(&path).expect("file meta").permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700);
    assert_eq!(file_mode, 0o600);
}

#[cfg(unix)]
#[test]
fn a_read_only_parent_does_not_replace_the_destination() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("record.json");
    write_private(&path, b"keep").expect("first write");
    let mut permissions = fs::metadata(dir.path()).expect("meta").permissions();
    permissions.set_mode(0o555);
    fs::set_permissions(dir.path(), permissions).expect("lock");
    let failed = write_private(&path, b"new");
    let mut restore = fs::metadata(dir.path()).expect("meta").permissions();
    restore.set_mode(0o700);
    fs::set_permissions(dir.path(), restore).expect("unlock");
    assert!(failed.is_err());
    assert_eq!(fs::read(&path).expect("read"), b"keep");
}
