use super::super::tests::*;
use crate::{
    conversations::{ConversationId, ConversationModelConfiguration},
    providers::{ModelSelection, ProviderKind},
};
use axum::http::StatusCode;
use tower::ServiceExt;

fn conversation(state: &crate::state::AppState) -> crate::conversations::ConversationRecord {
    state
        .conversations
        .create_saved(
            ConversationId::generate().unwrap(),
            None,
            Some("Directory test".to_owned()),
            Some(ConversationModelConfiguration::direct(
                ModelSelection::new(ProviderKind::Xai, "grok-4.6".to_owned(), None).unwrap(),
            )),
            None,
        )
        .unwrap()
}

#[tokio::test]
async fn picker_commands_are_patch_only_and_revision_bound() {
    let state = test_state();
    let token = connected(&state);
    let record = conversation(&state);
    let directory = tempfile::tempdir().unwrap();
    state
        .folder_picker
        .queue(Some(directory.path().to_path_buf()));
    let path = format!("/conversations/{}/directories/pick", record.id);

    let mut native = command(&path, &token, "revision=1");
    native.headers_mut().remove("graft-request");
    assert_eq!(
        app(&state).oneshot(native).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert!(
        state
            .conversations
            .get(&record.id)
            .unwrap()
            .model
            .unwrap()
            .settings
            .directories
            .is_empty()
    );

    let response = app(&state)
        .oneshot(command(&path, &token, "revision=1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("target=\"conversation-detail\""));
    let updated = state.conversations.get(&record.id).unwrap();
    let grant = &updated.model.as_ref().unwrap().settings.directories[0];
    assert_eq!(grant.host_path, directory.path().canonicalize().unwrap());

    let remove = format!(
        "/conversations/{}/directories/{}/remove",
        record.id,
        grant.id.as_hex()
    );
    assert_eq!(
        app(&state)
            .oneshot(command(&remove, &token, "revision=1"))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        state
            .conversations
            .get(&record.id)
            .unwrap()
            .model
            .unwrap()
            .settings
            .directories
            .len(),
        1
    );
}

#[tokio::test]
async fn picker_cancellation_creates_no_grant() {
    let state = test_state();
    let token = connected(&state);
    let record = conversation(&state);
    state.folder_picker.queue(None);

    let response = app(&state)
        .oneshot(command(
            &format!("/conversations/{}/directories/pick", record.id),
            &token,
            "revision=1",
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .conversations
            .get(&record.id)
            .unwrap()
            .model
            .unwrap()
            .settings
            .directories
            .is_empty()
    );
}
