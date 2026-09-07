use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;

fn connected_state() -> AppState {
    let state = crate::tests::test_state(crate::config::RuntimeConfig::development());
    state
        .vault
        .put(crate::providers::ProviderConnection::with_key(
            crate::providers::ProviderKind::Xai,
            "test-key",
            "grok-4.6",
        ))
        .expect("provider");
    state
}

#[tokio::test]
async fn task_loop_launch_accepts_a_whole_list_without_a_task_index() {
    let state = connected_state();
    let conversation = state
        .conversations
        .create("Loop".to_owned())
        .expect("conversation");
    let document = state
        .documents
        .create_task_list_from_text(
            conversation.id,
            "Tasks".to_owned(),
            "# Tasks\n\n- [x] Done\n- [ ] Remaining\n".to_owned(),
            None,
        )
        .expect("tasks");
    let document_token = format!(
        "{}/1/{}",
        document.id.as_hex(),
        document.current().content_hash.as_str()
    );
    let workflow = state
        .workflows
        .create(workflows::seeds::ralph_task_loop_definition(
            crate::tests::test_environment_id(),
        ))
        .expect("workflow");
    let selection = WorkflowSelection {
        workflow_id: workflow.id,
        definition_version: workflow.definition_version,
    }
    .as_token();
    let view = launch_view(
        &state,
        &conversation,
        Some(&selection),
        None,
        "Implement",
        "",
        "",
        &document_token,
        "",
        "",
        "",
        &[],
        "",
    )
    .await;
    assert!(view.error.is_empty(), "{}", view.error);
    assert!(view.task_preview.contains("[x] Done"));
    assert!(
        resolve_launch_task(
            ExecutionMode::TaskList,
            &state,
            &conversation,
            &document_token,
            "",
            "",
            "1"
        )
        .is_err()
    );
    let token = crate::sessions::generate_session_token().expect("session");
    state.sessions.insert(token.id());
    let app = crate::slices::router()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone());
    let response = app.oneshot(Request::builder().method("POST")
        .uri(format!("/conversations/{}/workflow", conversation.id.as_hex()))
        .header(header::COOKIE, format!("powerplant_session={}", token.raw().as_str()))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch").header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from(format!("revision={}&workflow={selection}&preview_workflow={selection}&brief=Implement&target=&task_document={document_token}&preview_task_document={document_token}", conversation.revision))).expect("request"))
        .await.expect("response");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("text");
    assert!(
        body.contains("Choose an available target project."),
        "{body}"
    );
    assert!(
        state
            .task_loops
            .for_conversation(&conversation.id)
            .is_empty()
    );
}

#[test]
fn task_selection_rejects_foreign_checked_and_removed_items() {
    let state = connected_state();
    let conversation = state
        .conversations
        .create("Tasks".to_owned())
        .expect("conversation");
    let other = state
        .conversations
        .create("Other".to_owned())
        .expect("conversation");
    let markdown =
        "# Tasks\n\nShared context.\n\n- [x] Done.\n- [ ] First pending.\n- [ ] Selected.\n";
    let document = state
        .documents
        .create_task_list_from_text(
            conversation.id,
            "Tasks".to_owned(),
            markdown.to_owned(),
            None,
        )
        .expect("tasks");
    let id = document.id.as_hex();
    let hash = document.current().content_hash.as_str();
    let resolve = |record: &ConversationRecord, index: &str, hash: &str| {
        super::resolve_selected_task(&state, record, &id, "1", hash, index)
    };
    assert!(resolve(&other, "2", &hash).is_err());
    for index in ["0", "3", "-1", "4294967296"] {
        assert!(resolve(&conversation, index, &hash).is_err());
    }
    assert!(
        resolve(
            &conversation,
            "2",
            &workflows::artefacts::ObjectHash::of(b"other").as_str()
        )
        .is_err()
    );
    let revised = state
        .documents
        .revise(
            &document.id,
            1,
            "Tasks".to_owned(),
            "# Tasks\n\n- [ ] Different task.\n".to_owned(),
            None,
        )
        .expect("revision");
    let selected = resolve(&conversation, "2", &hash)
        .expect("selection")
        .expect("task");
    assert_eq!(selected.task_list, markdown);
    assert_eq!(selected.markdown, "- [ ] Selected.\n");
    state
        .documents
        .disassociate(&document.id, revised.current().revision, conversation.id)
        .expect("remove");
    assert!(resolve(&conversation, "2", &hash).is_err());
}

