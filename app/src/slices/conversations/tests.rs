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

fn session_id(token: &str) -> sessions::SessionId {
    sessions::SessionId::from_validated(&sessions::ValidatedToken::parse(token).expect("token"))
}

fn form_value(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
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

fn candidate_run(
    state: &AppState,
) -> (
    crate::workflows::WorkflowRun,
    crate::workflows::artefacts::ArtefactReference,
) {
    let run_id = crate::workflows::RunId::generate().expect("run");
    let pinned = crate::workflows::pin_quick_task(
        crate::agents::AccessMode::ReadOnly,
        &[ToolId::List, ToolId::Read],
        "Review the selected candidate.",
        crate::tests::test_environment_id(),
    )
    .expect("workflow");
    let mut run = crate::workflows::WorkflowRun::create(
        run_id,
        1,
        crate::projects::ProjectId::generate().expect("project"),
        crate::agents::AgentId::generate().expect("agent"),
        crate::workflows::RunKind::Configured,
        pinned.clone(),
        crate::tests::test_environment_set(&pinned.definition),
    );
    let content = b"Review candidate files.";
    let file = state
        .workflow_artefacts
        .publish(content)
        .expect("file object");
    let candidate = crate::workflows::artefacts::candidate::CandidateRevisionArtefact {
        format_version: crate::workflows::artefacts::CANDIDATE_SCHEMA,
        candidate_hash: crate::workflows::artefacts::candidate::hash_entries(&[
            crate::workflows::artefacts::candidate::CandidateEntry {
                path: "AGENTS.md".to_owned(),
                kind: crate::workflows::artefacts::candidate::CandidateEntryKind::Regular {
                    executable: false,
                    bytes: content.len() as u64,
                    blob: file,
                },
            },
        ]),
        repository: crate::workflows::artefacts::candidate::RepositoryAnchor {
            object_format: crate::workflows::artefacts::candidate::GitObjectFormat::Sha1,
            head: None,
        },
        git_admin: crate::workflows::artefacts::candidate::GitAdministrativeFingerprint::parse(
            &crate::workflows::artefacts::ObjectHash::of(b"git-admin").as_str(),
        )
        .expect("git fingerprint"),
        entries: vec![crate::workflows::artefacts::candidate::CandidateEntry {
            path: "AGENTS.md".to_owned(),
            kind: crate::workflows::artefacts::candidate::CandidateEntryKind::Regular {
                executable: false,
                bytes: content.len() as u64,
                blob: file,
            },
        }],
    };
    let bytes = candidate.manifest_bytes().expect("manifest");
    let object = state
        .workflow_artefacts
        .publish(&bytes)
        .expect("manifest object");
    let record = crate::workflows::artefacts::ArtefactRecord {
        id: crate::workflows::ArtefactId::generate().expect("artefact"),
        kind: crate::workflows::definition::ArtefactKind::CandidateRevision,
        artefact_hash: crate::workflows::artefacts::artefact_hash_for(
            crate::workflows::definition::ArtefactKind::CandidateRevision,
            candidate.format_version,
            &bytes,
        ),
        object_hash: object,
        payload_bytes: bytes.len() as u64,
        created_at_ms: 1,
        provenance: crate::workflows::artefacts::ArtefactProvenance {
            run_id,
            producer: crate::workflows::artefacts::ArtefactProducer::RunSourceCapture,
            inputs: Vec::new(),
        },
        summary: crate::workflows::artefacts::ArtefactSummary::Candidate {
            candidate: candidate.candidate_hash,
            entries: 1,
            bytes: content.len() as u64,
            disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
        },
    };
    let reference = crate::workflows::artefacts::ArtefactReference {
        id: record.id,
        kind: record.kind,
        artefact_hash: record.artefact_hash,
    };
    run.record_initial_candidate(record)
        .expect("initial candidate");
    state.workflow_runs.create(run.clone()).expect("run");
    (run, reference)
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
async fn candidate_review_uses_immutable_selection_without_source_approval() {
    let mut state = test_state();
    let backend = crate::providers::tests::ScriptedBackend::accept();
    state.chat = std::sync::Arc::new(crate::providers::ChatBackend::Scripted(backend.clone()));
    let token = connected(&state);
    let (run, candidate) = candidate_run(&state);
    let path = format!(
        "/conversations/candidate-review?run={}&candidate={}&diff_base={}",
        run.id, candidate.id, candidate.id
    );
    let preview = app(&state)
        .oneshot(document(&path, &token))
        .await
        .expect("preview");
    assert_eq!(preview.status(), StatusCode::OK);

    let unknown = crate::workflows::ArtefactId::generate().expect("unknown artefact");
    let rejected = app(&state)
        .oneshot(command(
            "/conversations/candidate-review",
            &token,
            &format!(
                "run={}&candidate={}&diff_base={}&brief=Review&provider=xai&model=grok-4.6&thinking=medium",
                run.id, unknown, candidate.id
            ),
        ))
        .await
        .expect("candidate substitution");
    assert_eq!(rejected.status(), StatusCode::CONFLICT);
    assert!(
        text(rejected)
            .await
            .contains("selected candidate is unavailable")
    );
    assert!(state.conversations.list().is_empty());

    let started = app(&state)
        .oneshot(command(
            "/conversations/candidate-review",
            &token,
            &format!(
                "run={}&candidate={}&diff_base={}&brief={}&provider=xai&model=grok-4.6&thinking=medium",
                run.id,
                candidate.id,
                candidate.id,
                form_value("Review only this candidate."),
            ),
        ))
        .await
        .expect("start review");
    let started_status = started.status();
    let started_body = text(started).await;
    assert_eq!(started_status, StatusCode::OK, "{started_body}");
    let review = state
        .conversations
        .list()
        .pop()
        .expect("review conversation");
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state
            .conversations
            .get(&review.id)
            .expect("review")
            .active_job
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("review settlement");
    let history = backend.last_history();
    assert!(
        history
            .iter()
            .any(|turn| turn.text.contains("Review candidate files."))
    );
    assert!(
        history
            .iter()
            .any(|turn| turn.text.contains("Selected immutable candidate:"))
    );
    assert_eq!(state.workflow_runs.get(&run.id).expect("source"), run);
    let mut follow_up = state.conversations.get(&review.id).expect("review");
    follow_up.execution_target = Some(crate::projects::ProjectId::generate().expect("target"));
    let result = super::start_message(
        &state,
        session_id(&token),
        follow_up.clone(),
        follow_up.revision,
        follow_up.model.expect("model"),
        "Read the host worktree instead.".to_owned(),
    )
    .await;
    assert!(matches!(result, Err(super::StartMessageError::User(
        hypergraft::PatchStatus::Conflict, message,
    )) if message.contains("immutable evidence")));
}

#[test]
fn candidate_review_rejects_changed_hashes_and_secret_instructions() {
    let state = test_state();
    let (run, candidate) = candidate_run(&state);
    let mut changed = candidate.clone();
    changed.artefact_hash = crate::workflows::artefacts::ArtefactHash::of(b"test", b"different");
    assert!(
        super::job::validate_candidate_review(&state, &run, &changed, &candidate, None,).is_err()
    );
    assert!(
        super::job::validate_candidate_review(
            &state,
            &run,
            &candidate,
            &candidate,
            Some("Review candidate files."),
        )
        .is_err()
    );
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
async fn project_creation_form_attaches_only_after_an_explicit_post() {
    let state = test_state();
    let token = connected(&state);
    let project = register_project(&state, "Context project");
    let form_path = format!("/conversations/new?project={}", project.id.as_hex());

    let form = app(&state)
        .oneshot(document(&form_path, &token))
        .await
        .expect("new conversation form");
    assert_eq!(form.status(), StatusCode::OK);
    let form_body = text(form).await;
    assert!(form_body.contains(&format!(
        "name=\"project\" value=\"{}\"",
        project.id.as_hex()
    )));
    assert!(form_body.contains("This reference does not grant file access"));
    assert!(state.conversations.list().is_empty());

    let created = app(&state)
        .oneshot(command(
            "/conversations",
            &token,
            &format!("title=Project+discussion&project={}", project.id.as_hex()),
        ))
        .await
        .expect("create conversation");
    assert_eq!(created.status(), StatusCode::OK);
    let record = state.conversations.list().pop().expect("conversation");
    assert_eq!(record.projects, vec![project.id]);
    assert!(
        text(created)
            .await
            .contains(&format!("navigate=\"/conversations/{}\"", record.id))
    );
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

#[tokio::test]
async fn writable_access_is_explicit_and_adds_write_without_network_access() {
    let state = test_state();
    let token = connected(&state);
    let project = register_project(&state, "Writable project");
    let conversation = state
        .conversations
        .create("Implementation".to_owned())
        .expect("conversation");
    let attached = state
        .conversations
        .attach_project(&conversation.id, conversation.revision, project.id)
        .expect("attach");
    let path = format!("/conversations/{}/access", conversation.id);

    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!(
                "revision={}&project={}&access=read-write",
                attached.revision, project.id
            ),
        ))
        .await
        .expect("grant");
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("Writable access granted."));
    assert!(body.contains("List, Read, Run and Write"));
    assert!(body.contains("Network: None"));

    let granted = state.conversations.get(&conversation.id).expect("granted");
    let authority =
        crate::conversations::resolve_authority(&granted, &state.projects, &state.agents)
            .expect("authority")
            .expect("writable authority")
            .effective;
    assert_eq!(authority.grant_access, crate::agents::AccessMode::ReadWrite);
    assert_eq!(
        authority.tools,
        vec![ToolId::List, ToolId::Read, ToolId::Run, ToolId::Write]
    );
    assert_eq!(authority.network, NetworkAccess::None);
}

#[tokio::test]
async fn read_only_access_is_explicit_revisioned_and_selects_one_target() {
    let state = test_state();
    let token = connected(&state);
    let project = register_project(&state, "Inspectable project");
    let conversation = state
        .conversations
        .create("Inspection".to_owned())
        .expect("conversation");
    let attached = state
        .conversations
        .attach_project(&conversation.id, conversation.revision, project.id)
        .expect("attach");
    let detail = app(&state)
        .oneshot(document(
            &format!("/conversations/{}", conversation.id),
            &token,
        ))
        .await
        .expect("detail");
    let detail_body = text(detail).await;
    assert!(detail_body.contains("Grant effect: List, Read and Run"));
    assert!(detail_body.contains("Effective network: No network"));
    let path = format!("/conversations/{}/access", conversation.id);

    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("revision={}&project={}", attached.revision, project.id),
        ))
        .await
        .expect("grant");
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("Read-only access granted."));
    let granted = state.conversations.get(&conversation.id).expect("granted");
    assert_eq!(granted.execution_target, Some(project.id));
    assert_eq!(granted.grants.len(), 1);
    assert_eq!(granted.grants[0].project_revision, project.revision);
    assert_eq!(
        granted.grants[0].access,
        crate::agents::AccessMode::ReadOnly
    );

    let stale = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("revision={}&project={}", attached.revision, project.id),
        ))
        .await
        .expect("stale grant");
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(state.conversations.get(&conversation.id), Some(granted));
}

