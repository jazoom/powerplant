use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

use crate::{
    agents::AgentStore,
    config::{RuntimeConfig, StartupConfig},
    environments::{
        EnvironmentCatalogue, EnvironmentPreparationScheduler, EnvironmentSnapshotRepository,
        PreparationState, SnapshotAvailability,
    },
    preferences::Preferences,
    projects::ProjectStore,
    providers::{ChatBackend, ProviderKind},
    sessions::{self, JobStatus},
    state::AppState,
    vault::ProviderVault,
    workflows::{
        CommitJournals, WorkflowArtefactRepository, WorkflowCatalogue, WorkflowRunStore,
        workspace::WorkflowWorkspaces,
    },
};

const EXAMPLE: &str = "Explain how this project is structured.";
const USEFUL_REPLY: &str = "Hello from Power Plant.";

fn activation_state() -> AppState {
    let scratch = tempfile::tempdir().expect("data");
    let (config, local_data) = crate::local_data::prepare(StartupConfig {
        bind_address: "localhost:4000".to_owned(),
        runtime: RuntimeConfig::development(),
        static_dir: PathBuf::from("/tmp/powerplant-static"),
        data_dir: scratch.path().join("data"),
        protected_user_roots: Vec::new(),
    })
    .expect("owned root");
    let root = local_data.root();
    let environments = Arc::new(
        EnvironmentCatalogue::open(
            root.join("environments.json"),
            root.join("environment-preparation-logs"),
        )
        .expect("environments"),
    );
    let snapshots = Arc::new(
        EnvironmentSnapshotRepository::open(root.join("environment-snapshots")).expect("snapshots"),
    );
    let alpine = crate::workflows::alpine_git_id(&environments).expect("alpine-git");
    let workflows = WorkflowCatalogue::open_with_seeds(
        root.join("workflows.json"),
        &crate::workflows::seeds::production_seeds(alpine),
    )
    .expect("workflows");
    let mut state = crate::tests::test_state(config.runtime);
    state.chat = Arc::new(ChatBackend::Scripted(
        crate::tests::ScriptedBackend::accept(),
    ));
    state.vault = Arc::new(ProviderVault::open(root.join("providers.json")).expect("providers"));
    state.preferences = Arc::new(Preferences::open(root.join("preferences.json")));
    state.agents = Arc::new(
        AgentStore::open(root.join("agents"), &root.join("project.json")).expect("agents"),
    );
    state.projects = Arc::new(ProjectStore::open(root.join("projects.json")).expect("projects"));
    state.workflows = Arc::new(workflows);
    state.workflow_runs =
        Arc::new(WorkflowRunStore::open(root.join("workflow-runs")).expect("workflow runs"));
    state.workflow_artefacts = Arc::new(
        WorkflowArtefactRepository::open(root.join("workflow-artefacts")).expect("artefacts"),
    );
    state.workflow_workspaces =
        Arc::new(WorkflowWorkspaces::open(root.join("workflow-workspaces")).expect("workspaces"));
    state.commit_journals =
        Arc::new(CommitJournals::open(root.join("workflow-commit-journals")).expect("journals"));
    state.local_data = local_data;
    state.environments = environments.clone();
    state.environment_snapshots = snapshots.clone();
    state.environment_preparations = EnvironmentPreparationScheduler::idle(environments, snapshots);
    state.keep_temp_dir(scratch);
    state
}

