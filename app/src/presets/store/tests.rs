use crate::{
    environments::EnvironmentId,
    execution::ExecutionSettings,
    presets::{PresetDestination, PresetProvenance, PresetStore},
    providers::{ModelSelection, ProviderKind},
};

fn settings() -> ExecutionSettings {
    ExecutionSettings::new(
        ModelSelection::new(ProviderKind::Xai, "grok-4.6".to_owned(), None).unwrap(),
        "Use evidence.".to_owned(),
        vec![crate::agents::ToolId::Read],
        EnvironmentId::generate().unwrap(),
    )
    .unwrap()
}

#[test]
fn preview_survives_source_deletion_but_not_replay() {
    let store = PresetStore::in_memory();
    let record = store
        .create("Review", settings(), PresetProvenance::Draft)
        .unwrap();
    let session = crate::sessions::generate_session_token().unwrap().id();
    let destination = PresetDestination::Draft("draft digest".to_owned());
    let preview = store
        .preview(session, record.id, destination.clone())
        .unwrap();
    store.lock().remove(&record.id);
    let applied = store
        .consume_preview(session, &preview.token, &destination)
        .unwrap();
    assert_eq!(applied, record);
    assert!(
        store
            .consume_preview(session, &preview.token, &destination)
            .is_err()
    );
}

#[test]
fn persisted_snapshot_rejects_unknown_tools() {
    let temp = tempfile::tempdir().unwrap();
    let store = PresetStore::open(temp.path().to_path_buf()).unwrap();
    let record = store
        .create("Independent", settings(), PresetProvenance::Draft)
        .unwrap();
    let reopened = PresetStore::open(temp.path().to_path_buf()).unwrap();
    assert_eq!(reopened.get(&record.id).unwrap(), record);
    let path = temp.path().join(format!("{}.json", record.id));
    let mut file: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    file["settings"]["tools"] = serde_json::json!(["unknown"]);
    std::fs::write(path, serde_json::to_vec(&file).unwrap()).unwrap();
    assert!(PresetStore::open(temp.path().to_path_buf()).is_err());
}

#[test]
fn preview_rejects_other_sessions_destinations_revisions_and_expiry() {
    let store = PresetStore::in_memory();
    let record = store
        .create("Bound", settings(), PresetProvenance::Draft)
        .unwrap();
    let session = crate::sessions::generate_session_token().unwrap().id();
    let other_session = crate::sessions::generate_session_token().unwrap().id();
    let conversation = crate::conversations::ConversationId::generate().unwrap();
    let destination = PresetDestination::Conversation(conversation, 1);
    for (owner, target) in [
        (other_session, destination.clone()),
        (session, PresetDestination::Conversation(conversation, 2)),
        (
            session,
            PresetDestination::Conversation(
                crate::conversations::ConversationId::generate().unwrap(),
                1,
            ),
        ),
        (session, PresetDestination::Draft("digest".to_owned())),
    ] {
        let preview = store
            .preview(session, record.id, destination.clone())
            .unwrap();
        assert!(
            store
                .consume_preview(owner, &preview.token, &target)
                .is_err()
        );
    }
    let preview = store
        .preview(session, record.id, destination.clone())
        .unwrap();
    store
        .previews
        .lock()
        .unwrap()
        .get_mut(&preview.token)
        .unwrap()
        .expires = std::time::Instant::now();
    assert!(
        store
            .consume_preview(session, &preview.token, &destination)
            .is_err()
    );
}
