use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use super::forms::{DecisionForm, FormError};
use crate::{
    agents::{AccessMode, AgentDraft, DirectoryGrant, DirectoryPolicy, ToolId},
    config::RuntimeConfig,
    providers::{ProviderConnection, ProviderKind},
    sessions,
    state::AppState,
    workflows::{self, RunKind},
};

fn test_state() -> AppState {
    crate::state::for_test(RuntimeConfig::development_for_test())
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

#[test]
fn decision_forms_reject_duplicate_and_blank_revision_fields() {
    let duplicate = vec![
        ("gate-revision".to_owned(), "1".to_owned()),
        ("gate-revision".to_owned(), "1".to_owned()),
        ("candidate".to_owned(), "sha256:00".to_owned()),
    ];
    assert_eq!(
        DecisionForm::parse(duplicate, false).err(),
        Some(FormError::Invalid)
    );

    let blank_note = vec![
        ("gate-revision".to_owned(), "1".to_owned()),
        ("candidate".to_owned(), "sha256:00".to_owned()),
        ("note".to_owned(), "  ".to_owned()),
    ];
    assert_eq!(
        DecisionForm::parse(blank_note, true).err(),
        Some(FormError::Note)
    );
}

#[tokio::test]
async fn anonymous_gate_requests_redirect_to_connect() {
    let state = test_state();
    let id = "0".repeat(32);
    let detail = format!("/runs/{id}/gates/{id}");
    let approve = format!("/runs/{id}/gates/{id}/approve");
    let cases = [
        ("GET", detail.as_str(), None, false),
        ("GET", detail.as_str(), Some("navigation"), true),
        ("POST", approve.as_str(), None, false),
        ("POST", approve.as_str(), Some("patch"), true),
    ];
    for (method, uri, graft, enhanced) in cases {
        assert_connect_redirect(&state, method, uri, graft, enhanced).await;
    }
}

async fn assert_connect_redirect(
    state: &AppState,
    method: &str,
    uri: &str,
    graft: Option<&str>,
    enhanced: bool,
) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(graft) = graft {
        builder = builder
            .header(hypergraft::GRAFT_REQUEST, graft)
            .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
    }
    let response = app(state)
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("anonymous");
    if enhanced {
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            hypergraft::MEDIA_TYPE
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            text.contains("navigate=\"/connect\""),
            "{method} {uri} {graft:?}: {text}"
        );
    } else {
        assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/connect"
        );
    }
}

struct GateFixture {
    state: AppState,
    token: String,
    session: sessions::SessionId,
    key: sessions::ConversationKey,
    run_id: workflows::RunId,
    gate_id: workflows::GateId,
    project_id: crate::projects::ProjectId,
    agent_id: crate::agents::AgentId,
    candidate: String,
    host: std::path::PathBuf,
}

impl GateFixture {
    fn gate_path(&self) -> String {
        format!(
            "/runs/{}/gates/{}",
            self.run_id.as_hex(),
            self.gate_id.as_hex()
        )
    }

    fn desk_path(&self) -> String {
        crate::projects::desk_path(&self.project_id, &self.agent_id)
    }

    fn decision_body(&self, candidate: &str) -> String {
        format!("gate-revision=1&candidate={candidate}")
    }
}

fn cookie(token: &str) -> String {
    format!("powerplant_session={token}")
}

fn git_worktree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("dir");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .expect("git")
            .success()
    );
    dir
}

fn git_has_head(path: &std::path::Path) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(path)
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

async fn body_text(response: axum::http::Response<Body>) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

fn published_gate_candidate(
    run: &workflows::WorkflowRun,
    captured: &crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
    store: &crate::workflows::WorkflowArtefactRepository,
    producer: crate::workflows::artefacts::ArtefactProducer,
    inputs: Vec<crate::workflows::artefacts::ArtefactReference>,
) -> crate::workflows::artefacts::ArtefactRecord {
    let bytes = captured.manifest_bytes().expect("manifest");
    let object = store.publish(&bytes).expect("publish");
    crate::workflows::artefacts::ArtefactRecord {
        id: crate::workflows::ArtefactId::generate().expect("artefact"),
        kind: crate::workflows::definition::ArtefactKind::CandidateRevision,
        artefact_hash: crate::workflows::artefacts::artefact_hash_for(
            crate::workflows::definition::ArtefactKind::CandidateRevision,
            captured.format_version,
            &bytes,
        ),
        object_hash: object,
        payload_bytes: bytes.len() as u64,
        created_at_ms: 1,
        provenance: crate::workflows::artefacts::ArtefactProvenance {
            run_id: run.id,
            producer,
            inputs,
        },
        summary: crate::workflows::artefacts::ArtefactSummary::Candidate {
            candidate: captured.candidate_hash,
            entries: captured.entries.len() as u64,
            bytes: 0,
            disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
        },
    }
}