#[test]
fn saved_plan_selection_rejects_substitution_and_removal_but_pins_old_revisions() {
    let state = connected_state();
    let conversation = state
        .conversations
        .create("Workflow".to_owned())
        .expect("conversation");
    let plan = state
        .documents
        .create_from_text(
            conversation.id,
            "Implementation plan".to_owned(),
            "# Selected plan\n\nKeep this revision.\n".to_owned(),
            None,
        )
        .expect("plan");
    let base = workflows::seeds::sequential_team_definition(crate::tests::test_environment_id());
    let mut steps = base.steps().to_vec();
    let implementer = steps
        .iter_mut()
        .find(|step| step.key.as_str() == "implementer")
        .expect("implementer");
    let plan_input = implementer
        .inputs
        .iter_mut()
        .find(|input| input.kind == workflows::definition::ArtefactKind::Plan)
        .expect("plan input");
    plan_input.source = workflows::definition::ArtefactSource::LaunchInput {
        source: workflows::definition::LaunchInputSource::SavedPlan,
    };
    let definition = workflows::definition::WorkflowDefinition::from_parts(
        base.name().to_owned(),
        base.default_environment(),
        base.roles().to_vec(),
        steps,
    )
    .expect("definition");
    let token = super::plan_choice_token(&plan.id, plan.current());
    let resolve = |raw: &str| super::resolve_selected_plan(&state, &conversation, raw, &definition);
    assert!(resolve("").is_err());
    let tasks = state
        .documents
        .create_task_list_from_text(
            conversation.id,
            "Tasks".to_owned(),
            "# Tasks\n- [ ] Implement\n".to_owned(),
            None,
        )
        .expect("tasks");
    assert!(resolve(&super::plan_choice_token(&tasks.id, tasks.current())).is_err());
    let selected = resolve(&token).expect("selection").expect("plan");
    let mut changed: super::PlanChoiceToken = serde_json::from_str(&token).expect("token");
    changed.content_hash = workflows::artefacts::ObjectHash::of(b"substitute").as_str();
    assert!(resolve(&serde_json::to_string(&changed).expect("changed token")).is_err());
    assert!(super::resolve_selected_plan(&state, &conversation, &token, &base).is_err());
    let other = state
        .conversations
        .create("Other".to_owned())
        .expect("conversation");
    assert!(super::resolve_selected_plan(&state, &other, &token, &definition).is_err());

    let corrected = state
        .documents
        .revise(
            &plan.id,
            1,
            "Corrected plan".to_owned(),
            "A different plan.".to_owned(),
            None,
        )
        .expect("correction");
    assert_eq!(
        resolve(&token)
            .expect("old selection")
            .expect("plan")
            .content,
        selected.content
    );
    let imported = workflows::artefacts::import_saved_plan(
        workflows::RunId::generate().expect("run"),
        workflows::now_ms(),
        conversation.id,
        plan.id,
        &selected.reference,
        &selected.content,
        &state.workflow_artefacts,
    )
    .expect("import");
    state
        .documents
        .disassociate(&plan.id, corrected.current().revision, conversation.id)
        .expect("remove association");
    assert!(resolve(&token).is_err());
    let bytes = state
        .workflow_artefacts
        .get(&imported.object_hash)
        .expect("run-owned plan");
    assert_eq!(
        workflows::artefacts::parse_typed_payload(imported.kind, &bytes).expect("payload"),
        workflows::artefacts::TypedPayload::Plan(workflows::artefacts::payload::PlanArtefact {
            format_version: 1,
            markdown: selected.content,
        }),
    );
}

#[tokio::test]
async fn launch_rejects_stale_definitions_without_reserving_the_conversation() {
    let state = connected_state();
    let conversation = state
        .conversations
        .create("Workflow".to_owned())
        .expect("conversation");
    let mut seeds =
        workflows::seeds::production_seeds(crate::tests::test_environment_id()).into_iter();
    let workflow = state
        .workflows
        .create(seeds.next().expect("seed").definition)
        .expect("workflow");
    let selection = WorkflowSelection {
        workflow_id: workflow.id,
        definition_version: workflow.definition_version,
    }
    .as_token();
    state
        .workflows
        .update(
            &workflow.id,
            workflow.revision,
            seeds.next().expect("seed").definition,
        )
        .expect("update");
    let token = crate::sessions::generate_session_token().expect("session");
    state.sessions.insert(token.id());
    let app = crate::slices::router()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/conversations/{}/workflow",
                    conversation.id.as_hex()
                ))
                .header(
                    header::COOKIE,
                    format!("powerplant_session={}", token.raw().as_str()),
                )
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "revision={}&workflow={selection}&brief=Inspect&target=&phase=first&phase=second",
                    conversation.revision
                )))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("text");
    assert!(body.contains(ResolveWorkflowError::Changed.message()));
    assert_eq!(
        state
            .conversations
            .get(&conversation.id)
            .expect("conversation"),
        conversation
    );
    assert!(!state.sessions.busy(&token.id()));
    assert!(
        state
            .workflow_runs
            .for_conversation(&conversation.id)
            .is_empty()
    );
}

