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
            &format!("/conversations/{}/plans", record.id),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(plans.status(), StatusCode::OK);
    let plans = text(plans).await;
    assert!(plans.contains("id=\"plans-detail\""));
    assert!(plans.contains("Changed source"));
    assert!(plans.contains("Revision 2"));
    assert!(plans.contains("Open plan"));
    assert!(plans.contains("Remove"));
    assert!(plans.contains("Create a plan"));
    assert!(plans.contains("Add your own plan"));
    assert!(plans.contains("Standalone task lists"));
    // The Plans header action stays highlighted while the companion is open.
    assert!(plans.contains("aria-current=\"page\""));
    let legacy = app(&state)
        .oneshot(document(
            &format!("/conversations/{}?plans=true", record.id),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::OK);
    assert!(text(legacy).await.contains("id=\"plans-detail\""));
    let enhanced = app(&state)
        .oneshot(navigation(
            &format!("/conversations/{}/plans", record.id),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(enhanced.status(), StatusCode::OK);
    assert!(text(enhanced).await.contains("target=\"chat-main\""));
    let patch = Request::builder()
        .uri(format!("/conversations/{}/plans", record.id))
        .header("Cookie", format!("powerplant_session={token}"))
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header("Accept", hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .unwrap();
    let fragment = app(&state).oneshot(patch).await.unwrap();
    assert_eq!(fragment.status(), StatusCode::OK);
    let fragment = text(fragment).await;
    assert!(fragment.contains("target=\"conversation-detail\""));
    assert!(fragment.contains("id=\"plans-detail\""));
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

#[tokio::test]
async fn plans_request_companions_validate_mode_and_source_message() {
    let state = test_state();
    let token = connected(&state);
    let session = super::super::tests::session_id(&token);
    let record = state.conversations.create("Requests".to_owned()).unwrap();

    let empty = app(&state)
        .oneshot(document(
            &format!("/conversations/{}/plans", record.id),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(empty.status(), StatusCode::OK);
    let empty = text(empty).await;
    assert!(empty.contains("No plans yet."));
    assert!(empty.contains("A conversation does not need a plan."));

    for (mode, action, submit) in [
        ("create", "plans/request", "Create plan"),
        ("paste", "plans/text", "Add plan"),
    ] {
        let response = app(&state)
            .oneshot(document(
                &format!("/conversations/{}/plans/request?mode={mode}", record.id),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = text(response).await;
        assert!(body.contains("id=\"plan-request-detail\""));
        assert!(body.contains(&format!("/conversations/{}/{action}", record.id)));
        assert!(body.contains(submit));
        assert!(body.contains("Back to plans"));
    }

    let unknown = app(&state)
        .oneshot(document(
            &format!("/conversations/{}/plans/request?mode=review", record.id),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        text(unknown)
            .await
            .contains("Choose how to create the plan.")
    );

    let missing = app(&state)
        .oneshot(document(
            &format!(
                "/conversations/{}/plans/request?mode=from&message_index=0",
                record.id
            ),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        text(missing)
            .await
            .contains("Select a completed assistant message as the plan source.")
    );

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
            "User prompt".to_owned(),
        )
        .unwrap();
    state
        .conversations
        .settle_message(
            &record.id,
            job.id(),
            "Exact source reply.".to_owned(),
            crate::conversations::MessageStatus::Complete,
            None,
        )
        .unwrap();
    state
        .sessions
        .finish_conversation_job(&session, record.id, job.id());
    let record = state.conversations.get(&record.id).unwrap();

    let from = app(&state)
        .oneshot(document(
            &format!(
                "/conversations/{}/plans/request?mode=from&message_index=1",
                record.id
            ),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(from.status(), StatusCode::OK);
    let from = text(from).await;
    assert!(from.contains("id=\"plan-request-detail\""));
    assert!(from.contains("Source reply 2"));
    assert!(from.contains("Exact source reply."));
    assert!(from.contains("Response 2"));
    assert!(from.contains(&format!("/conversations/{}/plans/from-message", record.id)));

    for raw in ["0", "9", "not-an-index"] {
        let rejected = app(&state)
            .oneshot(document(
                &format!(
                    "/conversations/{}/plans/request?mode=from&message_index={raw}",
                    record.id
                ),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            text(rejected)
                .await
                .contains("Select a completed assistant message as the plan source.")
        );
    }

    // A tampered source index cannot dispatch a plan preparation job.
    let forged = Request::builder()
        .method("POST")
        .uri(format!("/conversations/{}/plans/from-message", record.id))
        .header("Cookie", format!("powerplant_session={token}"))
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header("Accept", hypergraft::MEDIA_TYPE)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "revision={}&message_index=9&title=Forged&request=Forged",
            record.revision
        )))
        .unwrap();
    let forged = app(&state).oneshot(forged).await.unwrap();
    assert_eq!(forged.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let forged = text(forged).await;
    assert!(forged.contains("id=\"plan-request-detail\""));
    assert!(forged.contains("Select a completed assistant message as the plan source."));
    assert!(state.documents.list_for_conversation(record.id).is_empty());
    assert!(
        state
            .conversations
            .get(&record.id)
            .unwrap()
            .active_job
            .is_none()
    );
}

#[test]
fn plan_sections_preserve_markdown_structure_and_revision_identity() {
    use super::super::page::{PlanDocumentPage, split_plan_sections};
    let state = test_state();
    let record = state.conversations.create("Sections".to_owned()).unwrap();
    let content = (0..3000)
        .map(|line| format!("# Heading\n- item {line}\n"))
        .collect::<String>();
    assert!(content.len() > super::super::page::PLAN_SECTION_CHARS);
    let plan = state
        .documents
        .create_from_text(record.id, "Large".to_owned(), content.clone(), None)
        .unwrap();
    let chunks = split_plan_sections(&content);
    assert!(chunks.len() > 1);
    assert_eq!(chunks.concat(), content);
    for chunk in &chunks {
        assert!(chunk.ends_with('\n'));
    }
    let first = PlanDocumentPage::from_document(&plan, 1, content.clone(), 0, "");
    let second = PlanDocumentPage::from_document(&plan, 1, content.clone(), 1, "");
    assert!(first.sections > 1);
    assert_eq!(first.sections, chunks.len());
    assert_eq!(first.document_revision, 1);
    assert_eq!(second.document_revision, 1);
    assert_eq!(first.content, content);
    assert_ne!(first.content_html, second.content_html);
    assert!(first.section_next.contains("&section=1"));
    assert!(second.section_prev.contains("revision=1"));
    if chunks.len() > 2 {
        assert!(second.section_next.contains("&section=2"));
    } else {
        assert!(second.section_next.is_empty());
    }
    let last_index = chunks.len() - 1;
    let last = PlanDocumentPage::from_document(&plan, 1, content.clone(), last_index, "");
    assert!(last.section_next.is_empty());
    if last_index == 1 {
        assert!(last.section_prev.contains("revision=1"));
    } else {
        assert!(
            last.section_prev
                .contains(&format!("&section={}", last_index - 1))
        );
    }
}

#[test]
fn plan_detail_uses_canonical_copy_and_escapes_untrusted_content() {
    use super::super::page::PlanDocumentPage;
    use askama::Template;
    let state = test_state();
    let record = state.conversations.create("Copy".to_owned()).unwrap();
    let plan = state
        .documents
        .create_from_text(
            record.id,
            "Plan <script>alert(1)</script>".to_owned(),
            "Body with <img src=x onerror=alert(1)> markup.".to_owned(),
            None,
        )
        .unwrap();
    let content = state.documents.content(&plan, 1).unwrap();
    let view =
        PlanDocumentPage::from_document(&plan, 1, content, 0, "").with_context(&state, &plan);
    let html = view.contents().render().unwrap();
    assert!(html.contains("Proposed approach"));
    assert!(html.contains("not execution approval"));
    assert!(!html.contains("Not execution approval"));
    assert!(html.contains(">Export<"));
    assert!(!html.contains("Export for Pi"));
    assert!(html.contains("Back to plans"));
    assert!(!html.contains("Back to conversation"));
    assert!(html.contains("Revise plan"));
    assert!(html.contains("Independent review"));
    assert!(html.contains("Plan history and source"));
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(!html.contains("<img src=x"));
}

#[tokio::test]
async fn plan_continuation_rejects_unknown_sections_without_state_change() {
    let state = test_state();
    let token = connected(&state);
    let record = state
        .conversations
        .create("Continuation".to_owned())
        .unwrap();
    let plan = state
        .documents
        .create_from_text(record.id, "Small".to_owned(), "Small text".to_owned(), None)
        .unwrap();
    for section in ["not-a-section", "9999", "-1", "1"] {
        let response = app(&state)
            .oneshot(document(
                &format!("/plans/{}?revision=1&section={section}", plan.id),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            text(response)
                .await
                .contains("Choose an available plan section.")
        );
    }
    let current = app(&state)
        .oneshot(document(&format!("/plans/{}?revision=9", plan.id), &token))
        .await
        .unwrap();
    assert_eq!(current.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        text(current)
            .await
            .contains("Choose an available plan revision.")
    );
    assert_eq!(state.documents.get(&plan.id).unwrap().current_revision(), 1);
}

#[tokio::test]
async fn task_continuation_validates_preamble_sections() {
    use super::super::page::{PlanDocumentPage, split_plan_sections};
    let state = test_state();
    let token = connected(&state);
    let record = state.conversations.create("Tasks".to_owned()).unwrap();
    let mut markdown = String::from("# Tasks\nSmall intro.\n");
    for index in 0..250 {
        use std::fmt::Write;
        let _ = writeln!(
            markdown,
            "- [ ] Task {index:03} with padding {}",
            "x".repeat(80)
        );
    }
    assert!(markdown.len() > super::super::page::PLAN_SECTION_CHARS);
    let plan = state
        .documents
        .create_task_list_from_text(record.id, "Tasks".to_owned(), markdown.clone(), None)
        .unwrap();
    let content = state.documents.content(&plan, 1).unwrap();
    let list = crate::workflows::task_list::parse(&content).unwrap();
    assert!(split_plan_sections(&list.preamble).len() == 1);
    assert!(split_plan_sections(&content).len() > 1);
    let view = PlanDocumentPage::from_document(&plan, 1, content, 0, "");
    assert_eq!(view.sections, 1);
    // Only the preamble section exists, so section 1 is unavailable even
    // though the complete task markdown spans multiple section lengths.
    let response = app(&state)
        .oneshot(document(
            &format!("/plans/{}?revision=1&section=1", plan.id),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        text(response)
            .await
            .contains("Choose an available plan section.")
    );
}
