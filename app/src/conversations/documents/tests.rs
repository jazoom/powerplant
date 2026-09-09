use std::sync::Arc;

use super::*;
use crate::conversations::{ConversationMessage, ConversationStore, MessageRole, MessageStatus};
use crate::workflows::artefacts::WorkflowArtefactRepository;

impl PlanDocumentStore {
    pub(crate) fn create_from_text(
        &self,
        conversation_id: ConversationId,
        title: String,
        markdown: String,
        secret: Option<&str>,
    ) -> Result<PlanDocument, DocumentError> {
        let source = PlanSource::SubmittedText {
            conversation_id,
            source_hash: ObjectHash::of(markdown.as_bytes()),
        };
        self.create(
            DocumentKind::Plan,
            conversation_id,
            title,
            &markdown,
            source,
            secret,
        )
    }

    pub(crate) fn create_from_message(
        &self,
        conversation: &ConversationRecord,
        message_index: usize,
        title: String,
        secret: Option<&str>,
    ) -> Result<PlanDocument, DocumentError> {
        let message = conversation
            .messages
            .get(message_index)
            .ok_or(DocumentError::Source)?;
        if message.role != MessageRole::Assistant
            || message.status != MessageStatus::Complete
            || message.text.trim().is_empty()
        {
            return Err(DocumentError::Source);
        }
        let source = PlanSource::ConversationMessage {
            conversation_id: conversation.id,
            message_index: message_index as u32,
            source_hash: ObjectHash::of(message.text.as_bytes()),
        };
        self.create(
            DocumentKind::Plan,
            conversation.id,
            title,
            &message.text,
            source,
            secret,
        )
    }

    pub(crate) fn in_memory(content: Arc<WorkflowArtefactRepository>) -> Self {
        Self {
            path: None,
            content,
            inner: Mutex::new(BTreeMap::new()),
        }
    }
}

fn conversation() -> ConversationRecord {
    let store = ConversationStore::in_memory();
    let mut record = store.create("Discussion".to_owned()).expect("conversation");
    record.messages.push(ConversationMessage {
        role: MessageRole::Assistant,
        text: "# First plan\n\n- Inspect the change\n".to_owned(),
        status: MessageStatus::Complete,
        error: None,
        request: None,
    });
    record
}

#[test]
fn explicit_actions_keep_titles_sources_and_content_after_revision_and_removal() {
    let directory = tempfile::tempdir().unwrap();
    let content = Arc::new(WorkflowArtefactRepository::in_memory());
    let store = PlanDocumentStore::open(directory.path().to_path_buf(), content.clone()).unwrap();
    let record = conversation();
    let action = |title: &str, markdown: &str, previous, plan| DocumentAction {
        title: title.to_owned(),
        markdown: markdown.to_owned(),
        assistant: true,
        message_index: 0,
        previous,
        plan,
    };
    let first = store
        .publish_action(
            &record,
            action("Original title", "Original text", None, None),
            None,
        )
        .unwrap();
    let tasks = store
        .publish_action(
            &record,
            action(
                "Breakdown",
                "# Tasks\n- [ ] Original task",
                None,
                Some((first.id, 1)),
            ),
            None,
        )
        .unwrap();
    let revised = store
        .publish_action(
            &record,
            action("New title", "New text", Some((first.id, 1)), None),
            None,
        )
        .unwrap();
    assert_eq!(revised.revisions[0], first.revisions[0]);
    assert!(
        matches!(&revised.revisions[0].source, PlanSource::Action { title, .. } if title == "Original title")
    );
    assert!(
        matches!(&tasks.current().source, PlanSource::Action { plan: Some(plan), .. } if *plan == first.current().reference(first.id))
    );
    assert_eq!(
        store
            .publish_action(
                &record,
                action("Stale", "Stale", Some((first.id, 1)), None),
                None
            )
            .err(),
        Some(DocumentError::Conflict)
    );
    store.disassociate(&first.id, 2, record.id).unwrap();
    drop(store);
    let reopened = PlanDocumentStore::open(directory.path().to_path_buf(), content).unwrap();
    let retained = reopened.get(&first.id).unwrap();
    assert_eq!(reopened.content(&retained, 1).unwrap(), "Original text");
    assert_eq!(reopened.action_documents(record.id).len(), 2);
    let path = directory.path().join(CATALOGUE_FILE);
    let mut file: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    file["documents"][0]["title"] = serde_json::json!("Forged current title");
    crate::storage::write_private(&path, &serde_json::to_vec(&file).unwrap()).unwrap();
    assert_eq!(
        PlanDocumentStore::open(directory.path().to_path_buf(), reopened.content.clone()).err(),
        Some(DocumentError::Corrupt)
    );
}

