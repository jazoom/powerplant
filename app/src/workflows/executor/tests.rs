use std::path::PathBuf;

use super::super::definition::{AgentAuthority, GuestDirectoryAccess, SystemCommandId};
use super::{
    StepOutcome, attempt_spec, cleanup_after_start_failure, guest_command, intersect_authority,
    record_unknown_observed, settle_transient_job,
};
use crate::agents::{AccessMode, AgentId, DirectoryPolicy, PolicyGrant};
use crate::sandbox::GUEST_PROJECT;
use crate::sessions::JobStatus;

#[test]
fn repository_status_uses_the_fixed_guest_command() {
    let exec = guest_command(SystemCommandId::RepositoryStatus);
    assert_eq!(exec.program, "git");
    assert_eq!(
        exec.args,
        ["status".to_owned(), "--porcelain=v1".to_owned()]
    );
    assert_eq!(exec.cwd, GUEST_PROJECT);
    assert!(exec.stdin.is_none());
}

#[test]
fn a_secondary_authority_does_not_expose_the_project_mount() {
    let host = DirectoryPolicy::from_grants(
        vec![
            PolicyGrant {
                alias: "project".to_owned(),
                guest_path: GUEST_PROJECT.to_owned(),
                host_path: PathBuf::from("/host/project"),
                access: AccessMode::ReadWrite,
            },
            PolicyGrant {
                alias: "docs".to_owned(),
                guest_path: "/access/docs".to_owned(),
                host_path: PathBuf::from("/host/docs"),
                access: AccessMode::ReadOnly,
            },
        ],
        "project".to_owned(),
    );
    let authority = AgentAuthority::new(
        Vec::new(),
        vec![GuestDirectoryAccess {
            alias: "docs".to_owned(),
            access: AccessMode::ReadOnly,
        }],
    )
    .expect("authority");
    let policy = intersect_authority(&authority, &host).expect("intersection");
    assert_eq!(policy.primary_guest(), "/access/docs");
    assert_eq!(
        policy.resolve(""),
        Ok(("/access/docs".to_owned(), AccessMode::ReadOnly))
    );
    assert!(policy.resolve(GUEST_PROJECT).is_err());
}

#[test]
fn terminal_settlement_releases_the_active_session_turn() {
    let state = crate::state::for_test(crate::config::RuntimeConfig::development_for_test());
    let token = crate::sessions::generate_session_token().expect("token");
    let session_id = token.id();
    let agent_id = AgentId::generate().expect("agent");
    state.sessions.insert(session_id);
    let begun = state
        .sessions
        .begin_turn(
            &session_id,
            agent_id,
            crate::workflows::RunId::generate().expect("run"),
            "Hello".to_owned(),
        )
        .expect("turn");
    settle_transient_job(
        &state,
        &session_id,
        &agent_id,
        &begun.job,
        JobStatus::Failed,
        Some("Operational error"),
    );
    let snapshot = state
        .sessions
        .snapshot(&session_id, &agent_id)
        .expect("session");
    assert!(!snapshot.session_busy);
    assert_eq!(begun.job.snapshot().status, JobStatus::Failed);
}

#[test]
fn attempt_spec_mounts_isolated_source_and_read_only_git() {
    let state = crate::state::for_test(crate::config::RuntimeConfig::development_for_test());
    let run = crate::workflows::RunId::generate().expect("run");
    let attempt = crate::workflows::AttemptId::generate().expect("attempt");
    let workspace = state
        .workflow_workspaces
        .create_attempt(run, attempt)
        .expect("workspace");
    let project = tempfile::tempdir().expect("project");
    std::fs::create_dir(project.path().join(".git")).expect("git");
    let host = DirectoryPolicy::from_grants(
        vec![PolicyGrant {
            alias: "project".to_owned(),
            guest_path: GUEST_PROJECT.to_owned(),
            host_path: project.path().to_path_buf(),
            access: AccessMode::ReadWrite,
        }],
        "project".to_owned(),
    );

    let spec = attempt_spec(
        &crate::workflows::capabilities::test_agent_capabilities(),
        &workspace,
        project.path(),
        &host,
        crate::sandbox::GuestAccess::default(),
    )
    .expect("spec");

    assert_eq!(spec.workdir, GUEST_PROJECT);
    assert_eq!(spec.mounts[0].host, workspace.project);
    assert!(!spec.mounts[0].read_only);
    assert_eq!(spec.mounts[1].guest, "/project/.git");
    assert_eq!(spec.mounts[1].host, project.path().join(".git"));
    assert!(spec.mounts[1].read_only);
    workspace.destroy().expect("destroy");
}