fn awaiting_gate(kind: RunKind) -> GateFixture {
    use crate::workflows::definition::{InputKey, OutputKey, StepKey};

    let state = test_state();
    let project_dir = git_worktree();
    std::fs::write(project_dir.path().join("file.txt"), b"candidate\n").expect("source");
    let initial_capture = crate::workflows::artefacts::CandidateCapture::capture_host(
        project_dir.path(),
        &state.workflow_artefacts,
    )
    .expect("initial capture");
    std::fs::write(project_dir.path().join("file.txt"), b"changed\n").expect("change");
    let produced_capture = crate::workflows::artefacts::CandidateCapture::capture_host(
        project_dir.path(),
        &state.workflow_artefacts,
    )
    .expect("changed capture");
    std::fs::write(project_dir.path().join("file.txt"), b"candidate\n").expect("restore source");
    let agent = state
        .agents
        .create(AgentDraft {
            name: "Desk agent".to_owned(),
            instructions: "Do the work.".to_owned(),
            tools: vec![ToolId::List],
            directories: vec![DirectoryGrant {
                alias: "project".to_owned(),
                host_path: project_dir.path().to_path_buf(),
                access: AccessMode::ReadWrite,
            }],
            primary_directory: "project".to_owned(),
        })
        .expect("agent");
    let project = state
        .projects
        .create(
            "Desk project".to_owned(),
            agent.directories[0].host_path.clone(),
        )
        .expect("project");
    let pinned = workflows::pin_quick_task(
        AccessMode::ReadWrite,
        &[ToolId::List],
        "Do the work.",
        crate::workflows::definition::test_environment_id(),
    )
    .expect("quick task");
    let environments = workflows::test_environment_set(&pinned.definition);
    let mut run = workflows::WorkflowRun::create(
        workflows::RunId::generate().expect("run"),
        1,
        project.id,
        agent.id,
        kind,
        pinned,
        environments,
    );
    let initial = published_gate_candidate(
        &run,
        &initial_capture,
        &state.workflow_artefacts,
        crate::workflows::artefacts::ArtefactProducer::RunSourceCapture,
        Vec::new(),
    );
    let initial_ref = crate::workflows::artefacts::ArtefactReference {
        id: initial.id,
        kind: initial.kind,
        artefact_hash: initial.artefact_hash,
    };
    run.record_initial_candidate(initial).expect("initial");
    let work = StepKey::parse("work").expect("work");
    let attempt = workflows::AttemptId::generate().expect("attempt");
    run.start_attempt(
        attempt,
        vec![crate::workflows::run::AttemptArtefactInput {
            key: InputKey::parse("candidate").expect("input"),
            artefact: initial_ref.clone(),
        }],
        crate::workflows::capabilities::test_agent_capabilities(),
        crate::workflows::run::AttemptSandboxRecord {
            kind: crate::workflows::run::AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest: run
                .environments
                .steps
                .iter()
                .find(|binding| binding.step == work)
                .expect("work environment")
                .snapshot_digest
                .clone(),
        },
        2,
    )
    .expect("start");
    let produced = published_gate_candidate(
        &run,
        &produced_capture,
        &state.workflow_artefacts,
        crate::workflows::artefacts::ArtefactProducer::StepAttempt {
            attempt_id: attempt,
            step: work.clone(),
            output: Some(OutputKey::parse("candidate").expect("output")),
            disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
        },
        vec![initial_ref.clone()],
    );
    let produced_ref = crate::workflows::artefacts::ArtefactReference {
        id: produced.id,
        kind: produced.kind,
        artefact_hash: produced.artefact_hash,
    };
    run.record_attempt_outputs(
        attempt,
        vec![produced],
        vec![crate::workflows::run::AttemptArtefactOutput {
            key: OutputKey::parse("candidate").expect("output"),
            artefact: produced_ref.clone(),
        }],
        Some(produced_ref.clone()),
        crate::workflows::run::ObservedCandidate::Exact {
            artefact: produced_ref.clone(),
        },
    )
    .expect("outputs");
    run.record_cleanup(
        attempt,
        crate::workflows::run::AttemptCleanupRecord::Complete,
    )
    .expect("cleanup");
    run.complete_attempt(attempt, 3).expect("complete work");
    let gate_id = workflows::GateId::generate().expect("gate");
    run.open_gate(gate_id, produced_ref, initial_ref, 4)
        .expect("gate");
    let candidate = run
        .artefact(&run.gates[0].candidate.id)
        .and_then(crate::workflows::artefacts::ArtefactRecord::candidate_hash)
        .expect("candidate")
        .as_str()
        .to_owned();
    let run_id = run.id;
    let host = project.host_path.clone();
    state.workflow_runs.create(run).expect("store run");
    state
        .scratch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(project_dir);
    let token = sessions::generate_session_token().expect("session token");
    let session = token.id();
    state.sessions.insert(session);
    state
        .vault
        .put(ProviderConnection::with_key(
            ProviderKind::Xai,
            "test-key",
            "grok-4.6",
        ))
        .expect("vault");
    let key = sessions::ConversationKey {
        project_id: project.id,
        agent_id: agent.id,
    };
    let begun = state
        .sessions
        .begin_turn(&session, key, run_id, "Change the file".to_owned())
        .expect("turn");
    begun.job.set_awaiting_decision();
    begun.job.set_workflow_name("Quick task".to_owned());
    begun.job.set_step_label("Awaiting decision".to_owned());
    let inserted = state.gate_continuations.insert(workflows::WorkflowJob {
        run_id,
        session_id: session,
        project_id: project.id,
        agent_id: agent.id,
        agent_revision: agent.revision,
        grant_alias: "project".to_owned(),
        grant_access: AccessMode::ReadWrite,
        connection: ProviderConnection::with_key(ProviderKind::Xai, "test-key", "grok-4.6"),
        host_policy: DirectoryPolicy::from_record_with_primary(&agent, "project"),
        turns: begun.turns,
        job: begun.job,
        eligible_reply: std::sync::Arc::new(std::sync::Mutex::new(
            "Here is the change.".to_owned(),
        )),
    });
    assert!(inserted);
    GateFixture {
        state,
        token: token.raw().as_str().to_owned(),
        session,
        key,
        run_id,
        gate_id,
        project_id: project.id,
        agent_id: agent.id,
        candidate,
        host,
    }
}