#[test]
fn explicit_actions_reject_cross_conversation_identity_and_invalid_content_without_publication() {
    let store = memory_store();
    let owner = conversation();
    let other = conversation();
    let action = |markdown: &str, previous, plan| DocumentAction {
        title: "Plan".to_owned(),
        markdown: markdown.to_owned(),
        assistant: true,
        message_index: 0,
        previous,
        plan,
    };
    let plan = store
        .publish_action(&owner, action("Original", None, None), None)
        .unwrap();
    assert_eq!(
        store
            .publish_action(&other, action("Revision", Some((plan.id, 1)), None), None)
            .err(),
        Some(DocumentError::Source)
    );
    assert_eq!(
        store
            .publish_action(
                &owner,
                action("No checkboxes", None, Some((plan.id, 1))),
                None
            )
            .err(),
        Some(DocumentError::TaskList)
    );
    assert_eq!(
        store
            .publish_action(&owner, action("credential", None, None), Some("credential"))
            .err(),
        Some(DocumentError::Credential)
    );
    let excessive = format!(
        "# Tasks\n{}",
        (0..129)
            .map(|index| format!("- [ ] Task {index}\n"))
            .collect::<String>()
    );
    assert_eq!(
        store
            .publish_action(&owner, action(&excessive, None, Some((plan.id, 1))), None)
            .err(),
        Some(DocumentError::TaskBreakdown)
    );
    assert_eq!(store.get(&plan.id), Some(plan));
    assert_eq!(store.list_for_conversation(owner.id).len(), 1);
    assert!(store.list_for_conversation(other.id).is_empty());
}

fn memory_store() -> PlanDocumentStore {
    PlanDocumentStore::in_memory(Arc::new(WorkflowArtefactRepository::in_memory()))
}

#[test]
fn document_ids_are_opaque_and_round_trip() {
    let first = DocumentId::generate().expect("first");
    let second = DocumentId::generate().expect("second");
    assert_ne!(first, second);
    assert_eq!(DocumentId::parse(&first.as_hex()), Some(first));
    assert!(DocumentId::parse("plan").is_none());
}

#[test]
fn message_source_and_correction_keep_exact_revision_references() {
    let store = memory_store();
    let conversation = conversation();
    let document = store
        .create_from_message(&conversation, 0, "First plan".to_owned(), None)
        .expect("save");
    let first = document.current().clone();
    assert_eq!(
        first.source,
        PlanSource::ConversationMessage {
            conversation_id: conversation.id,
            message_index: 0,
            source_hash: ObjectHash::of(conversation.messages[0].text.as_bytes()),
        }
    );

    let corrected = store
        .revise(
            &document.id,
            document.current_revision(),
            "Corrected plan".to_owned(),
            "# Corrected plan\n".to_owned(),
            None,
        )
        .expect("correct");
    assert_eq!(
        store.content(&corrected, 1).expect("first content"),
        "# First plan\n\n- Inspect the change\n"
    );
    assert_eq!(
        store.content(&corrected, 2).expect("corrected content"),
        "# Corrected plan\n"
    );
    assert_eq!(
        corrected.current().source,
        PlanSource::Correction {
            previous: PlanRevisionReference {
                document_id: document.id,
                revision: first.revision,
                content_hash: first.content_hash,
                object_hash: first.object_hash,
                artefact_hash: first.artefact_hash,
            },
        }
    );
}

#[test]
fn invalid_message_sources_and_secret_text_are_rejected() {
    let store = memory_store();
    let mut record = conversation();
    record.messages[0].role = MessageRole::User;
    assert_eq!(
        store
            .create_from_message(&record, 0, "Plan".to_owned(), None)
            .err(),
        Some(DocumentError::Source)
    );
    assert_eq!(
        store
            .create_from_text(
                record.id,
                "Plan".to_owned(),
                "use secret".to_owned(),
                Some("secret")
            )
            .err(),
        Some(DocumentError::Credential)
    );
    assert_eq!(
        store
            .create_from_text(
                record.id,
                "secret".to_owned(),
                "Plan".to_owned(),
                Some("secret")
            )
            .err(),
        Some(DocumentError::Credential)
    );
    assert_eq!(
        store
            .create_from_text(record.id, "Plan".to_owned(), "\0".to_owned(), None)
            .err(),
        Some(DocumentError::Content)
    );
    assert_eq!(
        store
            .create_from_text(
                record.id,
                "Plan".to_owned(),
                "x".repeat(MAXIMUM_DOCUMENT_CONTENT_BYTES + 1),
                None
            )
            .err(),
        Some(DocumentError::Full)
    );
}