#[tokio::test]
async fn conversation_network_controls_are_bounded_revisioned_and_reserved() {
    let state = test_state();
    let token = connected(&state);
    let conversation = state
        .conversations
        .create("Network settings".to_owned())
        .expect("conversation");
    let path = format!("/conversations/{}/network", conversation.id);

    let invalid = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!(
                "revision={}&network=restricted&network_domains=",
                conversation.revision
            ),
        ))
        .await
        .expect("invalid network");
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        state.conversations.get(&conversation.id),
        Some(conversation.clone())
    );

    let public = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!(
                "revision={}&network=public&network_domains=",
                conversation.revision
            ),
        ))
        .await
        .expect("public network");
    assert_eq!(public.status(), StatusCode::OK);
    let body = text(public).await;
    assert!(body.contains("Public internet"));
    let updated = state.conversations.get(&conversation.id).expect("updated");
    assert_eq!(updated.network, NetworkAccess::Public);

    let stale = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!(
                "revision={}&network=none&network_domains=",
                conversation.revision
            ),
        ))
        .await
        .expect("stale network");
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        state.conversations.get(&conversation.id),
        Some(updated.clone())
    );

    let other = sessions::generate_session_token().expect("other session");
    state.sessions.insert(other.id());
    let _job = state
        .sessions
        .begin_conversation_job(&other.id(), conversation.id, 1)
        .expect("reserved conversation");
    let reserved = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!(
                "revision={}&network=none&network_domains=",
                updated.revision
            ),
        ))
        .await
        .expect("reserved network");
    assert_eq!(reserved.status(), StatusCode::CONFLICT);
    assert_eq!(state.conversations.get(&conversation.id), Some(updated));
}

