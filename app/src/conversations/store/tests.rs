use std::path::Path;

use super::{ConversationError, ConversationStore, MAXIMUM_CATALOGUE_BYTES, MAXIMUM_TITLE_BYTES};

impl ConversationStore {
    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            inner: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }
}

fn write_catalogue(dir: &Path, contents: &str) {
    std::fs::write(dir.join("catalogue.json"), contents).expect("catalogue");
}

#[test]
fn distinct_opaque_conversations_survive_a_restart() {
    let dir = tempfile::tempdir().expect("directory");
    let (first, second);
    {
        let store = ConversationStore::open(dir.path().to_path_buf()).expect("store");
        first = store.create("First discussion".to_owned()).expect("first");
        second = store
            .create("Second discussion".to_owned())
            .expect("second");
        assert_ne!(first.id, second.id);
        assert_eq!(first.revision, 1);
        assert_eq!(second.revision, 1);
    }
    let store = ConversationStore::open(dir.path().to_path_buf()).expect("reopen");
    assert_eq!(store.get(&first.id), Some(first));
    assert_eq!(store.get(&second.id), Some(second));
}

#[test]
fn private_catalogue_path_rejects_a_symlink_without_replacement() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("root");
        let target = root.path().join("target");
        let path = root.path().join("conversations");
        std::fs::create_dir(&target).expect("target");
        symlink(&target, &path).expect("link");

        assert_eq!(
            ConversationStore::open(path).err(),
            Some(ConversationError::Persist)
        );
        assert!(target.read_dir().expect("target contents").next().is_none());
    }
}

#[test]
fn corrupt_catalogues_remain_unchanged() {
    let dir = tempfile::tempdir().expect("directory");
    write_catalogue(dir.path(), "{");
    let path = dir.path().join("catalogue.json");
    let original = std::fs::read(&path).expect("original");

    assert_eq!(
        ConversationStore::open(dir.path().to_path_buf()).err(),
        Some(ConversationError::Corrupt)
    );
    assert_eq!(std::fs::read(path).expect("unchanged"), original);
}

#[test]
fn stale_revisions_do_not_replace_current_records() {
    let store = ConversationStore::in_memory();
    let created = store.create("Initial".to_owned()).expect("create");
    let renamed = store
        .rename(&created.id, created.revision, "Current".to_owned())
        .expect("rename");

    assert_eq!(
        store.rename(&created.id, created.revision, "Stale".to_owned()),
        Err(ConversationError::Conflict)
    );
    assert_eq!(
        store.delete(&created.id, created.revision),
        Err(ConversationError::Conflict)
    );
    assert_eq!(store.get(&created.id), Some(renamed));
}

#[test]
fn titles_and_catalogue_reads_are_bounded() {
    let store = ConversationStore::in_memory();
    for title in [
        String::new(),
        "   ".to_owned(),
        "a\0b".to_owned(),
        "é".repeat(MAXIMUM_TITLE_BYTES / 2 + 1),
    ] {
        assert_eq!(store.create(title).err(), Some(ConversationError::Title));
    }

    let dir = tempfile::tempdir().expect("directory");
    write_catalogue(dir.path(), &"x".repeat(MAXIMUM_CATALOGUE_BYTES + 1));
    assert_eq!(
        ConversationStore::open(dir.path().to_path_buf()).err(),
        Some(ConversationError::Corrupt)
    );
}

#[test]
fn full_catalogue_rejects_creation_without_eviction() {
    let store = ConversationStore::in_memory();
    for index in 0..super::MAXIMUM_CONVERSATIONS {
        store
            .create(format!("Conversation {index}"))
            .expect("create");
    }
    let original = store.list();
    assert_eq!(
        store.create("Overflow".to_owned()),
        Err(ConversationError::Full)
    );
    assert_eq!(store.list(), original);
}

#[test]
fn rename_preserves_timestamp_order_after_clock_regression() {
    let dir = tempfile::tempdir().expect("directory");
    let store = ConversationStore::open(dir.path().to_path_buf()).expect("store");
    let record = store.create("Original".to_owned()).expect("create");
    let mut future = record.clone();
    future.created_at_ms = u64::MAX;
    future.updated_at_ms = u64::MAX;
    store.lock().insert(record.id, future);
    let renamed = store
        .rename(&record.id, record.revision, "Renamed".to_owned())
        .expect("rename");
    drop(store);
    let reopened = ConversationStore::open(dir.path().to_path_buf()).expect("reopen");
    assert_eq!(reopened.get(&record.id), Some(renamed));
}

#[test]
fn invalid_record_fields_do_not_replace_the_catalogue() {
    let dir = tempfile::tempdir().expect("directory");
    let store = ConversationStore::open(dir.path().to_path_buf()).expect("store");
    store.create("Original".to_owned()).expect("create");
    drop(store);
    let path = dir.path().join("catalogue.json");
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
    for (field, value) in [
        ("id", serde_json::json!("../outside")),
        ("revision", serde_json::json!(0)),
        ("title", serde_json::json!(" padded ")),
        ("updated-at-ms", serde_json::json!(0)),
    ] {
        let mut invalid = original.clone();
        invalid["conversations"][0][field] = value;
        let bytes = serde_json::to_vec(&invalid).expect("encode");
        std::fs::write(&path, &bytes).expect("write");
        assert_eq!(
            ConversationStore::open(dir.path().to_path_buf()).err(),
            Some(ConversationError::Corrupt)
        );
        assert_eq!(std::fs::read(&path).expect("unchanged"), bytes);
    }
}
