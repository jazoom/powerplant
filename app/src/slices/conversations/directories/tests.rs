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
async fn sensitive_saved_grant_needs_exact_single_use_consent() {
    let mut state = test_state();
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("power-plant-data");
    std::fs::create_dir(&data).unwrap();
    state.local_data = crate::local_data::LocalDataReset::for_test(data.clone());
    let token = connected(&state);
    let record = conversation(&state);
    state.folder_picker.queue(Some(home.path().to_path_buf()));
    let path = format!("/conversations/{}/directories/pick", record.id);

    let preview = app(&state)
        .oneshot(command(&path, &token, "revision=1"))
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
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
    let body = text(preview).await;
    assert!(body.contains("Approve sensitive directory access"));
    assert!(body.contains(&data.to_string_lossy().replace('&', "&amp;")));
    let request = hidden_value(&body, "consent_request");
    let grant = hidden_value(&body, "pending_directory");
    let consent_path = format!("/conversations/{}/directories/consent", record.id);
    let consent_body = format!(
        "revision=1&consent_request={}&pending_directory={}",
        form_value(&request),
        form_value(&grant),
    );
    let approved = app(&state)
        .oneshot(command(&consent_path, &token, &consent_body))
        .await
        .unwrap();
    let approved_status = approved.status();
    let approved_body = text(approved).await;
    assert_eq!(approved_status, StatusCode::OK, "{approved_body}");
    let updated = state.conversations.get(&record.id).unwrap();
    assert_eq!(updated.model.unwrap().settings.directories.len(), 1);

    let replay = app(&state)
        .oneshot(command(&consent_path, &token, &consent_body))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let current = state.conversations.get(&record.id).unwrap();
    let settings = &current.model.as_ref().unwrap().settings;
    let grant = &settings.directories[0];
    let session = session_id(&token);
    assert!(
        state
            .access_consent
            .authorised_conversation(session, record.id, settings, grant)
    );
    state
        .sessions
        .advance_clock(crate::sessions::SESSION_LIFETIME + std::time::Duration::from_secs(1));
    let restored = app(&state)
        .oneshot(command(
            &format!(
                "/conversations/{}/directories/{}/consent",
                record.id,
                grant.id.as_hex()
            ),
            &token,
            &format!("revision={}", current.revision),
        ))
        .await
        .unwrap();
    assert_eq!(restored.status(), StatusCode::OK);
    assert!(state.sessions.contains_live(&session));
    assert!(
        !state
            .access_consent
            .authorised_conversation(session, record.id, settings, grant)
    );
    let preview = text(restored).await;
    let request = hidden_value(&preview, "consent_request");
    let pending = hidden_value(&preview, "pending_directory");
    let renamed = state
        .conversations
        .rename(&record.id, current.revision, "Renamed".to_owned())
        .unwrap();
    let body = format!(
        "revision={}&existing=true&consent_request={}&pending_directory={}",
        current.revision,
        form_value(&request),
        form_value(&pending)
    );
    let stale = app(&state)
        .oneshot(command(&consent_path, &token, &body))
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert!(
        !state
            .access_consent
            .authorised_conversation(session, record.id, settings, grant)
    );
    let body = body.replacen(
        &format!("revision={}", current.revision),
        &format!("revision={}", renamed.revision),
        1,
    );
    let approved = app(&state)
        .oneshot(command(&consent_path, &token, &body))
        .await
        .unwrap();
    assert_eq!(approved.status(), StatusCode::OK);
    assert!(
        state
            .access_consent
            .authorised_conversation(session, record.id, settings, grant)
    );
}

fn hidden_value(body: &str, name: &str) -> String {
    let marker = format!("name=\"{name}\"");
    let tail = &body[body.find(&marker).expect("hidden field") + marker.len()..];
    let value = &tail[tail.find("value=\"").expect("value") + 7..];
    value[..value.find('"').expect("value end")]
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&amp;", "&")
        .replace("&#x2F;", "/")
        .replace("&#x3D;", "=")
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
