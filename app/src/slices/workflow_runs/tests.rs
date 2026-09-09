use askama::Template;
use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    config::RuntimeConfig,
    providers::{ProviderConnection, ProviderKind},
    sessions,
    state::AppState,
    workflows::{
        RunId, WorkflowRun, definition::PinnedWorkflowDefinition, seeds::one_agent_definition,
    },
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

fn cookie(token: &str) -> String {
    format!("powerplant_session={token}")
}

fn connected(state: &AppState) -> String {
    let token = sessions::generate_session_token().expect("session token");
    state
        .vault
        .put(ProviderConnection::with_key(
            ProviderKind::Xai,
            "test-key",
            "grok-4.6",
        ))
        .expect("vault");
    state.sessions.insert(token.id());
    token.raw().as_str().to_owned()
}

fn git_init(path: &std::path::Path) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .expect("git")
            .success()
    );
}

fn stored_run(state: &AppState) -> RunId {
    let dir = tempfile::tempdir().expect("dir");
    git_init(dir.path());
    let project = state
        .projects
        .create("Harbour".to_owned(), dir.path().to_path_buf())
        .expect("project");
    state.keep_temp_dir(dir);
    let definition = one_agent_definition(crate::tests::test_environment_id());
    let environments = crate::tests::test_environment_set(&definition);
    let run = WorkflowRun::create(
        RunId::generate().expect("run"),
        1,
        project.id,
        Some(crate::agents::AgentId::generate().expect("agent")),
        crate::workflows::RunKind::Configured,
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    let id = run.id;
    state.workflow_runs.create(run).expect("store");
    id
}

#[tokio::test]
async fn a_runs_document_uses_chat_main() {
    let state = test_state();
    let token = connected(&state);
    stored_run(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/runs")
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("index");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert_eq!(text.matches("id=\"chat-main\"").count(), 1);
    assert!(text.contains("href=\"/runs/"));
    assert!(text.contains("data-graft"));
    assert!(text.contains("Harbour"));
}

#[tokio::test]
async fn a_runs_navigation_patches_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/runs")
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "navigation")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("navigation");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("<!doctype html>"));
    assert!(text.contains("operation=\"children\" target=\"chat-main\""));
}

#[tokio::test]
async fn a_runs_patch_is_rejected() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri("/runs")
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("patch");
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_detail_document_uses_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let id = stored_run(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}", id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>"));
    assert_eq!(text.matches("id=\"run-detail\"").count(), 1);
    assert!(text.contains("Refresh"));
    assert!(text.contains("Harbour"));
    assert!(text.contains("href=\"/projects/"));
}

#[tokio::test]
async fn a_detail_navigation_patches_chat_main() {
    let state = test_state();
    let token = connected(&state);
    let id = stored_run(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}", id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "navigation")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("navigation");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("operation=\"children\" target=\"chat-main\""));
}

#[tokio::test]
async fn a_detail_patch_targets_run_detail() {
    let state = test_state();
    let token = connected(&state);
    let id = stored_run(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}", id.as_hex()))
                .header(header::COOKIE, cookie(&token))
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("patch");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("operation=\"children\" target=\"run-detail\""));
    assert!(!text.contains("id=\"run-detail\""));
}

#[test]
fn review_verdict_skips_candidate_outputs_from_fixing_reviews() {
    let candidate = crate::workflows::artefacts::ArtefactSummary::Candidate {
        candidate: crate::workflows::artefacts::CandidateHash::of(b"candidate"),
        entries: 1,
        bytes: 1,
        disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
    };
    let review = crate::workflows::artefacts::ArtefactSummary::Review {
        candidate: crate::workflows::artefacts::CandidateHash::of(b"candidate"),
        verdict: crate::workflows::artefacts::ReviewVerdict::Approved,
    };

    assert_eq!(
        super::page::review_verdict_label([&candidate, &review].into_iter()),
        "Approved"
    );
}