#[tokio::test]
async fn plan_commands_reject_credentials_without_a_selected_model_or_association() {
    let state = test_state();
    let token = connected(&state);
    let record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    for fields in [
        "title=test-key&markdown=Plan",
        "title=Plan&markdown=test-key",
    ] {
        let response = app(&state)
            .oneshot(command(
                &format!("/conversations/{}/plans/text", record.id),
                &token,
                &format!("revision={}&{fields}", record.revision),
            ))
            .await
            .expect("save");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert!(state.documents.list_for_conversation(record.id).is_empty());
    let plan = state
        .documents
        .create_from_text(record.id, "Plan".to_owned(), "Original".to_owned(), None)
        .expect("plan");
    state
        .documents
        .disassociate(&plan.id, 1, record.id)
        .expect("remove");
    let response = app(&state)
        .oneshot(command(
            &format!("/plans/{}/revisions", plan.id),
            &token,
            "revision=1&title=Plan&markdown=test-key",
        ))
        .await
        .expect("revision");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        state
            .documents
            .get(&plan.id)
            .expect("plan")
            .current_revision(),
        1
    );
}

#[tokio::test]
async fn maximum_plan_text_fits_navigation_after_html_escaping() {
    let state = test_state();
    let token = connected(&state);
    let record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    let plan = state
        .documents
        .create_from_text(record.id, "Plan".to_owned(), "&".repeat(64 * 1024), None)
        .expect("plan");
    let response = app(&state)
        .oneshot(navigation(&format!("/plans/{}", plan.id), &token))
        .await
        .expect("navigation");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        hypergraft::MEDIA_TYPE
    );
}