#[tokio::test]
async fn launch_sheet_supports_document_navigation_and_selection_preview() {
    let state = connected_state();
    let conversation = state
        .conversations
        .create("Workflow".to_owned())
        .expect("conversation");
    let token = crate::sessions::generate_session_token().expect("session");
    state.sessions.insert(token.id());
    let app = crate::slices::router()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone());
    let workflow = state
        .workflows
        .create(workflows::seeds::ralph_task_loop_definition(
            crate::tests::test_environment_id(),
        ))
        .expect("workflow");
    let selection = WorkflowSelection {
        workflow_id: workflow.id,
        definition_version: workflow.definition_version,
    }
    .as_token();
    let once = state
        .workflows
        .create(
            workflows::seeds::production_seeds(crate::tests::test_environment_id())
                .into_iter()
                .next()
                .expect("seed")
                .definition,
        )
        .expect("workflow");
    let once_selection = WorkflowSelection {
        workflow_id: once.id,
        definition_version: once.definition_version,
    }
    .as_token();
    let document = state
        .documents
        .create_task_list_from_text(
            conversation.id,
            "Review input".to_owned(),
            "# Tasks\n\n- [ ] Keep the selected task\n".to_owned(),
            None,
        )
        .expect("task list");
    let task_token = format!(
        "{}/1/{}",
        document.id.as_hex(),
        document.current().content_hash.as_str()
    );
    for selection in [&selection, &once_selection] {
        for stage in ["choose", "inputs", "review"] {
            for (representation, target) in [
                (None, "<!doctype html>"),
                (Some("navigation"), "chat-main"),
                (Some("patch"), "workflow-launch"),
            ] {
                let mut request = Request::builder()
            .uri(format!(
                "/conversations/{}/workflow?stage={stage}&workflow={selection}&brief=Preserve+this+brief&task_document={task_token}",
                conversation.id.as_hex()
            ))
            .header(
                header::COOKIE,
                format!("powerplant_session={}", token.raw().as_str()),
            );
                if let Some(representation) = representation {
                    request = request
                        .header(hypergraft::GRAFT_REQUEST, representation)
                        .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
                }
                let response = app
                    .clone()
                    .oneshot(request.body(Body::empty()).expect("request"))
                    .await
                    .expect("response");
                assert_eq!(response.status(), StatusCode::OK);
                let body = String::from_utf8(
                    to_bytes(response.into_body(), 1024 * 1024)
                        .await
                        .expect("body")
                        .to_vec(),
                )
                .expect("text");
                assert!(body.contains(target), "missing {target}");
                assert!(body.contains("Preserve this brief"));
                if stage == "review" {
                    assert!(body.contains("Workers receive selected inputs,"));
                    assert!(body.contains("workflow-start"));
                    if selection != &once_selection {
                        assert!(body.contains("Review input"));
                        assert!(body.contains(&task_token));
                    }
                }
            }
        }
    }
    for (query, error) in [
        (
            format!("stage=review&workflow={selection}&brief=Keep+this"),
            "Choose the required saved input before review.",
        ),
        (
            format!("stage=review&workflow={once_selection}&brief=Keep+this&phase=invalid"),
            "Choose a model for every model phase.",
        ),
        (
            format!("stage=review&workflow={once_selection}&brief=Keep+this&target=missing"),
            "The selected project is unavailable.",
        ),
        (
            "stage=inputs".to_owned(),
            "Choose a workflow before you continue.",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/conversations/{}/workflow?{query}",
                        conversation.id.as_hex()
                    ))
                    .header(
                        header::COOKIE,
                        format!("powerplant_session={}", token.raw().as_str()),
                    )
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = String::from_utf8(
            to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("body")
                .to_vec(),
        )
        .expect("text");
        assert!(body.contains(error));
        assert!(!body.contains("id=\"workflow-start\""));
        if query.contains("brief=") {
            assert!(body.contains("Keep this"));
        }
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/conversations/{}/workflow?stage=review&workflow=stale",
                    conversation.id.as_hex()
                ))
                .header(
                    header::COOKIE,
                    format!("powerplant_session={}", token.raw().as_str()),
                )
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("text");
    assert!(body.contains("This workflow selection is no longer available."));
    assert!(!state.sessions.busy(&token.id()));
    assert!(
        state
            .workflow_runs
            .for_conversation(&conversation.id)
            .is_empty()
    );
    assert!(
        state
            .task_loops
            .for_conversation(&conversation.id)
            .is_empty()
    );
    assert_eq!(
        state
            .conversations
            .get(&conversation.id)
            .expect("conversation"),
        conversation
    );
}