#[test]
fn run_timeline_renders_status_handoffs_and_the_commit_identifier() {
    let view = super::page::RunDetailView {
        run_id: "run".to_owned(),
        conversation_href: String::new(),
        project_href: String::new(),
        project_name: String::new(),
        name: "Sequential team".to_owned(),
        name_href: String::new(),
        catalogue_note: String::new(),
        version: "version".to_owned(),
        state: "Completed",
        state_note: "All steps completed.",
        review_href: String::new(),
        created: "now".to_owned(),
        current_step: "Commit".to_owned(),
        steps: vec![super::page::StepView {
            name: "Commit".to_owned(),
            action: "System command",
            candidate_access: "",
            environment: "Alpine Git".to_owned(),
            status: "Completed",
            result: "Completed".to_owned(),
            artefacts: vec![super::page::StepArtefactView {
                href: "/runs/run/artefacts/candidate".to_owned(),
                key: "committed-candidate".to_owned(),
                kind: "candidate-revision",
                candidate_hash: String::new(),
                status: "",
                note: "",
                review_href: String::new(),
            }],
            commit: "01234567".to_owned(),
            gate_href: String::new(),
            review_phase: String::new(),
            attempt_limit: String::new(),
            latest_verdict: String::new(),
            selected_route: String::new(),
            role: String::new(),
            model: String::new(),
            host_approval: String::new(),
        }],
        environments: Vec::new(),
        task_selection: None,
        launch_inputs: Vec::new(),
        attempts: Vec::new(),
        artefacts: Vec::new(),
        parent_href: String::new(),
        hierarchy: String::new(),
        context_boundaries: String::new(),
        process_phases: Vec::new(),
        host_approval: String::new(),
        pending_host_command: None,
    };

    let rendered = view.render().expect("render timeline");

    assert!(rendered.contains("Completed"));
    assert!(rendered.contains("Commit 01234567"));
    assert!(rendered.contains("href=\"/runs/run/artefacts/candidate\" data-graft"));
}

fn context_packet(prompt: String) -> crate::workflows::input_context::AttemptContextPacket {
    let mut packet = crate::workflows::input_context::AttemptContextPacket {
        prompt,
        messages: vec![crate::workflows::input_context::ContextMessage::User(
            "Execute the assigned task.".to_owned(),
        )],
        tools: vec![crate::workflows::input_context::ContextTool {
            name: "read".to_owned(),
            description: "Read a granted file.".to_owned(),
            parameters: serde_json::json!({"type": "object"}),
        }],
        source_available: "Candidate files are available through tools.".to_owned(),
        excluded_context: "Conversation and worker transcripts are excluded.".to_owned(),
        project_instructions: crate::workflows::input_context::ProjectInstructionSnapshot {
            candidate: None,
            guest_path: "AGENTS.md".to_owned(),
            state: crate::workflows::input_context::ProjectInstructionState::Present {
                text: "Use the test command.".to_owned(),
                content_hash: crate::workflows::artefacts::ObjectHash::of(b"Use the test command.")
                    .as_str(),
            },
        },
        budget: crate::workflows::input_context::ContextBudget {
            packet_bytes: 42,
            reserved_output_bytes: 128,
            reserved_tool_bytes: 256,
            total_bytes: 426,
            estimated_input_tokens: 11,
            estimated_total_tokens: 107,
            model_context_limit: None,
        },
    };
    packet.budget.packet_bytes = packet.byte_len() as u64;
    packet.budget.reserved_output_bytes =
        crate::workflows::input_context::RESERVED_MODEL_OUTPUT_BYTES as u64;
    packet.budget.reserved_tool_bytes =
        crate::workflows::input_context::RESERVED_TOOL_WORK_BYTES as u64;
    packet.budget.total_bytes = packet.budget.packet_bytes
        + packet.budget.reserved_output_bytes
        + packet.budget.reserved_tool_bytes;
    packet.budget.estimated_input_tokens = packet.budget.packet_bytes.div_ceil(4);
    packet.budget.estimated_total_tokens = packet.budget.total_bytes.div_ceil(4);
    packet
}