#[tokio::test]
async fn full_plan_catalogue_and_transcript_fit_conversation_navigation() {
    let state = test_state();
    let token = connected(&state);
    let mut record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    for _ in 0..4 {
        let job = crate::sessions::JobId::generate().expect("job");
        record = state
            .conversations
            .begin_message(
                &record.id,
                record.revision,
                ModelSelection::new(ProviderKind::Xai, "grok-4.6".to_owned(), None).expect("model"),
                job,
                "Question".to_owned(),
            )
            .expect("message");
        state
            .conversations
            .settle_message(
                &record.id,
                job,
                "&".repeat(64 * 1024),
                crate::conversations::MessageStatus::Complete,
            )
            .expect("reply");
        record = state.conversations.get(&record.id).expect("conversation");
    }
    for _ in 0..256 {
        state
            .documents
            .create_from_text(record.id, "&".repeat(120), "Plan".to_owned(), None)
            .expect("plan");
    }
    let response = app(&state)
        .oneshot(navigation(&format!("/conversations/{}", record.id), &token))
        .await
        .expect("navigation");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        hypergraft::MEDIA_TYPE
    );
}

#[tokio::test]
async fn plans_save_open_export_correct_and_remove_without_losing_old_revisions() {
    let state = test_state();
    let token = connected(&state);
    let record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    let model = ModelSelection::new(ProviderKind::Xai, "grok-4.6".to_owned(), None).expect("model");
    let owner = session_id(&token);
    let job = state
        .sessions
        .begin_conversation_job(&owner, record.id, 1)
        .expect("job");
    let record = state
        .conversations
        .begin_message(
            &record.id,
            record.revision,
            model,
            job.id(),
            "Question".to_owned(),
        )
        .expect("message");
    state
        .conversations
        .settle_message(
            &record.id,
            job.id(),
            "# First plan\n".to_owned(),
            crate::conversations::MessageStatus::Complete,
        )
        .expect("reply");
    state
        .sessions
        .finish_conversation_job(&owner, record.id, job.id());
    let record = state.conversations.get(&record.id).expect("settled");

    let save = app(&state)
        .oneshot(command(
            &format!("/conversations/{}/plans", record.id.as_hex()),
            &token,
            &format!(
                "revision={}&message_index=1&title=First+plan",
                record.revision
            ),
        ))
        .await
        .expect("save");
    assert_eq!(save.status(), StatusCode::OK);
    let plans = state.documents.list_for_conversation(record.id);
    let plan = plans.first().expect("saved plan").clone();

    let open = app(&state)
        .oneshot(document(&format!("/plans/{}", plan.id), &token))
        .await
        .expect("open");
    assert_eq!(open.status(), StatusCode::OK);
    assert!(text(open).await.contains("# First plan"));

    let revise = app(&state)
        .oneshot(command(
            &format!("/plans/{}/revisions", plan.id),
            &token,
            &format!(
                "revision=1&title=Corrected+plan&markdown={}",
                form_value("# Corrected plan\n")
            ),
        ))
        .await
        .expect("revise");
    assert_eq!(revise.status(), StatusCode::OK);
    let plan = state.documents.get(&plan.id).expect("corrected plan");
    assert_eq!(plan.current_revision(), 2);

    let old = app(&state)
        .oneshot(document(&format!("/plans/{}?revision=1", plan.id), &token))
        .await
        .expect("old revision");
    assert!(text(old).await.contains("First plan"));
    let export = app(&state)
        .oneshot(document(
            &format!("/plans/{}/export?revision=1", plan.id),
            &token,
        ))
        .await
        .expect("export");
    assert_eq!(export.status(), StatusCode::OK);
    assert_eq!(text(export).await, "# First plan\n");

    let remove = app(&state)
        .oneshot(command(
            &format!(
                "/conversations/{}/plans/{}/remove",
                record.id.as_hex(),
                plan.id.as_hex()
            ),
            &token,
            &format!("revision={}&document_revision=2", record.revision),
        ))
        .await
        .expect("remove association");
    assert_eq!(remove.status(), StatusCode::OK);
    assert!(state.documents.list_for_conversation(record.id).is_empty());
    assert_eq!(
        state
            .documents
            .content(&state.documents.get(&plan.id).expect("retained"), 1)
            .expect("old content"),
        "# First plan\n"
    );
}

