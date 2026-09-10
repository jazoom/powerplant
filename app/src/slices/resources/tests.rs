use askama::Template;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

#[tokio::test]
async fn resources_support_document_and_navigation_without_a_provider() {
    let state = crate::tests::test_state(crate::config::RuntimeConfig::development());
    let app = crate::slices::router()
        .layer(from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state);
    for (kind, status) in [
        (None, StatusCode::OK),
        (Some("navigation"), StatusCode::OK),
        (Some("patch"), StatusCode::BAD_REQUEST),
    ] {
        let mut request = Request::builder().uri("/resources");
        if let Some(kind) = kind {
            request = request
                .header(hypergraft::GRAFT_REQUEST, kind)
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
        }
        assert_eq!(
            app.clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            status
        );
    }
}

fn fixture() -> (
    crate::state::AppState,
    crate::conversations::ConversationId,
    String,
    crate::presets::PresetId,
) {
    let state = crate::tests::test_state(crate::config::RuntimeConfig::development());
    let conversation = state
        .conversations
        .create("Springfield".to_owned())
        .expect("conversation")
        .id;
    let workflow = state
        .workflows
        .create(crate::tests::test_named_definition("Review current code"))
        .expect("workflow");
    let token = crate::workflows::WorkflowSelection {
        workflow_id: workflow.id,
        definition_version: workflow.definition_version,
    }
    .as_token();
    let settings = crate::execution::ExecutionSettings::new(
        crate::providers::ModelSelection::new(
            crate::providers::ProviderKind::Xai,
            "grok-4.6".to_owned(),
            None,
        )
        .unwrap(),
        "Follow the brief.".to_owned(),
        Vec::new(),
        crate::tests::test_environment_id(),
    )
    .unwrap();
    let preset = state
        .presets
        .create(
            "Careful review",
            settings,
            crate::presets::PresetProvenance::Draft,
        )
        .expect("preset")
        .id;
    (state, conversation, token, preset)
}

/// Context selects the return destination only. Forged or deleted
/// identifiers fall back to the catalogue without substitution, and stale
/// resource identities report an error rather than another record.
#[test]
fn resource_context_validates_identifiers_without_substitution() {
    let (state, conversation, token, preset) = fixture();
    let conversation_hex = conversation.as_hex();
    let preset_hex = preset.as_hex();

    let contextual =
        super::page::ResourcesPage::new(&state, Some(conversation), &token, &preset_hex)
            .render()
            .unwrap();
    assert!(contextual.contains(">Back to conversation<"));
    assert!(contextual.contains(&format!("href=\"/conversations/{conversation_hex}\"")));
    assert!(contextual.contains(&format!(
        "/conversations/{conversation_hex}/workflow?workflow={token}"
    )));
    // The preset handoff stays on the canonical GET with both identifiers.
    // Selection never mints a preview or applies settings from this GET.
    assert!(contextual.contains(&format!(
        "/resources?conversation={conversation_hex}&#38;preset={preset_hex}"
    )));
    assert!(!contextual.contains("settings/presets/preview"));
    assert!(!contextual.contains("name=\"preset_preview\""));
    assert!(contextual.contains("id=\"resources-selected-workflow\""));
    assert!(contextual.contains("id=\"resources-preset-preview\""));
    assert!(!contextual.contains("id=\"resources-destination\""));
    assert!(!contextual.contains("id=\"resources-preset-destination\""));

    let plain = super::page::ResourcesPage::new(&state, None, "", "")
        .render()
        .unwrap();
    assert!(plain.contains(">Back to conversations<"));
    assert!(plain.contains("href=\"/conversations\""));
    assert!(!plain.contains("id=\"resources-destination\""));
    assert!(!plain.contains("id=\"resources-preset-destination\""));

    let chooser = super::page::ResourcesPage::new(&state, None, &token, "")
        .render()
        .unwrap();
    assert!(chooser.contains("id=\"resources-destination\""));
    assert!(chooser.contains(&format!(
        "/conversations/{conversation_hex}/workflow?workflow={token}"
    )));
    assert!(!chooser.contains("id=\"resources-selected-workflow\""));
    // The chooser retains the selected resource without starting work.
    assert!(!chooser.contains("action=\"/conversations/"));

    let preset_chooser = super::page::ResourcesPage::new(&state, None, "", &preset_hex)
        .render()
        .unwrap();
    assert!(preset_chooser.contains("id=\"resources-preset-destination\""));
    assert!(preset_chooser.contains(&conversation_hex));
    assert!(preset_chooser.contains(&preset_hex));
    assert!(!preset_chooser.contains("settings/presets/preview"));

    // A combined query keeps separate handoffs for each selection.
    let both = super::page::ResourcesPage::new(&state, None, &token, &preset_hex)
        .render()
        .unwrap();
    assert!(both.contains("id=\"resources-destination\""));
    assert!(both.contains("id=\"resources-preset-destination\""));
    assert!(both.contains(&format!(
        "/conversations/{conversation_hex}/workflow?workflow={token}"
    )));
    assert!(both.contains(&format!(
        "/resources?conversation={conversation_hex}&#38;preset={preset_hex}"
    )));

    let unknown =
        crate::conversations::ConversationId::parse("0123456789abcdef0123456789abcdef").unwrap();
    assert!(state.conversations.get(&unknown).is_none());
    let fallback = super::page::ResourcesPage::new(&state, Some(unknown), &token, "")
        .render()
        .unwrap();
    assert!(fallback.contains(">Back to conversations<"));
    assert!(!fallback.contains(">Back to conversation<"));
    assert!(fallback.contains("id=\"resources-destination\""));

    let stale_workflow = super::page::ResourcesPage::new(&state, Some(conversation), "00:00", "")
        .render()
        .unwrap();
    assert!(stale_workflow.contains("That workflow is no longer available."));
    assert!(!stale_workflow.contains("id=\"resources-destination\""));

    let stale_preset = super::page::ResourcesPage::new(
        &state,
        Some(conversation),
        "",
        "0123456789abcdef0123456789abcdef",
    )
    .render()
    .unwrap();
    assert!(stale_preset.contains("That preset is no longer available."));
}

