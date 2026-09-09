use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

use super::super::tests::{app, connected, document, navigation, test_state, text};

#[tokio::test]
async fn plans_navigation_and_revision_fragments_use_canonical_routes() {
    let state = test_state();
    let token = connected(&state);
    let record = state.conversations.create("Plans".to_owned()).unwrap();
    let starter = crate::workflows::seeds::production_seeds(crate::tests::test_environment_id())
        .into_iter()
        .find(|seed| seed.key.as_str() == "implement-saved-plan-v1")
        .unwrap();
    state.workflows.create(starter.definition).unwrap();
    let plan = state
        .documents
        .create_from_text(
            record.id,
            "Selected plan".to_owned(),
            "Immutable source text".to_owned(),
            None,
        )
        .unwrap();
    let path = format!("/plans/{}?revision=1", plan.id);
    let native = app(&state).oneshot(document(&path, &token)).await.unwrap();
    assert_eq!(native.status(), StatusCode::OK);
    let body = text(native).await;
    assert!(body.contains("id=\"conversation-detail\""));
    assert!(body.contains("id=\"plan-detail\""));
    let enhanced = app(&state)
        .oneshot(navigation(&path, &token))
        .await
        .unwrap();
    assert_eq!(enhanced.status(), StatusCode::OK);
    assert!(text(enhanced).await.contains("target=\"chat-main\""));
    let patch = Request::builder()
        .uri(&path)
        .header("Cookie", format!("powerplant_session={token}"))
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header("Accept", hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .unwrap();
    let fragment = app(&state).oneshot(patch).await.unwrap();
    assert_eq!(fragment.status(), StatusCode::OK);
    assert!(text(fragment).await.contains("target=\"plan-detail\""));
    let preview_path = super::super::workflow::implementation_href(&state, &plan, plan.current());
    state
        .documents
        .revise(
            &plan.id,
            1,
            "Changed source".to_owned(),
            "New text".to_owned(),
            None,
        )
        .unwrap();
    let preview = app(&state)
        .oneshot(document(&preview_path, &token))
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    let preview = text(preview).await;
    assert!(preview.contains("Revision 1"));
    assert!(preview.contains(&plan.current().content_hash.as_str()));
    assert!(state.workflow_runs.for_conversation(&record.id).is_empty());
    let plans = app(&state)
        .oneshot(document(
            &format!("/conversations/{}?plans=true", record.id),
            &token,
        ))
        .await
        .unwrap();
    assert!(text(plans).await.contains("data-documents-open=\"true\""));
}

#[tokio::test]
async fn a_large_escaped_plan_keeps_a_bounded_canonical_navigation_representation() {
    let state = test_state();
    let token = connected(&state);
    let record = state.conversations.create("Large plan".to_owned()).unwrap();
    let plan = state
        .documents
        .create_from_text(
            record.id,
            "Large plan".to_owned(),
            "&".repeat(64 * 1024),
            None,
        )
        .unwrap();
    let response = app(&state)
        .oneshot(navigation(&format!("/plans/{}", plan.id), &token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = text(response).await;
    assert!(html.len() <= 1024 * 1024);
    assert!(html.contains("target=\"chat-main\""));
    assert!(html.contains("id=\"plan-detail\""));
}

#[test]
fn scoped_plan_actions_reject_source_substitution_and_unknown_authority_fields() {
    let state = test_state();
    let mut record = state.conversations.create("Plans".to_owned()).unwrap();
    record
        .messages
        .push(crate::conversations::ConversationMessage {
            role: crate::conversations::MessageRole::Assistant,
            status: crate::conversations::MessageStatus::Complete,
            text: String::new(),
            error: None,
            request: None,
        });
    let first = state
        .documents
        .create_from_text(record.id, "First".to_owned(), "First text".to_owned(), None)
        .unwrap();
    let second = state
        .documents
        .create_from_text(
            record.id,
            "Second".to_owned(),
            "Second text".to_owned(),
            None,
        )
        .unwrap();
    let arguments = serde_json::json!({"title":"Revised", "markdown":"Revised text", "document_id":second.id.as_hex(), "revision":1});
    assert_eq!(
        super::publish(
            &state,
            &record,
            0,
            "revise_plan",
            arguments,
            None,
            Some(super::Scope::Revise(first.id, 1))
        )
        .err(),
        Some(crate::conversations::DocumentError::Source)
    );
    let arguments =
        serde_json::json!({"title":"Plan", "markdown":"Plan text", "approve_execution":true});
    assert_eq!(
        super::publish(&state, &record, 0, "create_plan", arguments, None, None).err(),
        Some(crate::conversations::DocumentError::Content)
    );
    assert_eq!(state.documents.get(&first.id), Some(first));
    assert_eq!(state.documents.get(&second.id), Some(second));
    assert!(state.workflow_runs.for_conversation(&record.id).is_empty());
}

#[tokio::test]
async fn a_plan_request_from_a_long_reply_uses_only_the_selected_source_and_no_execution_tools() {
    let mut state = test_state();
    let token = connected(&state);
    let session = super::super::tests::session_id(&token);
    let backend = crate::tests::ScriptedBackend::events(vec![Ok(
        crate::providers::ModelEvent::ToolCall {
            id: "explicit-plan".to_owned(),
            name: "create_plan".to_owned(),
            arguments: serde_json::json!({"title":"Generated plan", "markdown":"Exact generated contents"}),
        },
    )]);
    state.chat = std::sync::Arc::new(crate::providers::ChatBackend::Scripted(backend.clone()));
    let record = state
        .conversations
        .create("Source conversation".to_owned())
        .unwrap();
    let job = state
        .sessions
        .begin_conversation_job(&session, record.id, 1)
        .unwrap();
    let selection = crate::providers::ModelSelection::new(
        crate::providers::ProviderKind::Xai,
        "grok-4.6".to_owned(),
        state
            .models_dev
            .effective_effort(crate::providers::ProviderKind::Xai, "grok-4.6", None),
    )
    .unwrap();
    let record = state
        .conversations
        .begin_message(
            &record.id,
            record.revision,
            selection,
            job.id(),
            "Private earlier context".to_owned(),
        )
        .unwrap();
    let source = "Selected source. ".repeat(3072);
    state
        .conversations
        .settle_message(
            &record.id,
            job.id(),
            source.clone(),
            crate::conversations::MessageStatus::Complete,
            None,
        )
        .unwrap();
    state
        .sessions
        .finish_conversation_job(&session, record.id, job.id());
    let record = state.conversations.get(&record.id).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/conversations/{}/plans/from-message", record.id))
        .header("Cookie", format!("powerplant_session={token}"))
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header("Accept", hypergraft::MEDIA_TYPE)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "revision={}&message_index=1&title=Generated+plan&request=Create+a+plan",
            record.revision
        )))
        .unwrap();
    let response = app(&state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state
            .conversations
            .get(&record.id)
            .unwrap()
            .active_job
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(backend.last_tools(), ["create_plan"]);
    let history = backend.last_history();
    assert_eq!(history.len(), 1);
    assert!(history[0].text.contains(&source));
    assert!(!history[0].text.contains("Private earlier context"));
    assert_eq!(state.documents.list_for_conversation(record.id).len(), 1);
    assert!(state.workflow_runs.for_conversation(&record.id).is_empty());
    assert_eq!(
        state.conversations.get(&record.id).unwrap().messages[1].text,
        source
    );
}

#[tokio::test]
async fn native_plan_requests_create_no_action_or_conversation_job() {
    let state = test_state();
    let token = connected(&state);
    let record = state.conversations.create("Plans".to_owned()).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/conversations/{}/plans/request", record.id))
        .header("Cookie", format!("powerplant_session={token}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "revision={}&title=Plan&request=Create+a+plan",
            record.revision
        )))
        .unwrap();
    let response = app(&state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(state.documents.list_for_conversation(record.id).is_empty());
    assert_eq!(state.conversations.get(&record.id), Some(record));
}
