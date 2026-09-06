use std::path::Path;

use crate::{
    providers::{ModelSelection, ProviderKind},
    sessions::JobId,
};

use super::{
    ConversationError, ConversationStore, MAXIMUM_CATALOGUE_BYTES, MAXIMUM_TITLE_BYTES,
    MessageStatus,
};

impl ConversationStore {
    pub(crate) fn begin_message(
        &self,
        id: &super::ConversationId,
        expected_revision: u32,
        selection: ModelSelection,
        request: JobId,
        text: String,
    ) -> Result<super::ConversationRecord, ConversationError> {
        self.begin_message_with_model(
            id,
            expected_revision,
            super::ConversationModelConfiguration::direct(selection),
            request,
            text,
        )
    }

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
fn active_request_rejects_stale_settlement() {
    let store = ConversationStore::in_memory();
    let record = store.create("Discussion".to_owned()).expect("conversation");
    let request = JobId::generate().expect("request");
    let selection =
        ModelSelection::new(ProviderKind::Xai, "grok-4.6".to_owned(), None).expect("selection");
    store
        .begin_message(
            &record.id,
            record.revision,
            selection,
            request,
            "Question".to_owned(),
        )
        .expect("begin");

    assert_eq!(
        store.settle_message(
            &record.id,
            JobId::generate().expect("stale request"),
            "Wrong reply".to_owned(),
            MessageStatus::Complete,
        ),
        Err(ConversationError::Conflict)
    );
    let current = store.get(&record.id).expect("current");
    assert_eq!(current.active_job, Some(request));
    assert!(current.messages.last().expect("assistant").text.is_empty());
}

#[test]
fn completed_and_interrupted_messages_survive_restart() {
    let dir = tempfile::tempdir().expect("directory");
    let store = ConversationStore::open(dir.path().to_path_buf()).expect("store");
    let mut record = store.create("Discussion".to_owned()).expect("conversation");
    let selection =
        ModelSelection::new(ProviderKind::Xai, "grok-4.6".to_owned(), None).expect("model");
    for complete in [true, false] {
        let request = JobId::generate().expect("request");
        record = store
            .begin_message(
                &record.id,
                record.revision,
                selection.clone(),
                request,
                "First line\n\tSecond line".to_owned(),
            )
            .expect("multiline message");
        assert_eq!(
            store.delete(&record.id, record.revision),
            Err(ConversationError::Active)
        );
        store
            .append_output(&record.id, request, "Partial reply".to_owned())
            .expect("partial");
        if complete {
            store
                .settle_message(
                    &record.id,
                    request,
                    "Complete reply".to_owned(),
                    MessageStatus::Complete,
                )
                .expect("settle");
        }
        record = store.get(&record.id).expect("current");
    }
    drop(store);
    let store = ConversationStore::open(dir.path().to_path_buf()).expect("restart");
    let recovered = store.get(&record.id).expect("recovered");
    assert_eq!(recovered.active_job, None);
    assert_eq!(recovered.messages[1].status, MessageStatus::Complete);
    assert_eq!(recovered.messages[1].text, "Complete reply");
    assert_eq!(recovered.messages[3].status, MessageStatus::Interrupted);
    assert_eq!(recovered.messages[3].text, "Partial reply");
    assert_eq!(
        store.settle_message(
            &record.id,
            record.active_job.expect("old request"),
            "Stale".to_owned(),
            MessageStatus::Complete
        ),
        Err(ConversationError::Conflict)
    );
}

#[test]
fn message_bounds_reserve_space_for_terminal_output() {
    let store = ConversationStore::in_memory();
    let record = store.create("Discussion".to_owned()).expect("conversation");
    let selection =
        ModelSelection::new(ProviderKind::Xai, "grok-4.6".to_owned(), None).expect("model");
    let request = JobId::generate().expect("request");
    for text in [
        "x".repeat(super::MAXIMUM_MESSAGE_BYTES + 1),
        "invalid\0text".to_owned(),
    ] {
        assert_eq!(
            store.begin_message(
                &record.id,
                record.revision,
                selection.clone(),
                request,
                text
            ),
            Err(ConversationError::Message)
        );
    }
    store
        .begin_message(
            &record.id,
            record.revision,
            selection.clone(),
            request,
            "Question".to_owned(),
        )
        .expect("begin");
    let other = store.create("Other".to_owned()).expect("other");
    assert_eq!(
        store.begin_message(
            &other.id,
            other.revision,
            selection,
            JobId::generate().expect("request"),
            "Question".to_owned()
        ),
        Err(ConversationError::Full)
    );
    assert_eq!(
        store.append_output(
            &record.id,
            request,
            "x".repeat(super::MAXIMUM_REPLY_BYTES + 1)
        ),
        Err(ConversationError::Message)
    );
    let reply = "\u{0001}".repeat(super::MAXIMUM_REPLY_BYTES);
    store
        .append_output(&record.id, request, reply.clone())
        .expect("reserved capacity");
    store
        .settle_message(&record.id, request, reply, MessageStatus::Interrupted)
        .expect("terminal capacity");
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
