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