async fn get_gate(fixture: &GateFixture, graft: Option<&str>) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .uri(fixture.gate_path())
        .header(header::COOKIE, cookie(&fixture.token));
    if let Some(graft) = graft {
        builder = builder
            .header(hypergraft::GRAFT_REQUEST, graft)
            .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
    }
    app(&fixture.state)
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("gate")
}

async fn post_decision(
    fixture: &GateFixture,
    action: &str,
    body: String,
    graft: Option<&str>,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("{}/{action}", fixture.gate_path()))
        .header(header::COOKIE, cookie(&fixture.token))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(graft) = graft {
        builder = builder
            .header(hypergraft::GRAFT_REQUEST, graft)
            .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
    }
    app(&fixture.state)
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .expect("decision")
}

#[tokio::test]
async fn a_quick_task_gate_uses_apply_and_discard_labels() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    let response = get_gate(&fixture, None).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("Apply changes"));
    assert!(text.contains("Discard changes"));
    assert!(!text.contains("Request revision"));
    assert!(!text.contains("/request-revision"));
    assert!(text.contains("data-run-kind=\"quick-task\""));
    assert!(text.contains(&format!("data-project=\"{}\"", fixture.project_id.as_hex())));
    assert!(text.contains(&fixture.desk_path()));

    let navigation = get_gate(&fixture, Some("navigation")).await;
    assert_eq!(navigation.status(), axum::http::StatusCode::OK);
    let text = body_text(navigation).await;
    assert!(text.contains("target=\"chat-main\""));
    assert!(text.contains("Apply changes"));
    assert!(!text.contains("Request revision"));
}

