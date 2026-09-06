use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    config::RuntimeConfig,
    providers::{ModelSelection, ProviderConnection, ProviderKind},
    sessions,
    state::AppState,
};

fn test_state() -> AppState {
    crate::tests::test_state(RuntimeConfig::development())
}

fn app(state: &AppState) -> axum::Router {
    crate::slices::router()
        .layer(from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone())
}

fn connected(state: &AppState) -> String {
    state
        .vault
        .put(ProviderConnection::with_key(
            ProviderKind::Xai,
            "test-key",
            "grok-4.6",
        ))
        .expect("provider");
    let token = sessions::generate_session_token().expect("token");
    state.sessions.insert(token.id());
    token.raw().as_str().to_owned()
}

fn cookie(token: &str) -> String {
    format!("powerplant_session={token}")
}

fn document(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header(header::COOKIE, cookie(token))
        .body(Body::empty())
        .expect("request")
}

fn navigation(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header(header::COOKIE, cookie(token))
        .header(hypergraft::GRAFT_REQUEST, "navigation")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .expect("request")
}

fn command(path: &str, token: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::COOKIE, cookie(token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from(body.to_owned()))
        .expect("request")
}

async fn text(response: axum::response::Response) -> String {
    String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("text")
}

#[tokio::test]
async fn catalogue_and_new_conversation_use_document_and_navigation_representations() {
    let state = test_state();
    let token = connected(&state);

    let document_response = app(&state)
        .oneshot(document("/conversations", &token))
        .await
        .expect("document");
    assert_eq!(document_response.status(), StatusCode::OK);
    let document_body = text(document_response).await;
    assert!(document_body.contains("Conversation history stays on this local"));
    assert_eq!(document_body.matches("id=\"chat-main\"").count(), 1);

    let navigation_response = app(&state)
        .oneshot(navigation("/conversations/new", &token))
        .await
        .expect("navigation");
    assert_eq!(navigation_response.status(), StatusCode::OK);
    let navigation_body = text(navigation_response).await;
    assert!(navigation_body.contains("operation=\"children\" target=\"chat-main\""));
}

#[tokio::test]
async fn create_rename_and_delete_use_independent_conversation_identity() {
    let state = test_state();
    let token = connected(&state);

    let create = app(&state)
        .oneshot(command("/conversations", &token, "title=First+discussion"))
        .await
        .expect("create");
    assert_eq!(create.status(), StatusCode::OK);
    let create_body = text(create).await;
    let record = state.conversations.list().pop().expect("record");
    let path = format!("/conversations/{}", record.id.as_hex());
    assert!(create_body.contains(&format!("navigate=\"{path}\"")));

    let detail = app(&state)
        .oneshot(document(&path, &token))
        .await
        .expect("detail");
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_body = text(detail).await;
    assert_eq!(detail_body.matches("id=\"conversation-detail\"").count(), 1);

    let rename = app(&state)
        .oneshot(command(
            &format!("{path}/rename"),
            &token,
            &format!("title=Renamed&revision={}", record.revision),
        ))
        .await
        .expect("rename");
    assert_eq!(rename.status(), StatusCode::OK);
    let rename_body = text(rename).await;
    assert!(rename_body.contains("target=\"conversation-detail\""));
    assert!(!rename_body.contains("id=\"conversation-detail\""));
    assert!(rename_body.contains("title=\"Renamed | Power Plant\""));
    let renamed = state.conversations.get(&record.id).expect("renamed");
    assert_eq!(renamed.title, "Renamed");

    let delete = app(&state)
        .oneshot(command(
            &format!("{path}/delete"),
            &token,
            &format!("revision={}", renamed.revision),
        ))
        .await
        .expect("delete");
    assert_eq!(delete.status(), StatusCode::OK);
    assert!(text(delete).await.contains("navigate=\"/conversations\""));
    assert!(state.conversations.get(&record.id).is_none());
}

#[tokio::test]
async fn send_persists_a_project_free_reply() {
    let state = test_state();
    let token = connected(&state);
    let record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    let model = "grok-4.6".to_owned();
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Xai, &model, None);
    let selection = ModelSelection::new(ProviderKind::Xai, model, effort).expect("selection");
    let record = state
        .conversations
        .select_model(&record.id, record.revision, selection)
        .expect("selection saved");
    let path = format!("/conversations/{}/messages", record.id.as_hex());

    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("revision={}&message=Hello", record.revision),
        ))
        .await
        .expect("send");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        text(response)
            .await
            .contains("target=\"conversation-detail\"")
    );

    for _ in 0..20 {
        let current = state.conversations.get(&record.id).expect("conversation");
        if current.active_job.is_none() {
            assert_eq!(current.messages.len(), 2);
            assert_eq!(current.messages[1].text, "Hello from Power Plant.");
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("reply did not settle");
}

#[tokio::test]
async fn observation_uses_the_page_route_and_cancel_needs_only_the_job_identity() {
    let state = test_state();
    let token = connected(&state);
    let owner = sessions::generate_session_token().expect("owner");
    state.sessions.insert(owner.id());
    let record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("record");
    let job = state
        .sessions
        .begin_conversation_job(&owner.id(), record.id, 1)
        .expect("job");
    let selection =
        ModelSelection::new(ProviderKind::Xai, "grok-4.6".to_owned(), None).expect("model");
    state
        .conversations
        .begin_message(
            &record.id,
            record.revision,
            selection,
            job.id(),
            "Question".to_owned(),
        )
        .expect("begin");
    let path = format!("/conversations/{}", record.id);
    let observe = format!("{path}?job={}&cursor=0", job.id());
    for request in [document(&observe, &token), navigation(&observe, &token)] {
        let response = app(&state).oneshot(request).await.expect("page");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(text(response).await.contains("Question"));
    }
    let response = app(&state)
        .oneshot(command(
            &format!("{path}/cancel"),
            &token,
            &format!("job={}", job.id()),
        ))
        .await
        .expect("cancel");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(job.cancel_requested());
    state
        .conversations
        .settle_message(
            &record.id,
            job.id(),
            String::new(),
            crate::conversations::MessageStatus::Interrupted,
        )
        .expect("settle");
    state
        .sessions
        .finish_conversation_job(&owner.id(), record.id, job.id());
    let request = Request::builder()
        .uri(&observe)
        .header(header::COOKIE, cookie(&token))
        .header("Graft-Request", "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .expect("request");
    let response = app(&state)
        .oneshot(request)
        .await
        .expect("final observation");
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("target=\"conversation-detail\""));
    assert!(!body.contains("navigate="));
}

#[tokio::test]
async fn stale_rename_returns_a_conflict_without_replacing_the_current_title() {
    let state = test_state();
    let token = connected(&state);
    let record = state
        .conversations
        .create("First discussion".to_owned())
        .expect("record");
    state
        .conversations
        .rename(&record.id, record.revision, "Current title".to_owned())
        .expect("current");
    let path = format!("/conversations/{}/rename", record.id.as_hex());

    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("title=Stale&revision={}", record.revision),
        ))
        .await
        .expect("rename");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = text(response).await;
    assert!(body.contains("target=\"conversation-detail\""));
    assert!(body.contains("Current title"));
    assert_eq!(
        state.conversations.get(&record.id).expect("current").title,
        "Current title"
    );
}
