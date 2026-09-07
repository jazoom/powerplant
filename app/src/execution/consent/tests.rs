use crate::{agents::AccessMode, conversations::ConversationId, sessions};

use super::AccessConsentStore;

fn session() -> sessions::SessionId {
    let token = sessions::generate_session_token().unwrap();
    token.id()
}

#[test]
fn draft_consent_is_single_use_and_becomes_conversation_consent() {
    let root = tempfile::tempdir().unwrap();
    let grant = crate::execution::DirectoryGrant::from_selected(root.path(), &[]).unwrap();
    let directories = vec![grant.clone()];
    let settings = crate::execution::ExecutionSettings::new(
        crate::providers::ModelSelection::new(
            crate::providers::ProviderKind::Xai,
            "model".to_owned(),
            None,
        )
        .unwrap(),
        String::new(),
        Vec::new(),
    )
    .unwrap()
    .with_directories(directories.clone())
    .unwrap();
    let session = session();
    let store = AccessConsentStore::new();
    let request = store
        .request_draft(session, "draft", &directories, &grant)
        .unwrap();
    let reference = store
        .approve_draft(&request, session, "draft", &directories, &grant)
        .unwrap();

    assert!(
        store
            .approve_draft(&request, session, "draft", &directories, &grant)
            .is_err()
    );
    let conversation = ConversationId::generate().unwrap();
    store
        .consume_draft(
            &reference,
            session,
            "draft",
            &settings,
            conversation,
            &directories,
        )
        .unwrap();
    assert!(store.authorised_conversation(session, conversation, &settings, &grant));
    let mut changed = settings.clone();
    changed.network = crate::agents::NetworkAccess::Public;
    assert!(!store.authorised_conversation(session, conversation, &changed, &grant));
    changed = settings.clone();
    changed.tools.push(crate::agents::ToolId::Read);
    assert!(!store.authorised_conversation(session, conversation, &changed, &grant));
    assert!(!AccessConsentStore::new().authorised_conversation(
        session,
        conversation,
        &settings,
        &grant
    ));
    assert!(
        store
            .consume_draft(
                &reference,
                session,
                "draft",
                &settings,
                conversation,
                &directories,
            )
            .is_err()
    );
    assert!(
        store
            .request_draft(session, "draft:changed-settings", &directories, &grant)
            .is_err()
    );
    store.retain_sessions(|_| false);
    assert!(!store.authorised_conversation(session, conversation, &settings, &grant));
}

#[test]
fn consent_rejects_another_session_strategy_or_root_identity() {
    let parent = tempfile::tempdir().unwrap();
    let first = parent.path().join("first");
    std::fs::create_dir(&first).unwrap();
    let grant = crate::execution::DirectoryGrant::from_selected(&first, &[]).unwrap();
    let directories = vec![grant.clone()];
    let owner = session();
    let other = session();
    let store = AccessConsentStore::new();
    let request = store
        .request_draft(owner, "draft", &directories, &grant)
        .unwrap();

    assert!(
        store
            .approve_draft(&request, other, "draft", &directories, &grant)
            .is_err()
    );
    let mut changed = grant.clone();
    changed.access = AccessMode::ReadWrite;
    assert!(
        store
            .approve_draft(&request, owner, "draft", &[changed.clone()], &changed)
            .is_err()
    );

    std::fs::rename(&first, parent.path().join("old")).unwrap();
    std::fs::create_dir(&first).unwrap();
    let replacement = crate::execution::DirectoryGrant::from_selected(&first, &[]).unwrap();
    assert!(
        store
            .approve_draft(
                &request,
                owner,
                "draft",
                std::slice::from_ref(&replacement),
                &replacement
            )
            .is_err()
    );
}