fn app(state: &AppState) -> axum::Router {
    crate::slices::router()
        .layer(from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(from_fn_with_state(
            state.clone(),
            crate::security::enforce_origin,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone())
}

fn git_worktree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("worktree");
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

fn cookie(token: &str) -> String {
    format!("powerplant_session={token}")
}

fn session_cookie(headers: &HeaderMap) -> String {
    let header = headers
        .get(header::SET_COOKIE)
        .expect("session cookie")
        .to_str()
        .expect("cookie utf8");
    let start =
        header.find("powerplant_session=").expect("session name") + "powerplant_session=".len();
    let rest = &header[start..];
    rest[..rest.find(';').unwrap_or(rest.len())].to_owned()
}

fn navigate_target(text: &str) -> String {
    let marker = "navigate=\"";
    let start = text.find(marker).expect("navigate") + marker.len();
    let end = text[start..].find('"').expect("navigate end") + start;
    text[start..end].to_owned()
}

fn location(headers: &HeaderMap) -> String {
    headers
        .get(header::LOCATION)
        .expect("location")
        .to_str()
        .expect("location utf8")
        .to_owned()
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

fn opening_tag_for<'a>(html: &'a str, marker: &str) -> &'a str {
    let marker_start = html.find(marker).expect("element marker");
    let tag_start = html[..marker_start].rfind('<').expect("opening tag");
    let tag_end = html[marker_start..].find('>').expect("opening tag end") + marker_start;
    &html[tag_start..=tag_end]
}

fn job_id_from(html: &str) -> String {
    let marker = "name=\"job\" value=\"";
    let start = html.find(marker).expect("job field") + marker.len();
    let end = html[start..].find('"').expect("job field end") + start;
    html[start..end].to_owned()
}

fn stream_frames(body: &[u8]) -> Vec<String> {
    let mut frames = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let newline = rest
            .iter()
            .position(|&b| b == b'\n')
            .expect("length prefix");
        let len: usize = std::str::from_utf8(&rest[..newline])
            .expect("length utf8")
            .parse()
            .expect("length");
        let start = newline + 1;
        let end = start + len;
        frames.push(String::from_utf8(rest[start..end].to_vec()).expect("frame utf8"));
        rest = &rest[end..];
    }
    frames
}

fn conversation_key(state: &AppState) -> sessions::ConversationKey {
    sessions::ConversationKey {
        project_id: state.projects.list()[0].id,
        agent_id: state.agents.list()[0].id,
    }
}

fn session_id(token: &str) -> sessions::SessionId {
    sessions::SessionId::from_validated(&sessions::ValidatedToken::parse(token).expect("token"))
}

fn session_snapshot(state: &AppState, token: &str) -> sessions::SessionSnapshot {
    state
        .sessions
        .snapshot(&session_id(token), &conversation_key(state))
        .expect("session")
}

async fn wait_until_job_idle(state: &AppState, token: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let job = session_snapshot(state, token).job.expect("submitted job");
            if job.status != JobStatus::Running {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("job finished before timeout");
}

async fn send(state: &AppState, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let response = app(state).oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

fn document(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, cookie(token));
    }
    builder.body(Body::empty()).unwrap()
}

fn patch(uri: &str, token: Option<&str>, body: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::ORIGIN, "http://localhost:4000")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, cookie(token));
    }
    builder.body(Body::from(body.to_owned())).unwrap()
}

fn observe_patch(desk: &str, token: &str, job: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("{desk}?job={job}&cursor=0"))
        .header(header::COOKIE, cookie(token))
        .header(hypergraft::GRAFT_REQUEST, "patch")
        .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::empty())
        .unwrap()
}

fn ready_alpine_git(state: &AppState) {
    let preparation = state
        .environments
        .claim_oldest_queued()
        .expect("claim")
        .expect("queued");
    assert_eq!(preparation.state, PreparationState::Preparing);
    let snapshot = crate::tests::sample_snapshot(preparation.id);
    state.environment_snapshots.mark(
        snapshot.artifact_key.clone(),
        SnapshotAvailability::Available,
    );
    state
        .environments
        .finish_ready(&preparation.id, snapshot, preparation.log)
        .expect("ready");
}

