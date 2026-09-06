use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    agents::{AgentDraft, NetworkAccess, ToolId},
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

fn register_project(state: &AppState, name: &str) -> crate::projects::ProjectRecord {
    let directory = tempfile::tempdir().expect("project directory");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(directory.path())
            .status()
            .expect("git")
            .success()
    );
    let project = state
        .projects
        .create(name.to_owned(), directory.path().to_path_buf())
        .expect("project");
    state.keep_temp_dir(directory);
    project
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
async fn applied_preset_copies_model_and_instructions_without_directory_authority() {
    let mut state = test_state();
    let backend = crate::providers::tests::ScriptedBackend::accept();
    state.chat = std::sync::Arc::new(crate::providers::ChatBackend::Scripted(backend.clone()));
    let token = connected(&state);
    let conversation = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    let model = "grok-4.6".to_owned();
    let selection = ModelSelection::new(
        ProviderKind::Xai,
        model.clone(),
        state
            .models_dev
            .effective_effort(ProviderKind::Xai, &model, None),
    )
    .expect("selection");
    let preset = state
        .agents
        .create(AgentDraft {
            name: "Review preset".to_owned(),
            instructions: "Review only the supplied discussion.".to_owned(),
            selection: Some(selection.clone()),
            tools: vec![ToolId::Write],
            network: NetworkAccess::Public,
            directories: vec![crate::agents::DirectoryGrant {
                alias: "project".to_owned(),
                host_path: std::env::current_dir().expect("checkout"),
                access: crate::agents::AccessMode::ReadWrite,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("preset");
    let path = format!("/conversations/{}/preset", conversation.id);

    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("revision={}&preset={}", conversation.revision, preset.id),
        ))
        .await
        .expect("apply preset");
    assert_eq!(response.status(), StatusCode::OK);
    let updated = state.conversations.get(&conversation.id).expect("updated");
    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("revision={}&preset={}", conversation.revision, preset.id),
        ))
        .await
        .expect("stale apply");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        state.conversations.get(&conversation.id).as_ref(),
        Some(&updated)
    );
    let model = updated.model.as_ref().expect("model configuration");
    assert_eq!(model.instructions, "Review only the supplied discussion.");
    assert_eq!(model.selection, selection);
    let applied = model.preset.as_ref().expect("preset identity");
    assert_eq!((applied.id, applied.revision), (preset.id, preset.revision));
    assert_eq!(updated.id, conversation.id);

    let response = app(&state)
        .oneshot(command(
            &format!("/conversations/{}/messages", conversation.id),
            &token,
            &format!("revision={}&message=Hello", updated.revision),
        ))
        .await
        .expect("send");
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state
            .conversations
            .get(&conversation.id)
            .expect("conversation")
            .active_job
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("settlement");
    assert_eq!(
        backend.last_preamble().as_deref(),
        Some("Review only the supplied discussion.")
    );
    assert!(backend.last_tools().is_empty());

    let instructions_only = state
        .agents
        .create(AgentDraft {
            name: "Concise".to_owned(),
            instructions: "Reply briefly.".to_owned(),
            selection: None,
            tools: Vec::new(),
            network: NetworkAccess::None,
            directories: Vec::new(),
            primary_directory: String::new(),
        })
        .expect("instructions-only preset");
    let current = state.conversations.get(&conversation.id).expect("current");
    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!(
                "revision={}&preset={}",
                current.revision, instructions_only.id
            ),
        ))
        .await
        .expect("switch preset");
    assert_eq!(response.status(), StatusCode::OK);
    let current = state.conversations.get(&conversation.id).expect("current");
    let model = current.model.as_ref().expect("model");
    assert_eq!(model.selection, selection);
    assert_eq!(model.instructions, "Reply briefly.");
    assert_eq!(
        model.preset.as_ref().expect("preset").id,
        instructions_only.id
    );

    let response = app(&state)
        .oneshot(command(
            &format!("/conversations/{}/model", conversation.id),
            &token,
            &format!(
                "revision={}&provider=xai&model={}&thinking={}",
                current.revision,
                selection.model,
                selection
                    .thinking
                    .as_ref()
                    .map(|effort| effort.as_str())
                    .unwrap_or("")
            ),
        ))
        .await
        .expect("direct model");
    assert_eq!(response.status(), StatusCode::OK);
    let current = state.conversations.get(&conversation.id).expect("current");
    let model = current.model.expect("direct model");
    assert_eq!(model.selection, selection);
    assert!(model.instructions.is_empty());
    assert!(model.preset.is_none());
}