#[tokio::test]
async fn a_configured_gate_keeps_revision_controls() {
    let fixture = awaiting_gate(RunKind::Configured);
    let text = body_text(get_gate(&fixture, None).await).await;
    assert!(text.contains("Approve candidate"));
    assert!(text.contains("Request revision"));
    assert!(text.contains("/request-revision"));
    assert!(text.contains("Cancel run"));
    assert!(!text.contains("Apply changes"));
    assert!(!text.contains("Discard changes"));
}

#[tokio::test]
async fn the_project_desk_links_review_changes_for_a_quick_task_gate() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    let response = app(&fixture.state)
        .oneshot(
            Request::builder()
                .uri(fixture.desk_path())
                .header(header::COOKIE, cookie(&fixture.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("desk");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("Review changes"));
    assert!(text.contains(&fixture.gate_path()));
}

#[tokio::test]
async fn a_quick_task_approval_redirects_to_the_project_desk() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    let response = post_decision(
        &fixture,
        "approve",
        fixture.decision_body(&fixture.candidate),
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        fixture.desk_path().as_str()
    );
    let run = fixture
        .state
        .workflow_runs
        .get(&fixture.run_id)
        .expect("run");
    let decision = run.gates[0].decision.as_ref().expect("decision");
    let record = run.artefact(&decision.id).expect("decision record");
    let bytes = fixture
        .state
        .workflow_artefacts
        .get(&record.object_hash)
        .expect("decision object");
    let payload = crate::workflows::artefacts::parse_typed_payload(record.kind, &bytes)
        .expect("decision payload");
    let crate::workflows::artefacts::TypedPayload::HumanDecision(payload) = payload else {
        panic!("human decision");
    };
    assert_eq!(payload.candidate, fixture.candidate);
    assert_eq!(
        payload.decision,
        crate::workflows::gates::HumanDecisionKind::Approved
    );
}

#[tokio::test]
async fn an_enhanced_quick_task_approval_navigates_to_the_project_desk() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    let response = post_decision(
        &fixture,
        "approve",
        fixture.decision_body(&fixture.candidate),
        Some("patch"),
    )
    .await;
    let text = body_text(response).await;
    assert!(text.contains(&format!("navigate=\"{}\"", fixture.desk_path())));
}

#[tokio::test]
async fn a_configured_decision_redirects_to_run_detail() {
    let fixture = awaiting_gate(RunKind::Configured);
    let response = post_decision(
        &fixture,
        "approve",
        fixture.decision_body(&fixture.candidate),
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        format!("/runs/{}", fixture.run_id.as_hex()).as_str()
    );
}

#[tokio::test]
async fn a_quick_task_discard_settles_the_transcript() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    let response = post_decision(
        &fixture,
        "cancel",
        fixture.decision_body(&fixture.candidate),
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        fixture.desk_path().as_str()
    );
    let run = fixture
        .state
        .workflow_runs
        .get(&fixture.run_id)
        .expect("run");
    assert!(matches!(
        run.state,
        crate::workflows::run::RunState::Cancelled
    ));
    let snapshot = fixture
        .state
        .sessions
        .snapshot(&fixture.session, &fixture.key)
        .expect("session");
    assert!(!snapshot.session_busy);
    assert_eq!(
        snapshot.turns.last().map(|turn| turn.text.as_str()),
        Some("Here is the change.")
    );
    assert!(!git_has_head(&fixture.host));
    let begun = fixture.state.sessions.begin_turn(
        &fixture.session,
        fixture.key,
        workflows::RunId::generate().expect("next"),
        "Next task".to_owned(),
    );
    assert!(begun.is_ok());
}

#[tokio::test]
async fn a_quick_task_revision_request_is_rejected() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    let response = post_decision(
        &fixture,
        "request-revision",
        format!(
            "gate-revision=1&candidate={}&note=Please+change+it",
            fixture.candidate
        ),
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    assert!(
        fixture
            .state
            .gate_continuations
            .available(&fixture.run_id, &fixture.session)
    );
    let run = fixture
        .state
        .workflow_runs
        .get(&fixture.run_id)
        .expect("run");
    assert!(matches!(
        run.state,
        crate::workflows::run::RunState::AwaitingHuman { .. }
    ));
}