#[test]
fn context_inspection_preserves_text_across_bounded_escaped_pages() {
    let packet = context_packet("<script>é&".repeat(8000));
    let mut collected = String::new();
    loop {
        let context = super::page::initial_context_view(
            &packet,
            "/runs/run",
            "/runs/run/attempts/attempt/context",
            0,
            collected.len(),
        )
        .expect("context");
        let rendered = context.render().expect("render");
        assert!(!rendered.contains("<script>"));
        hypergraft::outcome::page_patch("Initial context", "chat-main", &context)
            .expect("bounded navigation envelope");
        collected.push_str(&context.prompt);
        if collected.len() == packet.prompt.len() {
            assert!(context.next_href.contains("part=1"));
            break;
        }
        assert!(
            context
                .next_href
                .ends_with(&format!("offset={}", collected.len()))
        );
    }
    assert_eq!(collected, packet.prompt);
    let messages =
        super::page::initial_context_view(&packet, "/runs/run", "/context", 1, 0).expect("message");
    assert_eq!(messages.prompt, packet.request_messages()[0].text);
    let tools =
        super::page::initial_context_view(&packet, "/runs/run", "/context", 2, 0).expect("tools");
    let tools: serde_json::Value = serde_json::from_str(&tools.prompt).expect("tool JSON");
    assert_eq!(tools[0]["parameters"], packet.request_tools()[0].parameters);
    assert!(
        super::page::initial_context_view(&packet, "/runs/run", "/context", usize::MAX, 0)
            .is_none()
    );
    assert!(
        super::page::initial_context_view(&packet, "/runs/run", "/context", 0, usize::MAX)
            .is_none()
    );
}

#[tokio::test]
async fn context_routes_reject_cross_run_attempts_and_unsupported_patches() {
    let state = test_state();
    let token = connected(&state);
    let run_id = stored_run(&state);
    let other_run = stored_run(&state);
    let attempt_id = crate::workflows::AttemptId::generate().expect("attempt");
    state
        .workflow_runs
        .mutate(&run_id, |run| {
            let sandbox = crate::workflows::run::AttemptSandboxRecord {
                kind: crate::workflows::run::AttemptSandboxKind::IsolatedAttempt,
                snapshot_digest: run.environments.steps[0].snapshot_digest.clone(),
            };
            run.start_attempt(
                attempt_id,
                vec![],
                crate::tests::test_agent_capabilities(),
                sandbox,
                2,
            )?;
            run.record_initial_context(
                attempt_id,
                context_packet("Private attempt direction".to_owned()),
            )
        })
        .expect("attempt");
    for (owner, representation, status) in [
        (run_id, None, 200),
        (run_id, Some("navigation"), 200),
        (run_id, Some("patch"), 400),
        (other_run, None, 303),
    ] {
        let mut request = Request::builder()
            .uri(format!(
                "/runs/{}/attempts/{}/context",
                owner.as_hex(),
                attempt_id.as_hex()
            ))
            .header(header::COOKIE, cookie(&token));
        if let Some(representation) = representation {
            request = request
                .header(hypergraft::GRAFT_REQUEST, representation)
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
        }
        let response = app(&state)
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .expect("context response");
        assert_eq!(response.status().as_u16(), status);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert_eq!(text.contains("Private attempt direction"), status == 200);
    }
}

#[tokio::test]
async fn evidence_routes_reject_cross_run_attempts_and_unsupported_patches() {
    let state = test_state();
    let token = connected(&state);
    let run_id = stored_run(&state);
    let other_run = stored_run(&state);
    let attempt_id = crate::workflows::AttemptId::generate().expect("attempt");
    state
        .workflow_runs
        .mutate(&run_id, |run| {
            let sandbox = crate::workflows::run::AttemptSandboxRecord {
                kind: crate::workflows::run::AttemptSandboxKind::IsolatedAttempt,
                snapshot_digest: run.environments.steps[0].snapshot_digest.clone(),
            };
            run.start_attempt(
                attempt_id,
                vec![],
                crate::tests::test_agent_capabilities(),
                sandbox,
                2,
            )
        })
        .expect("attempt");
    let phase = state.workflow_runs.get(&run_id).unwrap().attempts[0]
        .step
        .as_str()
        .to_owned();
    let evidence = crate::workflows::AttemptEvidenceContext::new(
        state.workflow_evidence.clone(),
        run_id,
        attempt_id,
        phase,
    );
    for _ in 0..crate::workflows::evidence::MAXIMUM_ACTIVITY_EVENTS {
        evidence.response(&"<&'\"".repeat(64), None);
    }
    let mut reply = crate::providers::AssistantReply::from("<&'\"".repeat(16 * 1024));
    reply.thinking = reply.text.clone();
    reply.tools = vec![
        crate::providers::ToolOutput {
            label: reply.text.clone(),
            output: reply.text.clone(),
        };
        8
    ];
    evidence.terminal(
        crate::workflows::evidence::TerminalState::Completed,
        &reply,
        Some(&reply.text),
        None,
    );
    for view in ["activity", "changes", "result"] {
        for (owner, representation, status) in [
            (run_id, None, 200),
            (run_id, Some("navigation"), 200),
            (run_id, Some("patch"), 400),
            (other_run, None, 303),
        ] {
            let mut request = Request::builder()
                .uri(format!(
                    "/runs/{}/attempts/{}/{view}",
                    owner.as_hex(),
                    attempt_id.as_hex()
                ))
                .header(header::COOKIE, cookie(&token));
            if let Some(representation) = representation {
                request = request
                    .header(hypergraft::GRAFT_REQUEST, representation)
                    .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
            }
            let response = app(&state)
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .expect("evidence response");
            assert_eq!(response.status().as_u16(), status, "{view}");
            if status == 303 {
                assert_eq!(
                    response.headers()[header::LOCATION],
                    format!("/runs/{}", other_run.as_hex())
                );
            }
        }
    }
}