#[tokio::test]
async fn plan_review_uses_the_selected_revision_without_inheriting_source_history() {
    let mut state = test_state();
    let backend = crate::providers::tests::ScriptedBackend::accept();
    state.chat = std::sync::Arc::new(crate::providers::ChatBackend::Scripted(backend.clone()));
    let token = connected(&state);
    let source = state
        .conversations
        .create("Planning discussion".to_owned())
        .expect("source");
    let source_job = state
        .sessions
        .begin_conversation_job(&session_id(&token), source.id, 1)
        .expect("source job");
    let source = state
        .conversations
        .begin_message(
            &source.id,
            source.revision,
            ModelSelection::new(ProviderKind::Xai, "grok-4.6".to_owned(), None)
                .expect("source model"),
            source_job.id(),
            "Source discussion only".to_owned(),
        )
        .expect("source message");
    state
        .conversations
        .settle_message(
            &source.id,
            source_job.id(),
            "Source private thoughts must not cross the link".to_owned(),
            crate::conversations::MessageStatus::Complete,
        )
        .expect("source reply");
    state
        .sessions
        .finish_conversation_job(&session_id(&token), source.id, source_job.id());
    let source = state.conversations.get(&source.id).expect("settled source");
    let plan_text = "# Selected plan\n\nKeep this exact revision.\n";
    let plan = state
        .documents
        .create_from_text(source.id, "界".repeat(39), plan_text.to_owned(), None)
        .expect("plan");
    let model = "grok-4.6".to_owned();
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Xai, &model, None);
    let thinking = effort.as_ref().map(|effort| effort.as_str()).unwrap_or("");
    let review_path = format!(
        "/conversations/{}/plans/{}/review?revision=1",
        source.id, plan.id
    );
    let preview = app(&state)
        .oneshot(document(&review_path, &token))
        .await
        .expect("preview");
    assert_eq!(preview.status(), StatusCode::OK);
    let preview_body = text(preview).await;
    assert!(preview_body.contains(&plan.current().content_hash.as_str()));

    let renamed = state
        .conversations
        .rename(
            &source.id,
            source.revision,
            "Source changed later".to_owned(),
        )
        .expect("rename source after preview");
    let body = format!(
        "source_revision={}&document_revision=1&brief={}&provider=xai&model={}&thinking={}&preset=",
        source.revision,
        form_value("Check only the selected plan."),
        form_value(&model),
        form_value(thinking),
    );
    let response = app(&state)
        .oneshot(command(
            &format!("/conversations/{}/plans/{}/review", source.id, plan.id),
            &token,
            &body,
        ))
        .await
        .expect("stale review");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(state.conversations.list().len(), 1);
    state
        .documents
        .revise(
            &plan.id,
            1,
            plan.title.clone(),
            "# Replacement plan\n\nNot selected.\n".to_owned(),
            None,
        )
        .expect("correct plan after preview");
    let response = app(&state)
        .oneshot(command(
            &format!("/conversations/{}/plans/{}/review", source.id, plan.id),
            &token,
            &body.replace(
                &format!("source_revision={}", source.revision),
                &format!("source_revision={}", renamed.revision),
            ),
        ))
        .await
        .expect("create review");
    assert_eq!(response.status(), StatusCode::OK);
    let review_id = state
        .conversations
        .list()
        .into_iter()
        .find(|record| record.id != source.id)
        .expect("review conversation")
        .id;
    assert!(
        text(response)
            .await
            .contains(&format!("navigate=\"/conversations/{review_id}\""))
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state
            .conversations
            .get(&review_id)
            .expect("review")
            .active_job
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("review settlement");

    let review = state.conversations.get(&review_id).expect("review record");
    assert_eq!(
        review
            .source_review
            .as_ref()
            .expect("source link")
            .conversation_id,
        renamed.id
    );
    assert!(review.review_context.is_some());
    assert_eq!(review.messages[0].text, "Check only the selected plan.");
    assert_eq!(review.messages[1].text, "Hello from Power Plant.");
    let source_record = state.conversations.get(&source.id).expect("source record");
    assert_eq!(source_record.plan_reviews.len(), 1);
    assert_eq!(source_record.plan_reviews[0].conversation_id, review_id);
    let history = backend.last_history();
    assert!(history.iter().any(|turn| turn.text.contains(plan_text)));
    assert!(
        history
            .iter()
            .all(|turn| !turn.text.contains("Source private thoughts"))
    );
    let review_page = app(&state)
        .oneshot(document(&format!("/conversations/{review_id}"), &token))
        .await
        .expect("review page");
    let review_body = text(review_page).await;
    assert!(review_body.contains(&format!("href=\"/conversations/{}\"", source.id)));
    let response = app(&state)
        .oneshot(command(
            &format!("/conversations/{review_id}/messages"),
            &token,
            &format!(
                "revision={}&message=Explain+the+first+step",
                review.revision
            ),
        ))
        .await
        .expect("follow-up");
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state
            .conversations
            .get(&review_id)
            .expect("review")
            .active_job
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("follow-up settlement");
    let history = backend.last_history();
    assert!(history.iter().any(|turn| turn.text.contains(plan_text)));
    assert!(
        history
            .iter()
            .any(|turn| turn.text == "Explain the first step")
    );
    assert!(
        history
            .iter()
            .all(|turn| !turn.text.contains("Replacement plan")
                && !turn.text.contains("Source private thoughts"))
    );
}