#[tokio::test]
async fn a_wrong_candidate_decision_is_rejected() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    let response = post_decision(
        &fixture,
        "approve",
        fixture.decision_body("sha256:00"),
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    assert!(
        fixture
            .state
            .gate_continuations
            .available(&fixture.run_id, &fixture.session)
    );
    assert!(matches!(
        fixture
            .state
            .workflow_runs
            .get(&fixture.run_id)
            .expect("run")
            .state,
        crate::workflows::run::RunState::AwaitingHuman { .. }
    ));
    assert!(!git_has_head(&fixture.host));
}

#[tokio::test]
async fn a_stale_agent_revision_interrupts_without_host_mutation() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    let agent = fixture.state.agents.get(&fixture.agent_id).expect("agent");
    fixture
        .state
        .agents
        .update(
            &agent.id,
            agent.revision,
            AgentDraft {
                name: "Renamed agent".to_owned(),
                instructions: agent.instructions.clone(),
                tools: agent.tools.clone(),
                directories: agent.directories.clone(),
                primary_directory: agent.primary_directory.clone(),
            },
        )
        .expect("rename");
    let response = post_decision(
        &fixture,
        "approve",
        fixture.decision_body(&fixture.candidate),
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        fixture.desk_path().as_str()
    );
    let run = fixture
        .state
        .workflow_runs
        .get(&fixture.run_id)
        .expect("run");
    assert!(matches!(
        run.state,
        crate::workflows::run::RunState::Interrupted
    ));
    assert!(
        !fixture
            .state
            .gate_continuations
            .available(&fixture.run_id, &fixture.session)
    );
    assert!(!git_has_head(&fixture.host));
    let snapshot = fixture
        .state
        .sessions
        .snapshot(&fixture.session, &fixture.key)
        .expect("session");
    assert!(!snapshot.session_busy);
}

#[tokio::test]
async fn a_changed_grant_interrupts_without_host_mutation() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    let other = git_worktree();
    let agent = fixture.state.agents.get(&fixture.agent_id).expect("agent");
    fixture
        .state
        .agents
        .update(
            &agent.id,
            agent.revision,
            AgentDraft {
                name: agent.name.clone(),
                instructions: agent.instructions.clone(),
                tools: agent.tools.clone(),
                directories: vec![DirectoryGrant {
                    alias: "project".to_owned(),
                    host_path: other.path().to_path_buf(),
                    access: AccessMode::ReadWrite,
                }],
                primary_directory: "project".to_owned(),
            },
        )
        .expect("change grant");
    fixture
        .state
        .scratch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(other);
    let response = post_decision(
        &fixture,
        "approve",
        fixture.decision_body(&fixture.candidate),
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let run = fixture
        .state
        .workflow_runs
        .get(&fixture.run_id)
        .expect("run");
    assert!(matches!(
        run.state,
        crate::workflows::run::RunState::Interrupted
    ));
    assert!(!git_has_head(&fixture.host));
}

#[tokio::test]
async fn an_unavailable_path_keeps_the_continuation() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    std::fs::remove_dir_all(&fixture.host).expect("remove path");
    let response = post_decision(
        &fixture,
        "approve",
        fixture.decision_body(&fixture.candidate),
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let text = body_text(response).await;
    assert!(text.contains("A granted directory is no longer at the saved path."));
    assert!(
        fixture
            .state
            .gate_continuations
            .available(&fixture.run_id, &fixture.session)
    );
    let run = fixture
        .state
        .workflow_runs
        .get(&fixture.run_id)
        .expect("run");
    assert!(matches!(
        run.state,
        crate::workflows::run::RunState::AwaitingHuman { .. }
    ));
    let snapshot = fixture
        .state
        .sessions
        .snapshot(&fixture.session, &fixture.key)
        .expect("session");
    assert!(snapshot.session_busy);
}

#[tokio::test]
async fn a_gate_object_download_stays_available() {
    let fixture = awaiting_gate(RunKind::QuickTask);
    let response = app(&fixture.state)
        .oneshot(
            Request::builder()
                .uri(format!("{}/objects/target/0", fixture.gate_path()))
                .header(header::COOKIE, cookie(&fixture.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("object");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/octet-stream"
    );
}