#[tokio::test]
async fn first_task_activation_reaches_a_useful_quick_task_without_onboarding() {
    let state = activation_state();
    assert!(!state.vault.has_providers());
    assert!(state.projects.list().is_empty());
    assert!(state.agents.list().is_empty());
    assert!(!state.workflows.list().is_empty());
    let alpine = crate::workflows::alpine_git_id(&state.environments).expect("alpine-git");
    assert!(
        state
            .environments
            .get(&alpine)
            .expect("environment")
            .ready_preparation
            .is_none()
    );
    let (status, headers, _) = send(&state, document("/", None)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location(&headers), "/connect");

    let (status, _, text) = send(&state, document("/connect", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("action=\"/connect\""));
    assert!(text.contains("Connect a model"));

    let (status, headers, text) = send(
        &state,
        patch("/connect", None, "provider=xai&api_key=sk-test-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(navigate_target(&text), "/");
    assert!(!text.contains("sk-test-key"));
    assert!(state.vault.contains(ProviderKind::Xai));
    let token = session_cookie(&headers);

    let (status, headers, _) = send(&state, document("/", Some(&token))).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location(&headers), "/projects/new");

    let (status, _, text) = send(&state, document("/projects/new", Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("Choose project folder"));
    assert!(text.contains("href=\"/projects/new?entry=manual\""));

    let (status, _, text) =
        send(&state, document("/projects/new?entry=manual", Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("Git project path"));
    assert!(text.contains("action=\"/projects\""));

    let worktree = git_worktree();
    let path = worktree.path().canonicalize().expect("canonical");
    state.keep_temp_dir(worktree);
    let create_body = format!(
        "name=Desk&path={}&entry=manual",
        form_value(&path.to_string_lossy())
    );
    let (status, _, text) = send(&state, patch("/projects", Some(&token), &create_body)).await;
    assert_eq!(status, StatusCode::OK);
    let project = &state.projects.list()[0];
    assert_eq!(project.name, "Desk");
    assert_eq!(project.host_path, path);
    assert_eq!(
        navigate_target(&text),
        format!("/projects/{}", project.id.as_hex())
    );

    let project_path = format!("/projects/{}", project.id.as_hex());
    let (status, _, text) = send(&state, document(&project_path, Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("Create starter agent"));
    assert!(text.contains(&format!(
        "action=\"/projects/{}/agents/starter\"",
        project.id.as_hex()
    )));

    let (status, _, text) = send(
        &state,
        patch(
            &format!("/projects/{}/agents/starter", project.id.as_hex()),
            Some(&token),
            "",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.agents.list().len(), 1);
    let desk = crate::projects::desk_path(&project.id, &state.agents.list()[0].id);
    assert_eq!(navigate_target(&text), desk);

    let (status, _, text) = send(&state, document(&desk, Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("Sandbox preparation is in progress"));
    assert!(opening_tag_for(&text, "value=\"quick\"").contains(" disabled"));
    assert!(
        opening_tag_for(&text, &format!("data-task-example=\"{EXAMPLE}\""))
            .contains("type=\"button\"")
    );
    assert!(text.contains("data-island=\"task-examples\""));
    assert!(session_snapshot(&state, &token).job.is_none());

    ready_alpine_git(&state);

    let (status, _, text) = send(&state, document(&desk, Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("Sandbox is ready"));
    assert!(!opening_tag_for(&text, "value=\"quick\"").contains(" disabled"));
    assert!(text.contains("data-island=\"task-examples\""));
    assert!(session_snapshot(&state, &token).job.is_none());

    let send_body = format!("message={}&mode=quick", form_value(EXAMPLE));
    let (status, _, text) = send(&state, patch(&desk, Some(&token), &send_body)).await;
    assert_eq!(status, StatusCode::OK);
    let job = job_id_from(&text);
    wait_until_job_idle(&state, &token).await;
    assert_eq!(
        session_snapshot(&state, &token)
            .job
            .as_ref()
            .map(|job| job.status),
        Some(JobStatus::Completed)
    );

    let (status, _, body) = send(&state, observe_patch(&desk, &token, &job)).await;
    assert_eq!(status, StatusCode::OK);
    let frames = stream_frames(body.as_bytes());
    let final_frame = frames.last().expect("final frame");
    assert!(final_frame.contains("phase=\"final\""));
    assert!(frames.iter().any(|frame| frame.contains(USEFUL_REPLY)));
    assert!(final_frame.contains("Task finished."));
    assert!(opening_tag_for(final_frame, "Task finished.").contains("role=\"status\""));

    let (status, _, text) = send(&state, document(&desk, Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains(USEFUL_REPLY));
    assert!(text.contains("Task finished."));
    assert!(!text.contains("data-island=\"task-examples\""));
    let run = state
        .workflow_runs
        .get(&state.workflow_runs.summaries()[0].id)
        .expect("run");
    assert_eq!(run.kind, crate::workflows::RunKind::QuickTask);
    assert_eq!(run.pinned.workflow_id, None);
}
