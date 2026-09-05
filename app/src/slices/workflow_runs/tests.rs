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
        crate::agents::AgentId::generate().expect("agent"),
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
            }],
            commit: "01234567".to_owned(),
            gate_href: String::new(),
            review_phase: String::new(),
            attempt_limit: String::new(),
            latest_verdict: String::new(),
            selected_route: String::new(),
            role: String::new(),
        }],
        environments: Vec::new(),
        attempts: Vec::new(),
        artefacts: Vec::new(),
    };

    let rendered = view.render().expect("render timeline");

    assert!(rendered.contains("Completed"));
    assert!(rendered.contains("Commit 01234567"));
    assert!(rendered.contains("href=\"/runs/run/artefacts/candidate\" data-graft"));
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