#[tokio::test]
async fn plan_review_copies_only_confirmed_projects_as_read_only() {
    let state = test_state();
    let token = connected(&state);
    let source = state
        .conversations
        .create("Source".to_owned())
        .expect("source");
    let first = register_project(&state, "Target");
    let second = register_project(&state, "Context");
    let omitted = register_project(&state, "Not confirmed");
    let mut source = source;
    for project in [&first, &second, &omitted] {
        source = state
            .conversations
            .attach_project(&source.id, source.revision, project.id)
            .expect("attach");
        source = state
            .conversations
            .grant_access(
                &source.id,
                source.revision,
                project.id,
                project.revision,
                if project.id == first.id {
                    crate::agents::AccessMode::ReadWrite
                } else {
                    crate::agents::AccessMode::ReadOnly
                },
            )
            .expect("grant");
    }
    source = state
        .conversations
        .set_network(&source.id, source.revision, NetworkAccess::Public)
        .expect("network");
    let plan = state
        .documents
        .create_from_text(
            source.id,
            "Plan".to_owned(),
            "Selected plan".to_owned(),
            None,
        )
        .expect("plan");
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Xai, "grok-4.6", None);
    let body = format!(
        "source_revision={}&document_revision=1&brief=Review&provider=xai&model=grok-4.6&thinking={}&read_only_project={}&read_only_project={}",
        source.revision,
        form_value(effort.as_ref().map(|value| value.as_str()).unwrap_or("")),
        first.id,
        second.id,
    );
    let response = app(&state)
        .oneshot(command(
            &format!("/conversations/{}/plans/{}/review", source.id, plan.id),
            &token,
            &body,
        ))
        .await
        .expect("review");
    let review = state
        .conversations
        .list()
        .into_iter()
        .find(|record| record.id != source.id)
        .expect("linked review");
    assert_eq!(review.projects, vec![first.id, second.id]);
    assert_eq!(review.network, NetworkAccess::None);
    assert!(
        review
            .grants
            .iter()
            .all(|grant| grant.access == crate::agents::AccessMode::ReadOnly)
    );
    // Missing execution prerequisites must still produce a patch for the preview's live root.
    let response = text(response).await;
    assert!(response.contains("target=\"chat-main\""));
    assert!(response.contains(&format!("location=\"/conversations/{}\"", review.id)));
}