#[tokio::test]
async fn unavailable_preset_models_do_not_replace_conversation_configuration() {
    let state = test_state();
    let token = connected(&state);
    let conversation = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    let selections = [
        ModelSelection::new(ProviderKind::Deepseek, "deepseek-chat".to_owned(), None).unwrap(),
        ModelSelection::new(ProviderKind::Xai, "missing-model".to_owned(), None).unwrap(),
        ModelSelection::new(
            ProviderKind::Xai,
            "grok-4.6".to_owned(),
            Some(crate::providers::ThinkingEffort::new("invalid".to_owned()).unwrap()),
        )
        .unwrap(),
    ];
    for selection in selections {
        let preset = state
            .agents
            .create(AgentDraft {
                name: "Unavailable".to_owned(),
                instructions: String::new(),
                selection: Some(selection),
                tools: Vec::new(),
                network: NetworkAccess::None,
                directories: Vec::new(),
                primary_directory: String::new(),
            })
            .expect("preset");
        let response = app(&state)
            .oneshot(command(
                &format!("/conversations/{}/preset", conversation.id),
                &token,
                &format!("revision={}&preset={}", conversation.revision, preset.id),
            ))
            .await
            .expect("apply");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            state.conversations.get(&conversation.id).as_ref(),
            Some(&conversation)
        );
    }
}

#[tokio::test]
async fn project_context_references_are_distinct_and_do_not_expose_paths() {
    let mut state = test_state();
    let backend = crate::providers::tests::ScriptedBackend::accept();
    state.chat = std::sync::Arc::new(crate::providers::ChatBackend::Scripted(backend.clone()));
    let token = connected(&state);
    let first = register_project(&state, "First project");
    let second = register_project(&state, "Second project");
    let conversation = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    let path = format!("/conversations/{}/projects", conversation.id);

    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("revision={}&project={}", conversation.revision, first.id),
        ))
        .await
        .expect("attach first");
    assert_eq!(response.status(), StatusCode::OK);
    let first_attachment = state.conversations.get(&conversation.id).expect("attached");
    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!(
                "revision={}&project={}",
                first_attachment.revision, second.id
            ),
        ))
        .await
        .expect("attach second");
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("First project"));
    assert!(body.contains("Second project"));
    assert!(body.contains("Context reference only. File access is not granted."));
    assert!(!body.contains(first.host_path.to_string_lossy().as_ref()));
    assert!(!body.contains(second.host_path.to_string_lossy().as_ref()));
    let attached = state.conversations.get(&conversation.id).expect("attached");
    assert_eq!(attached.projects, vec![first.id, second.id]);
    let model = "grok-4.6".to_owned();
    let attached = state
        .conversations
        .select_model(
            &attached.id,
            attached.revision,
            ModelSelection::new(
                ProviderKind::Xai,
                model.clone(),
                state
                    .models_dev
                    .effective_effort(ProviderKind::Xai, &model, None),
            )
            .expect("model"),
        )
        .expect("select model");
    let response = app(&state)
        .oneshot(command(
            &format!("/conversations/{}/messages", attached.id),
            &token,
            &format!("revision={}&message=Hello", attached.revision),
        ))
        .await
        .expect("send");
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state
            .conversations
            .get(&attached.id)
            .expect("conversation")
            .active_job
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("settlement");
    let preamble = backend.last_preamble().expect("project metadata");
    assert!(preamble.contains("First project"));
    assert!(preamble.contains("Second project"));
    assert!(!preamble.contains(first.host_path.to_string_lossy().as_ref()));
    assert!(!preamble.contains(second.host_path.to_string_lossy().as_ref()));
    assert!(backend.last_tools().is_empty());

    state
        .conversations
        .create("Unrelated conversation".to_owned())
        .expect("unrelated conversation");
    let filter_path = format!("/conversations?project={}", first.id);
    let patch = Request::builder()
        .uri(&filter_path)
        .header(header::COOKIE, cookie(&token))
        .header("graft-request", "patch")
        .header(header::ACCEPT, "text/vnd.hypergraft.patches+html")
        .body(Body::empty())
        .expect("patch request");
    for request in [
        document(&filter_path, &token),
        navigation(&filter_path, &token),
        patch,
    ] {
        let response = app(&state).oneshot(request).await.expect("filter");
        assert_eq!(response.status(), StatusCode::OK);
        let body = text(response).await;
        assert!(body.contains("Discussion"));
        assert!(!body.contains("Unrelated conversation"));
    }

    std::fs::remove_dir_all(&first.host_path).expect("remove project directory");
    let response = app(&state)
        .oneshot(document(&format!("/conversations/{}", attached.id), &token))
        .await
        .expect("unavailable project");
    let body = text(response).await;
    assert!(body.contains("Project unavailable."));
    assert!(body.contains("First project"));
    let current = state.conversations.get(&attached.id).expect("conversation");
    let response = app(&state)
        .oneshot(command(
            &format!("{path}/{}", first.id),
            &token,
            &format!("revision={}", current.revision),
        ))
        .await
        .expect("detach unavailable project");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        state
            .conversations
            .get(&attached.id)
            .expect("conversation")
            .projects,
        vec![second.id]
    );
}