#[tokio::test]
async fn partial_start_cleanup_retains_resources_until_the_guest_is_gone() {
    enum Failure {
        None,
        Stop,
        Remove,
    }

    for failure in [Failure::None, Failure::Stop, Failure::Remove] {
        let state = crate::state::for_test(crate::config::RuntimeConfig::development_for_test());
        let run = crate::workflows::RunId::generate().expect("run");
        let attempt = crate::workflows::AttemptId::generate().expect("attempt");
        let workspace = state
            .workflow_workspaces
            .create_attempt(run, attempt)
            .expect("workspace");
        let workspace_path = workspace.root.clone();
        let sandbox = state.sandboxes.attempt_handle(run, attempt);
        let spec = crate::sandbox::SandboxSpec {
            mounts: vec![crate::sandbox::MountSpec {
                guest: GUEST_PROJECT.to_owned(),
                host: workspace.project.clone(),
                read_only: false,
            }],
            workdir: GUEST_PROJECT.to_owned(),
            access: crate::sandbox::GuestAccess::default(),
        };
        sandbox
            .start_from_snapshot(std::path::Path::new("snapshot"), "sha256:deadbeef", spec)
            .await
            .expect("start");
        match failure {
            Failure::None => {}
            Failure::Stop => sandbox.fail_next_stop(),
            Failure::Remove => sandbox.fail_next_remove(),
        }

        let (outcome, cleanup) = cleanup_after_start_failure(
            &state,
            attempt,
            sandbox,
            workspace,
            StepOutcome::Failed {
                category: crate::workflows::run::FailureCategory::Operational,
                error: None,
            },
        )
        .await;

        match failure {
            Failure::None => {
                assert!(matches!(
                    cleanup,
                    crate::workflows::run::AttemptCleanupRecord::Complete
                ));
                assert!(matches!(
                    outcome,
                    StepOutcome::Failed {
                        category: crate::workflows::run::FailureCategory::Operational,
                        ..
                    }
                ));
                assert!(!workspace_path.exists());
                assert!(!state.sandboxes.guest_named(attempt));
            }
            Failure::Stop | Failure::Remove => {
                assert!(matches!(
                    cleanup,
                    crate::workflows::run::AttemptCleanupRecord::Orphaned {
                        sandbox: true,
                        workspace: true
                    }
                ));
                assert!(matches!(
                    outcome,
                    StepOutcome::Failed {
                        category: crate::workflows::run::FailureCategory::Cleanup,
                        ..
                    }
                ));
                assert!(workspace_path.exists());
                assert!(
                    state
                        .sandboxes
                        .orphans()
                        .iter()
                        .any(|orphan| orphan.name.starts_with("pp-attempt-"))
                );
            }
        }
    }
}

#[test]
fn failed_capture_records_unknown_observed_source() {
    let state = crate::state::for_test(crate::config::RuntimeConfig::development_for_test());
    let definition = crate::workflows::definition::test_named_definition("Work");
    let environments = crate::workflows::test_environment_set(&definition);
    let run_id = crate::workflows::RunId::generate().expect("run");
    let mut run = crate::workflows::run::WorkflowRun::create(
        run_id,
        1,
        crate::workflows::definition::PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    let artefact_id = crate::workflows::ArtefactId::generate().expect("artefact");
    run.record_initial_candidate(crate::workflows::artefacts::ArtefactRecord {
        id: artefact_id,
        kind: crate::workflows::definition::ArtefactKind::CandidateRevision,
        artefact_hash: crate::workflows::artefacts::ArtefactHash::of(b"test", b"payload"),
        object_hash: crate::workflows::artefacts::ObjectHash::of(b"payload"),
        payload_bytes: 7,
        created_at_ms: 1,
        provenance: crate::workflows::artefacts::ArtefactProvenance {
            run_id,
            producer: crate::workflows::artefacts::ArtefactProducer::RunSourceCapture,
            inputs: Vec::new(),
        },
        summary: crate::workflows::artefacts::ArtefactSummary::Candidate {
            candidate: crate::workflows::artefacts::CandidateHash::of(b"tree"),
            entries: 0,
            bytes: 0,
            disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
        },
    })
    .expect("source");
    let attempt = crate::workflows::AttemptId::generate().expect("attempt");
    let sandbox = crate::workflows::run::AttemptSandboxRecord {
        kind: crate::workflows::run::AttemptSandboxKind::IsolatedAttempt,
        snapshot_digest: run.environments.steps[0].snapshot_digest.clone(),
    };
    run.start_attempt(
        attempt,
        Vec::new(),
        crate::workflows::capabilities::test_agent_capabilities(),
        sandbox,
        2,
    )
    .expect("start");
    state.workflow_runs.create(run).expect("store");

    record_unknown_observed(&state, &run_id, attempt).expect("unknown");

    let loaded = state.workflow_runs.get(&run_id).expect("run");
    let crate::workflows::run::RunSource::Captured { source } = loaded.source else {
        panic!("expected captured source");
    };
    assert_eq!(
        source.observed,
        crate::workflows::run::ObservedCandidate::Unknown
    );
}

#[test]
fn unregistered_command_text_cannot_become_a_system_command() {
    assert!(SystemCommandId::parse("rm -rf /").is_none());
    assert!(SystemCommandId::parse("git status --porcelain=v1").is_none());
    assert_eq!(
        SystemCommandId::parse("repository-status"),
        Some(SystemCommandId::RepositoryStatus)
    );
}