#[tokio::test]
async fn task_preparation_uses_the_selected_plan_without_guest_tools() {
    let mut state = test_state();
    let backend = crate::providers::tests::ScriptedBackend::accept();
    state.chat = std::sync::Arc::new(crate::providers::ChatBackend::Scripted(backend.clone()));
    let token = connected(&state);
    let record = state
        .conversations
        .create("Tasks".to_owned())
        .expect("conversation");
    let selection = ModelSelection::new(
        ProviderKind::Xai,
        "grok-4.6".to_owned(),
        state
            .models_dev
            .effective_effort(ProviderKind::Xai, "grok-4.6", None),
    )
    .expect("model");
    let record = state
        .conversations
        .select_model(&record.id, record.revision, selection)
        .expect("selection");
    let project = register_project(&state, "Writable project");
    let record = state
        .conversations
        .attach_project(&record.id, record.revision, project.id)
        .expect("attach");
    let record = state
        .conversations
        .grant_access(
            &record.id,
            record.revision,
            project.id,
            project.revision,
            crate::agents::AccessMode::ReadWrite,
        )
        .expect("grant");
    let plan = state
        .documents
        .create_from_text(
            record.id,
            "Selected plan".to_owned(),
            "# Exact plan\nPreserve this requirement.".to_owned(),
            None,
        )
        .expect("plan");
    let path = format!("/conversations/{}/plans/{}/tasks", record.id, plan.id);
    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("revision={}&document_revision=2", record.revision),
        ))
        .await
        .expect("stale preparation");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(
        state
            .conversations
            .get(&record.id)
            .expect("conversation")
            .messages
            .is_empty()
    );
    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!("revision={}&document_revision=1", record.revision),
        ))
        .await
        .expect("prepare");
    assert_eq!(response.status(), StatusCode::OK);
    for _ in 0..100 {
        if state
            .conversations
            .get(&record.id)
            .expect("conversation")
            .active_job
            .is_none()
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    let history = backend.last_history();
    assert!(history.iter().any(|turn| {
        turn.text
            .contains("# Exact plan\nPreserve this requirement.")
            && turn.text.contains(&plan.current().content_hash.as_str())
    }));
    assert!(backend.last_tools().is_empty());
    assert_eq!(state.documents.list_for_conversation(record.id).len(), 1);
}

