use super::super::tests::*;
use crate::conversations::{ConversationId, ConversationStore};
use crate::providers::{ProviderConnection, ProviderKind};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn model_preference_changes_only_for_valid_patches_without_a_conversation() {
    let state = test_state();
    state
        .vault
        .put(ProviderConnection::with_key(
            ProviderKind::Deepseek,
            "test-key",
            "deepseek-v4-flash",
        ))
        .unwrap();
    let token = connected(&state);
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Deepseek, "deepseek-v4-pro", None)
        .unwrap();
    let fields = format!(
        "provider=deepseek&model=deepseek-v4-pro&thinking={}",
        effort.as_str()
    );
    let mut native = command("/conversations/new/model", &token, &fields);
    native.headers_mut().remove("graft-request");
    assert_eq!(
        app(&state).oneshot(native).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app(&state)
            .oneshot(document("/conversations/new/model", &token))
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    let selected = state.preferences.selected_provider(&state.vault).unwrap();
    assert_eq!(
        (selected.kind, selected.model.as_str(), selected.thinking),
        (ProviderKind::Xai, "grok-4.6", None)
    );
    for (fields, status) in [
        (fields.as_str(), StatusCode::OK),
        (
            "provider=missing&model=bad",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "provider=deepseek&model=unknown",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "provider=deepseek&model=deepseek-v4-pro&thinking=invalid",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let response = app(&state)
            .oneshot(command("/conversations/new/model", &token, fields))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert!(
            text(response)
                .await
                .contains("target=\"conversation-model-status\"")
        );
        let selected = state.preferences.selected_provider(&state.vault).unwrap();
        assert_eq!(selected.kind, ProviderKind::Deepseek);
        assert_eq!(selected.model, "deepseek-v4-pro");
        assert_eq!(selected.thinking.as_ref(), Some(&effort));
        assert!(state.conversations.list().is_empty());
    }
}

#[tokio::test]
async fn preference_write_failure_is_known_and_does_not_prevent_first_send() {
    let mut state = test_state();
    let dir = tempfile::tempdir().unwrap();
    state.preferences = std::sync::Arc::new(crate::preferences::Preferences::open(
        dir.path().to_path_buf(),
    ));
    let token = connected(&state);
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Xai, "grok-4.6", None)
        .unwrap();
    let fields = format!("provider=xai&model=grok-4.6&thinking={}", effort.as_str());
    let response = app(&state)
        .oneshot(command("/conversations/new/model", &token, &fields))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = text(response).await;
    assert!(body.contains("target=\"conversation-model-status\""));
    assert!(body.contains("Power Plant cannot store the model preference."));
    assert!(state.conversations.list().is_empty());

    let response = app(&state)
        .oneshot(command(
            "/conversations/new",
            &token,
            &format!("{fields}&action=send&message=Hello"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let record = state.conversations.list().pop().unwrap();
    let body = text(response).await;
    assert!(body.contains(&format!("location=\"/conversations/{}\"", record.id)));
    assert!(body.contains("Power Plant cannot store the model preference."));
    assert_eq!(record.messages[0].text, "Hello");
}

#[tokio::test]
async fn new_navigation_and_invalid_submissions_leave_no_record_or_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = test_state();
    state.conversations =
        std::sync::Arc::new(ConversationStore::open(dir.path().to_path_buf()).unwrap());
    let token = connected(&state);
    for request in [
        document("/conversations/new", &token),
        navigation("/conversations/new", &token),
        command("/conversations", &token, ""),
    ] {
        let response = app(&state).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.conversations.list().is_empty());
    }
    for body in [
        "action=send&message=",
        "action=save",
        "action=send&project=missing",
        "action=send&title=%20",
        "action=send&provider=missing&model=bad",
        "action=send&preset=missing",
        "action=unknown",
    ] {
        let response = app(&state)
            .oneshot(command("/conversations/new", &token, body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            text(response)
                .await
                .contains("target=\"conversation-detail\"")
        );
        assert!(state.conversations.list().is_empty());
        assert!(!dir.path().join("catalogue.json").exists());
    }
    assert_eq!(
        app(&state)
            .oneshot(document("/conversations", &token))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(state.conversations.list().is_empty());
}

#[tokio::test]
async fn invalid_first_submissions_preserve_all_local_choices_and_unsent_text() {
    let state = test_state();
    let token = connected(&state);
    let project = register_project(&state, "Context");
    let preset = state
        .agents
        .create(crate::agents::AgentDraft {
            name: "Local preset".to_owned(),
            instructions: String::new(),
            selection: None,
            tools: Vec::new(),
            network: crate::agents::NetworkAccess::None,
            directories: Vec::new(),
            primary_directory: String::new(),
        })
        .unwrap();
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Xai, "grok-4.6", None)
        .unwrap();
    for (action, title) in [("save", "Local title"), ("send", " ")] {
        let response = app(&state)
            .oneshot(command(
                "/conversations/new",
                &token,
                &format!(
                    "action={action}&project={}&title={title}&message=Unsent%20text&provider=xai&model=grok-4.6&thinking={}&preset={}",
                    project.id, effort.as_str(), preset.id
                ),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = text(response).await;
        assert!(body.contains("Unsent text</textarea>"));
        assert!(body.contains(&format!("value=\"{title}\"")));
        assert!(body.contains("value=\"grok-4.6\""));
        for value in [
            project.id.as_hex(),
            preset.id.as_hex(),
            "xai".to_owned(),
            effort.as_str().to_owned(),
        ] {
            let option = body
                .split(&format!("value=\"{value}\""))
                .nth(1)
                .unwrap()
                .split('>')
                .next()
                .unwrap();
            assert!(option.contains("selected"), "{value}");
        }
        assert!(!body.contains("location="));
        assert!(state.conversations.list().is_empty());
    }
}

#[tokio::test]
async fn first_send_persists_message_and_model_then_replaces_location() {
    let state = test_state();
    let token = connected(&state);
    let project = register_project(&state, "Context");
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Xai, "grok-4.6", None)
        .unwrap();
    let fields = format!(
        "action=send&project={}&provider=xai&model=grok-4.6&thinking={}",
        project.id,
        effort.as_str()
    );
    for message in [
        "%20",
        "%00",
        &"x".repeat(crate::conversations::MAXIMUM_MESSAGE_BYTES + 1),
    ] {
        let response = app(&state)
            .oneshot(command(
                "/conversations/new",
                &token,
                &format!("{fields}&message={message}"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(state.conversations.list().is_empty());
        assert!(!state.sessions.busy(&session_id(&token)));
    }
    let response = app(&state)
        .oneshot(command(
            "/conversations/new",
            &token,
            &format!("{fields}&message=Explain%20this"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    let record = state.conversations.list().pop().unwrap();
    assert!(body.contains(&format!("location=\"/conversations/{}\"", record.id)));
    assert!(body.contains("data-conversation-state=\"saved\""));
    assert!(body.contains("id=\"conversation-model-form\""));
    assert!(body.contains("id=\"conversation-model-search\""));
    assert_eq!(record.messages[0].text, "Explain this");
    assert_eq!(record.model.unwrap().selection.model, "grok-4.6");
    assert_eq!(record.projects, vec![project.id]);
    assert!(record.grants.is_empty());
    assert!(record.execution_target.is_none());
}

#[tokio::test]
async fn busy_session_and_full_store_reject_first_send_without_new_record() {
    let state = test_state();
    let token = connected(&state);
    let id = ConversationId::generate().unwrap();
    let job = state
        .sessions
        .begin_conversation_job(&session_id(&token), id, 1)
        .unwrap();
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Xai, "grok-4.6", None)
        .unwrap();
    let fields = format!(
        "action=send&provider=xai&model=grok-4.6&thinking={}&message=Hello",
        effort.as_str()
    );
    let response = app(&state)
        .oneshot(command("/conversations/new", &token, &fields))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(state.conversations.list().is_empty());
    state
        .sessions
        .finish_conversation_job(&session_id(&token), id, job.id());
    for _ in 0..128 {
        state.conversations.create("Saved".to_owned()).unwrap();
    }
    let response = app(&state)
        .oneshot(command("/conversations/new", &token, &fields))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.conversations.list().len(), 128);
    assert!(!state.sessions.busy(&session_id(&token)));
}