/// Resource GET representations stay non-mutating: they create no
/// conversation, start no workflow and mint no preset preview token.
#[tokio::test]
async fn resource_get_creates_no_records_and_supports_safe_representations() {
    let (state, conversation, token, preset) = fixture();
    let app = crate::slices::router()
        .layer(from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone());
    let conversations_before = state.conversations.list().len();
    let uri = format!(
        "/resources?conversation={}&workflow={token}&preset={}",
        conversation.as_hex(),
        preset.as_hex()
    );
    for (kind, status) in [
        (None, StatusCode::OK),
        (Some("navigation"), StatusCode::OK),
        (Some("patch"), StatusCode::BAD_REQUEST),
    ] {
        let mut request = Request::builder().uri(&uri);
        if let Some(kind) = kind {
            request = request
                .header(hypergraft::GRAFT_REQUEST, kind)
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status == StatusCode::OK {
            let body = String::from_utf8(
                to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap();
            assert!(body.contains(">Back to conversation<"));
            // Preset handoff posts through the existing session-bound
            // preview command; the GET mints no preview token itself.
            assert!(!body.contains("name=\"preset_preview\""));
            assert!(!body.contains("settings/presets/preview"));
        }
    }
    // Forged conversation identifiers fall back without reflection and
    // without creating a record.
    for uri in [
        "/resources?conversation=not-a-conversation-id".to_owned(),
        "/resources?conversation=https%3A%2F%2Fexample.com%2Fevil".to_owned(),
        "/resources?conversation=0123456789abcdef0123456789abcdef".to_owned(),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = String::from_utf8(
            to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains(">Back to conversations<"), "{uri}");
        assert!(!body.contains(">Back to conversation<"), "{uri}");
        assert!(!body.contains("example.com/evil"), "{uri}");
    }
    assert_eq!(state.conversations.list().len(), conversations_before);
    assert!(state.conversations.get(&conversation).is_some());
    assert!(state.presets.get(&preset).is_some());
}

/// An empty catalogue still offers an explicit creation destination without
/// selecting a conversation automatically.
#[test]
fn resource_chooser_without_conversations_offers_creation() {
    let state = crate::tests::test_state(crate::config::RuntimeConfig::development());
    let workflow = state
        .workflows
        .create(crate::tests::test_named_definition("Review current code"))
        .expect("workflow");
    let token = crate::workflows::WorkflowSelection {
        workflow_id: workflow.id,
        definition_version: workflow.definition_version,
    }
    .as_token();
    assert!(state.conversations.list().is_empty());
    let body = super::page::ResourcesPage::new(&state, None, &token, "")
        .render()
        .unwrap();
    assert!(body.contains("id=\"resources-destination\""));
    assert!(body.contains("href=\"/conversations/new\""));
    assert!(!body.contains("/workflow?workflow="));
}