#[tokio::test]
async fn an_unknown_run_redirects_to_the_index() {
    let state = test_state();
    let token = connected(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}", "a".repeat(32)))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("missing");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/runs");
}

#[tokio::test]
async fn an_unknown_artefact_redirects_to_the_run() {
    let state = test_state();
    let token = connected(&state);
    let id = stored_run(&state);
    let response = app(&state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/runs/{}/artefacts/{}",
                    id.as_hex(),
                    "a".repeat(32)
                ))
                .header(header::COOKIE, cookie(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("missing artefact");
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        format!("/runs/{}", id.as_hex()).as_str()
    );
}

fn stored_loop(state: &AppState) -> crate::workflows::TaskLoop {
    use crate::workflows::task_loop::tests::loop_record;
    state.task_loops.create(loop_record()).expect("loop")
}

async fn post_loop_command(
    state: &AppState,
    token: &str,
    loop_id: &str,
    action: &str,
    body: String,
) -> axum::http::Response<Body> {
    app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/runs/loops/{loop_id}/{action}"))
                .header(header::COOKIE, cookie(token))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .expect("loop command")
}

#[tokio::test]
async fn stale_loop_commands_leave_the_checkpoint_unchanged() {
    use crate::workflows::task_loop::TaskLoopState;
    let state = test_state();
    let token = connected(&state);
    let parent = stored_loop(&state);
    let (parent, child, _) = state
        .task_loops
        .reserve_next_child(&parent.id, 0)
        .expect("reserve");
    state
        .task_loops
        .mark_dispatched(&parent.id, child)
        .expect("dispatch");
    let parent = state.task_loops.get(&parent.id).expect("loop");
    let stale = post_loop_command(
        &state,
        &token,
        &parent.id.as_hex(),
        "pause",
        "token=paused%3A0".to_owned(),
    )
    .await;
    assert_eq!(stale.status(), axum::http::StatusCode::CONFLICT);
    let body = to_bytes(stale.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("target=\"loop-controls\""), "{text}");
    assert!(!text.contains("id=\"loop-controls\""), "{text}");
    assert!(text.contains("stale"), "{text}");
    let current = state.task_loops.get(&parent.id).expect("unchanged");
    assert!(matches!(current.state, TaskLoopState::Active { .. }));
    let continue_stale = post_loop_command(
        &state,
        &token,
        &parent.id.as_hex(),
        "continue",
        format!("token={}", parent.command_token()),
    )
    .await;
    assert_eq!(continue_stale.status(), axum::http::StatusCode::CONFLICT);
    assert!(matches!(
        state
            .task_loops
            .get(&parent.id)
            .expect("still active")
            .state,
        TaskLoopState::Active { .. }
    ));
}

