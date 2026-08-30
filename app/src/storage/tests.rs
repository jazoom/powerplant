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

#[cfg(unix)]
#[test]
fn recursive_removal_does_not_follow_a_replaced_directory_entry() {
    use cap_std::ambient_authority;
    use cap_std::fs::Dir;

    let root = tempfile::tempdir().expect("root");
    let outside = tempfile::tempdir().expect("outside");
    let sentinel = outside.path().join("keep.txt");
    fs::write(&sentinel, b"keep").expect("sentinel");
    let victim = root.path().join("victim");
    fs::create_dir(&victim).expect("victim");
    let parent = Dir::open_ambient_dir(root.path(), ambient_authority()).expect("parent");
    let entry = parent
        .entries()
        .expect("entries")
        .next()
        .expect("entry")
        .expect("entry");
    fs::remove_dir(&victim).expect("remove victim");
    std::os::unix::fs::symlink(outside.path(), &victim).expect("replace with link");

    super::remove_entry_nofollow(entry).expect("remove link");

    assert_eq!(fs::read(&sentinel).expect("read sentinel"), b"keep");
    assert!(!victim.exists());
}

#[test]
fn confined_child_rejects_separators_and_dot_components() {
    let dir = tempfile::tempdir().expect("dir");
    let root = dir.path();
    assert!(super::confined_child(root, "abc").is_ok());
    assert!(super::confined_child(root, "").is_err());
    assert!(super::confined_child(root, ".").is_err());
    assert!(super::confined_child(root, "..").is_err());
    assert!(super::confined_child(root, "a/b").is_err());
    assert!(super::confined_child(root, "a\\b").is_err());
    assert!(super::confined_child(root, "a\0b").is_err());
}

#[test]
fn bounded_logger_appends_until_the_byte_limit_then_truncates() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("prep.log");
    let mut logger = super::BoundedLogger::create(path.clone()).expect("create");
    logger.append(b"hello\n").expect("append");
    assert_eq!(fs::read(&path).expect("read"), b"hello\n");
    assert!(!logger.state().truncated);
    let chunk = vec![b'x'; super::LOG_LIMIT_BYTES as usize];
    let state = logger.append(&chunk).expect("overflow");
    assert!(state.truncated);
    let bytes = fs::read(&path).expect("truncated");
    assert!(bytes.starts_with(b"hello\n"));
    assert!(
        bytes
            .windows(super::LOG_TRUNCATION_MARKER.len())
            .any(|window| window == super::LOG_TRUNCATION_MARKER)
    );
    logger.append(b"tail-bytes").expect("later");
    let later = fs::read(&path).expect("later read");
    assert!(later.ends_with(b"tail-bytes"));
    assert!(
        later
            .windows(super::LOG_TRUNCATION_MARKER.len())
            .any(|window| window == super::LOG_TRUNCATION_MARKER)
    );
}

#[cfg(unix)]
#[test]
fn bounded_logger_uses_private_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("prep.log");
    let mut logger = super::BoundedLogger::create(path.clone()).expect("create");
    logger.append(b"secret").expect("append");
    let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}
