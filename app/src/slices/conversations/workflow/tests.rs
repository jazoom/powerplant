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
    for (representation, target) in [
        (None, "<!doctype html>"),
        (Some("navigation"), "chat-main"),
        (Some("patch"), "workflow-launch"),
    ] {
        let mut request = Request::builder()
            .uri(format!(
                "/conversations/{}/workflow?brief=Preserve+this+brief&phase=first&phase=second",
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
    }
    assert_eq!(
        state
            .conversations
            .get(&conversation.id)
            .expect("conversation"),
        conversation
    );
}