#[tokio::test]
async fn pause_at_a_child_gate_is_not_approval() {
    use crate::workflows::task_loop::TaskLoopState;
    let (state, token, _, parent) = crate::slices::human_gates::tests::loop_at_gate();
    let child_id = parent.current_child().expect("child");
    let gates = state.workflow_runs.get(&child_id).expect("child").gates;
    let paused = post_loop_command(
        &state,
        &token,
        &parent.id.as_hex(),
        "pause",
        format!("token={}", parent.command_token()),
    )
    .await;
    assert_eq!(paused.status(), axum::http::StatusCode::OK);
    let body = to_bytes(paused.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Pause is not approval"), "{text}");
    let parent = state.task_loops.get(&parent.id).expect("paused request");
    assert!(matches!(
        parent.state,
        TaskLoopState::PauseRequested { child, .. } if child == child_id
    ));
    assert_eq!(
        parent.tasks[0].outcome,
        crate::workflows::TaskOutcome::Dispatched
    );
    let child = state.workflow_runs.get(&child_id).expect("child");
    assert!(!child.is_terminal());
    assert_eq!(child.gates, gates);
    assert!(!child.artefacts.iter().any(|artefact| {
        artefact.kind == crate::workflows::definition::ArtefactKind::HumanDecision
    }));
    let stale = post_loop_command(
        &state,
        &token,
        &parent.id.as_hex(),
        "stop",
        format!(
            "token={}",
            parent
                .command_token()
                .replace("pause-requested", "awaiting")
        ),
    )
    .await;
    assert_eq!(stale.status(), axum::http::StatusCode::CONFLICT);
    assert!(matches!(
        state.task_loops.get(&parent.id).expect("still pause").state,
        TaskLoopState::PauseRequested { .. }
    ));
}

fn paused_loop() -> (
    AppState,
    String,
    sessions::SessionId,
    crate::workflows::TaskLoop,
) {
    let (state, token, session, parent) = crate::slices::human_gates::tests::loop_at_gate();
    let child = parent.current_child().expect("child");
    let continuation = state.gate_continuations.take(&child).expect("continuation");
    let project = state
        .projects
        .get(&parent.project_id.expect("project"))
        .expect("project");
    for args in [
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "-qm",
            "Completed task",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&project.host_path)
                .status()
                .expect("git")
                .success()
        );
    }
    let source = crate::workflows::artefacts::CandidateCapture::capture_host(
        &project.host_path,
        &state.workflow_artefacts,
    )
    .expect("post-commit source");
    let completed = crate::workflows::task_loop::tests::completed_child(
        &parent,
        child,
        crate::workflows::task_loop::tests::source(1),
    );
    state
        .workflow_runs
        .mutate(&child, |run| {
            *run = completed.clone();
            Ok(())
        })
        .expect("completed child");
    state
        .task_loops
        .request_pause(&parent.id, &parent.command_token())
        .expect("pause");
    let (parent, _) = state
        .task_loops
        .complete_child(&parent.id, &completed)
        .expect("checkpoint");
    assert!(
        state
            .gate_continuations
            .park_paused(parent.id, continuation, source)
    );
    (state, token, session, parent)
}