#[test]
fn task_lists_require_valid_model_or_submitted_output() {
    let store = memory_store();
    let conversation = conversation();
    let document = store
        .create_task_list_from_text(
            conversation.id,
            "Tasks".to_owned(),
            "# Tasks\n\n- [x] Done\n- [ ] Next\n".to_owned(),
            None,
        )
        .expect("task list");
    assert_eq!(document.kind, DocumentKind::TaskList);
    assert_eq!(
        store
            .create_task_list_from_text(
                conversation.id,
                "Invalid".to_owned(),
                "# Missing tasks\n".to_owned(),
                None,
            )
            .err(),
        Some(DocumentError::TaskList)
    );
}

#[test]
fn document_count_is_bounded() {
    let store = memory_store();
    let conversation = conversation();
    for _ in 0..MAXIMUM_DOCUMENTS {
        store
            .create_from_text(
                conversation.id,
                "Plan".to_owned(),
                "one line".to_owned(),
                None,
            )
            .expect("within document count");
    }
    assert_eq!(
        store
            .create_from_text(
                conversation.id,
                "Plan".to_owned(),
                "one line".to_owned(),
                None,
            )
            .err(),
        Some(DocumentError::Full)
    );
}

#[test]
fn stale_and_excessive_revisions_leave_prior_content_unchanged() {
    let store = memory_store();
    let record = conversation();
    let first = store
        .create_from_message(&record, 0, "Plan".to_owned(), None)
        .expect("plan");
    let mut latest = first.clone();
    for _ in 1..MAXIMUM_DOCUMENT_REVISIONS {
        latest = store
            .revise(
                &latest.id,
                latest.current_revision(),
                "Plan".to_owned(),
                "Correction".to_owned(),
                None,
            )
            .expect("revision");
    }
    for (revision, error) in [
        (1, DocumentError::Conflict),
        (latest.current_revision(), DocumentError::Full),
    ] {
        assert_eq!(
            store
                .revise(
                    &latest.id,
                    revision,
                    "Changed title".to_owned(),
                    "Rejected".to_owned(),
                    None
                )
                .err(),
            Some(error)
        );
        assert_eq!(store.get(&latest.id), Some(latest.clone()));
    }
    assert_eq!(
        store.content(&latest, 1).expect("original"),
        record.messages[0].text
    );
}

#[test]
fn documents_and_content_survive_restart() {
    let root = tempfile::tempdir().expect("root");
    let objects =
        Arc::new(WorkflowArtefactRepository::open(root.path().join("objects")).expect("objects"));
    let conversation = conversation();
    let saved;
    {
        let store = PlanDocumentStore::open(root.path().join("documents"), objects.clone())
            .expect("documents");
        saved = store
            .create_from_message(&conversation, 0, "Plan".to_owned(), None)
            .expect("save");
    }
    let reopened = PlanDocumentStore::open(root.path().join("documents"), objects).expect("open");
    let document = reopened.get(&saved.id).expect("document");
    assert_eq!(
        reopened.content(&document, 1).expect("content"),
        "# First plan\n\n- Inspect the change\n"
    );
}

#[test]
fn association_removal_keeps_document_content() {
    let store = memory_store();
    let conversation = conversation();
    let document = store
        .create_from_text(
            conversation.id,
            "Plan".to_owned(),
            "# Plan\n".to_owned(),
            None,
        )
        .expect("save");
    store
        .disassociate(&document.id, document.current_revision(), conversation.id)
        .expect("remove association");
    let retained = store.get(&document.id).expect("retained document");
    assert_eq!(retained.associated_conversation, None);
    assert_eq!(
        store.content(&retained, 1).expect("retained content"),
        "# Plan\n"
    );
    assert!(store.list_for_conversation(conversation.id).is_empty());
}

#[test]
fn task_corrections_reject_invalid_structure_without_replacing_the_original() {
    let store = memory_store();
    let record = conversation();
    let source = "# Tasks\r\n\r\n- [x] Done\r\n- [ ] Pending\r\n";
    let document = store
        .create_task_list_from_text(record.id, "Tasks".to_owned(), source.to_owned(), None)
        .expect("tasks");
    assert_eq!(
        store
            .revise(
                &document.id,
                1,
                "Broken".to_owned(),
                "# No tasks".to_owned(),
                None
            )
            .err(),
        Some(DocumentError::TaskList)
    );
    assert_eq!(store.get(&document.id), Some(document.clone()));
    let corrected = store
        .revise(
            &document.id,
            1,
            "Corrected".to_owned(),
            "# Tasks\n- [ ] Revised\n".to_owned(),
            None,
        )
        .expect("correction");
    assert_eq!(store.content(&corrected, 1).expect("original"), source);
    assert_eq!(
        store
            .create_task_list_from_message(&record, 0, "Not tasks".to_owned(), None)
            .err(),
        Some(DocumentError::TaskList)
    );
}