#[tokio::test]
async fn project_context_rejects_unknown_duplicate_stale_and_active_changes() {
    let state = test_state();
    let token = connected(&state);
    let project = register_project(&state, "Project");
    let conversation = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    let path = format!("/conversations/{}/projects", conversation.id);

    for submitted in [
        "not-a-project".to_owned(),
        crate::projects::ProjectId::generate()
            .expect("unknown identifier")
            .as_hex(),
        project.host_path.to_string_lossy().into_owned(),
    ] {
        let response = app(&state)
            .oneshot(command(
                &path,
                &token,
                &format!("revision={}&project={submitted}", conversation.revision),
            ))
            .await
            .expect("invalid project");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            state.conversations.get(&conversation.id),
            Some(conversation.clone())
        );
    }

    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("revision={}&project={}", conversation.revision, project.id),
        ))
        .await
        .expect("attach");
    assert_eq!(response.status(), StatusCode::OK);
    let attached = state.conversations.get(&conversation.id).expect("attached");
    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("revision={}&project={}", attached.revision, project.id),
        ))
        .await
        .expect("duplicate");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        state.conversations.get(&conversation.id),
        Some(attached.clone())
    );

    let detach = format!("{path}/{}", project.id);
    let response = app(&state)
        .oneshot(command(
            &detach,
            &token,
            &format!("revision={}", conversation.revision),
        ))
        .await
        .expect("stale detach");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        state.conversations.get(&conversation.id),
        Some(attached.clone())
    );

    let owner = sessions::generate_session_token().expect("owner");
    state.sessions.insert(owner.id());
    let job = state
        .sessions
        .begin_conversation_job(&owner.id(), conversation.id, 1)
        .expect("job");
    let selection =
        ModelSelection::new(ProviderKind::Xai, "grok-4.6".to_owned(), None).expect("selection");
    state
        .conversations
        .begin_message(
            &conversation.id,
            attached.revision,
            selection,
            job.id(),
            "Question".to_owned(),
        )
        .expect("active message");
    let active = state.conversations.get(&conversation.id).expect("active");
    let response = app(&state)
        .oneshot(command(
            &detach,
            &token,
            &format!("revision={}", active.revision),
        ))
        .await
        .expect("active detach");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        state
            .conversations
            .get(&conversation.id)
            .expect("unchanged"),
        active
    );
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