#[tokio::test]
async fn paused_commands_preserve_busy_reservations_and_reject_source_drift() {
    let (state, token, session, parent) = paused_loop();
    let other = state
        .conversations
        .create("Other work".to_owned())
        .expect("conversation");
    let other_job = state
        .sessions
        .begin_conversation_job(&session, other.id, 2)
        .expect("other job");
    let body = format!("token={}&surface=conversation", parent.command_token());
    let busy = post_loop_command(
        &state,
        &token,
        &parent.id.as_hex(),
        "continue",
        body.clone(),
    )
    .await;
    assert_eq!(busy.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(state.task_loops.get(&parent.id).expect("parent"), parent);
    assert!(
        state
            .sessions
            .release_job_reservation(&session, Some(other.id), other_job.id())
    );
    let project = state
        .projects
        .get(&parent.project_id.expect("project"))
        .expect("project");
    std::fs::write(
        project.host_path.join("external-change.txt"),
        "outside edit",
    )
    .expect("drift");
    let drift = post_loop_command(
        &state,
        &token,
        &parent.id.as_hex(),
        "continue",
        body.clone(),
    )
    .await;
    assert_eq!(drift.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(state.task_loops.get(&parent.id).expect("parent"), parent);
    assert!(
        state
            .sessions
            .acquire_job_reservation(&session, Some(other.id), other_job.id())
            .is_ok()
    );
    let stopped = post_loop_command(&state, &token, &parent.id.as_hex(), "stop", body).await;
    assert_eq!(stopped.status(), axum::http::StatusCode::OK);
    let text = String::from_utf8(
        to_bytes(stopped.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("target=\"conversation-detail\""), "{text}");
    assert!(!text.contains("<graft-nav"), "{text}");
    assert!(
        state
            .sessions
            .release_job_reservation(&session, Some(other.id), other_job.id())
    );
    assert!(
        state
            .conversations
            .get(&parent.conversation_id)
            .expect("conversation")
            .active_job
            .is_none()
    );
}

#[test]
fn forgotten_phase_credentials_or_sessions_cannot_leave_a_paused_job_live() {
    for forget_provider in [true, false] {
        let (state, _, session, parent) = paused_loop();
        let mut checkpoint = state
            .gate_continuations
            .take_paused(&parent.id)
            .expect("checkpoint");
        checkpoint.job.phase_providers = vec![ProviderKind::Deepseek];
        state
            .gate_continuations
            .put_back_paused(parent.id, checkpoint);
        if forget_provider {
            crate::workflows::interrupt_provider_continuations(&state, ProviderKind::Deepseek)
                .expect("forget provider");
        } else {
            crate::workflows::interrupt_session_continuations(&state, session)
                .expect("forget session");
        }
        assert!(state.gate_continuations.take_paused(&parent.id).is_none());
        assert!(
            state
                .conversations
                .get(&parent.conversation_id)
                .expect("conversation")
                .active_job
                .is_none()
        );
        let current = state.task_loops.get(&parent.id).expect("parent");
        assert!(current.state.is_terminal());
        assert_eq!(current.tasks[0], parent.tasks[0]);
    }
}

#[tokio::test]
async fn continue_after_a_commit_reserves_only_the_next_task() {
    let (state, token, _, parent) = paused_loop();
    let response = post_loop_command(
        &state,
        &token,
        &parent.id.as_hex(),
        "continue",
        format!("token={}", parent.command_token()),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let current = state.task_loops.get(&parent.id).expect("parent");
    assert_eq!(current.tasks[0], parent.tasks[0]);
    assert!(current.tasks[1].child_id.is_some());
    let job_id = state
        .conversations
        .get(&parent.conversation_id)
        .expect("conversation")
        .active_job
        .expect("reserved");
    state
        .sessions
        .conversation_job(parent.conversation_id, job_id)
        .expect("job")
        .request_cancel();
}

#[tokio::test]
async fn anonymous_run_requests_redirect_to_connect() {
    let state = test_state();
    let detail = format!("/runs/{}", "0".repeat(32));
    let cases = [
        ("GET", "/runs", None, false),
        ("GET", "/runs", Some("navigation"), true),
        ("GET", detail.as_str(), Some("patch"), true),
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

#[test]
fn failed_loop_releases_the_session_but_retains_its_conversation() {
    let (state, _, session, parent) = crate::slices::human_gates::tests::loop_at_gate();
    let child = parent.current_child().expect("child");
    let job = state.gate_continuations.take(&child).expect("job");
    state
        .sessions
        .acquire_job_reservation(&session, job.conversation_id, job.job.id())
        .expect("session reservation");
    assert!(job.job.resume());
    let run = state
        .workflow_runs
        .mutate(&child, |run| run.interrupt(crate::workflows::now_ms()))
        .expect("interrupted child");
    crate::workflows::settle_terminal_job(&state, &job, &run);
    assert_eq!(
        state.task_loops.get(&parent.id).expect("parent").state,
        crate::workflows::task_loop::TaskLoopState::Failed
    );
    assert_eq!(
        state
            .conversations
            .get(&parent.conversation_id)
            .expect("conversation")
            .active_job,
        Some(job.job.id())
    );
    let other = state
        .conversations
        .create("Other work".to_owned())
        .expect("conversation");
    state
        .sessions
        .begin_conversation_job(&session, other.id, 0)
        .expect("session released");
}

#[test]
fn recovered_retry_uses_the_recorded_base_not_a_new_host_capture() {
    let (state, _, _, parent) = crate::slices::human_gates::tests::loop_at_gate();
    let failed = state.task_loops.fail(&parent.id).expect("failed task");
    let before = super::loop_checkpoint_source(&state, &failed).expect("recorded base");
    let project = state
        .projects
        .get(&parent.project_id.expect("project"))
        .expect("project");
    std::fs::write(
        project.host_path.join("external-change.txt"),
        "outside edit",
    )
    .expect("drift");
    let after = super::loop_checkpoint_source(&state, &failed).expect("recorded base");
    assert_eq!(before, after);
    let live = crate::workflows::artefacts::CandidateCapture::capture_host(
        &project.host_path,
        &state.workflow_artefacts,
    )
    .expect("host");
    assert_ne!(after, live);
}