#[tokio::test]
async fn task_import_rejects_ungranted_project_and_releases_its_reservation() {
    let state = test_state();
    let token = connected(&state);
    let record = state
        .conversations
        .create("Tasks".to_owned())
        .expect("conversation");
    let project = register_project(&state, "Not authorised");
    let response = app(&state)
        .oneshot(command(
            &format!("/conversations/{}/tasks/import", record.id),
            &token,
            &format!(
                "revision={}&project_id={}&path=tasks.md&title=Tasks",
                record.revision, project.id
            ),
        ))
        .await
        .expect("import");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(response).await.contains("Grant read access"));
    assert!(state.documents.list_for_conversation(record.id).is_empty());
    assert!(!state.sessions.conversation_reserved(record.id));
}

#[tokio::test]
async fn plan_review_rejects_an_oversized_task_brief_without_creating_a_link() {
    let mut state = test_state();
    state.chat = std::sync::Arc::new(crate::providers::ChatBackend::Scripted(
        crate::providers::tests::ScriptedBackend::accept(),
    ));
    let token = connected(&state);
    let source = state
        .conversations
        .create("Planning discussion".to_owned())
        .expect("source");
    let plan = state
        .documents
        .create_from_text(source.id, "Plan".to_owned(), "Plan text".to_owned(), None)
        .expect("plan");
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Xai, "grok-4.6", None);
    let brief = "x".repeat(32 * 1024 + 1);
    let body = format!(
        "source_revision={}&document_revision=1&brief={}&provider=xai&model=grok-4.6&thinking={}&preset=",
        source.revision,
        form_value(&brief),
        form_value(effort.as_ref().map(|value| value.as_str()).unwrap_or("")),
    );
    let response = app(&state)
        .oneshot(command(
            &format!("/conversations/{}/plans/{}/review", source.id, plan.id),
            &token,
            &body,
        ))
        .await
        .expect("oversized review");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.conversations.list().len(), 1);
}
