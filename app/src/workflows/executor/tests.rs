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
                        workspace: true,
                        journal: false,
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
        crate::agents::AgentId::generate().expect("agent"),
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

#[test]
fn commit_recovery_restores_before_the_reference_and_finalises_after_it() {
    for reference_updated in [false, true] {
        let state = crate::state::for_test(crate::config::RuntimeConfig::development_for_test());
        let project = tempfile::tempdir().expect("project");
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(project.path())
                .status()
                .expect("init")
                .success()
        );
        std::fs::write(project.path().join("file.txt"), b"initial\n").expect("initial");
        assert!(
            std::process::Command::new("git")
                .args(["add", "file.txt"])
                .current_dir(project.path())
                .status()
                .expect("add")
                .success()
        );
        assert!(
            std::process::Command::new("git")
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@localhost",
                    "commit",
                    "-q",
                    "-m",
                    "initial"
                ])
                .current_dir(project.path())
                .status()
                .expect("commit")
                .success()
        );
        let old = git_text(project.path(), &["rev-parse", "HEAD"]);
        let reference = git_text(project.path(), &["symbolic-ref", "HEAD"]);
        let original_index = std::fs::read(project.path().join(".git/index")).expect("index");
        let store = &state.workflow_artefacts;
        let initial =
            crate::workflows::artefacts::CandidateCapture::capture_host(project.path(), store)
                .expect("capture initial");
        let target_bytes = b"target\n";
        std::fs::write(project.path().join("file.txt"), target_bytes).expect("target source");
        let target =
            crate::workflows::artefacts::CandidateCapture::capture_host(project.path(), store)
                .expect("capture target");
        std::fs::write(project.path().join("file.txt"), b"initial\n").expect("restore source");

        let temporary_index = project.path().join(".git/recovery-test.index");
        let index_env = temporary_index.to_string_lossy().into_owned();
        assert!(
            std::process::Command::new("git")
                .args(["read-tree", "--empty"])
                .env("GIT_INDEX_FILE", &index_env)
                .current_dir(project.path())
                .status()
                .expect("empty index")
                .success()
        );
        let blob = git_with_input(
            project.path(),
            &["hash-object", "-w", "--stdin"],
            target_bytes,
        );
        assert!(
            std::process::Command::new("git")
                .args([
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("100644,{blob},file.txt")
                ])
                .env("GIT_INDEX_FILE", &index_env)
                .current_dir(project.path())
                .status()
                .expect("target index")
                .success()
        );
        let tree = git_env_text(project.path(), &["write-tree"], Some((&index_env, "")));
        let commit = git_commit_tree(project.path(), &tree, &old);
        let target_index = std::fs::read(&temporary_index).expect("target index bytes");
        std::fs::remove_file(temporary_index).expect("remove temporary index");

        let agent = state
            .agents
            .create(crate::agents::AgentDraft {
                name: format!("Recovery {reference_updated}"),
                instructions: String::new(),
                tools: crate::agents::ToolId::ALL.to_vec(),
                directories: vec![crate::agents::DirectoryGrant {
                    alias: "project".to_owned(),
                    host_path: project.path().to_path_buf(),
                    access: AccessMode::ReadWrite,
                }],
                primary_directory: "project".to_owned(),
            })
            .expect("agent");
        let definition = crate::workflows::seeds::sequential_team_definition(
            crate::workflows::definition::test_environment_id(),
        );
        let environments = crate::workflows::test_environment_set(&definition);
        let mut run = crate::workflows::WorkflowRun::create(
            crate::workflows::RunId::generate().expect("run"),
            1,
            agent.id,
            crate::workflows::definition::PinnedWorkflowDefinition::pin(None, definition),
            environments,
        );
        let initial_record = candidate_record(&run, &initial, store, true);
        run.record_initial_candidate(initial_record.clone())
            .expect("source");
        let target_record = candidate_record(&run, &target, store, false);
        let target_reference = crate::workflows::artefacts::ArtefactReference {
            id: target_record.id,
            kind: target_record.kind,
            artefact_hash: target_record.artefact_hash,
        };
        let review_record = review_record(&run, target.candidate_hash, store);
        let review_reference = crate::workflows::artefacts::ArtefactReference {
            id: review_record.id,
            kind: review_record.kind,
            artefact_hash: review_record.artefact_hash,
        };
        run.artefacts.extend([target_record, review_record]);
        let crate::workflows::RunSource::Captured { source } = &mut run.source else {
            panic!("source")
        };
        source.accepted = target_reference.clone();
        source.observed = crate::workflows::run::ObservedCandidate::Exact {
            artefact: target_reference.clone(),
        };
        let commit_key =
            crate::workflows::definition::StepKey::parse("commit").expect("commit key");
        run.state = crate::workflows::run::RunState::Ready {
            step: commit_key.clone(),
        };
        let commit_step = run
            .pinned
            .definition
            .step(&commit_key)
            .expect("commit step")
            .clone();
        let capabilities = crate::workflows::capabilities::AttemptCapabilities::derive(
            &commit_step,
            &agent,
            &crate::providers::ProviderConnection::with_key(
                crate::providers::ProviderKind::Xai,
                "key",
                "model",
            ),
        )
        .expect("capabilities");
        let attempt = crate::workflows::AttemptId::generate().expect("attempt");
        let inputs = vec![
            crate::workflows::run::AttemptArtefactInput {
                key: crate::workflows::definition::InputKey::parse("candidate").expect("key"),
                artefact: target_reference.clone(),
            },
            crate::workflows::run::AttemptArtefactInput {
                key: crate::workflows::definition::InputKey::parse("review").expect("key"),
                artefact: review_reference.clone(),
            },
        ];
        let sandbox = crate::workflows::run::AttemptSandboxRecord {
            kind: crate::workflows::run::AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest: run
                .environments
                .steps
                .iter()
                .find(|step| step.step == commit_key)
                .expect("binding")
                .snapshot_digest
                .clone(),
        };
        run.start_attempt(attempt, inputs, capabilities, sandbox, 2)
            .expect("start commit");
        let run_id = run.id;
        state.workflow_runs.create(run).expect("store run");
        let journal = state
            .commit_journals
            .create(run_id, attempt)
            .expect("journal");
        journal
            .write_index_backup("original.index", &original_index)
            .expect("original index");
        journal
            .write_index_backup("target.index", &target_index)
            .expect("target index");
        journal.flush().expect("flush");
        crate::workflows::artefacts::CandidateApply::apply(
            project.path(),
            &initial,
            &target,
            target_reference.artefact_hash,
            store,
        )
        .expect("apply target");
        if reference_updated {
            assert!(
                std::process::Command::new("git")
                    .args(["update-ref", &reference, &commit, &old])
                    .current_dir(project.path())
                    .status()
                    .expect("update ref")
                    .success()
            );
        }
        let transaction = crate::workflows::commit::CommitTransaction {
            state: if reference_updated {
                crate::workflows::commit::CommitTransactionState::ReferenceUpdated {
                    commit: commit.clone(),
                }
            } else {
                crate::workflows::commit::CommitTransactionState::WorktreeApplied
            },
            candidate: target_reference,
            review: review_reference,
            expected_reference: reference,
            old_object: Some(old.clone()),
            target_tree: Some(tree),
            expected_commit: Some(commit.clone()),
            timestamp: "1700000000 +0000".to_owned(),
        };
        state
            .workflow_runs
            .mutate(&run_id, |run| {
                run.record_commit_transaction(attempt, transaction)
            })
            .expect("transaction");

        super::recover_commit_transactions(&state).expect("recover");

        let recovered = state.workflow_runs.get(&run_id).expect("recovered run");
        if reference_updated {
            assert_eq!(recovered.state, crate::workflows::run::RunState::Completed);
            assert_eq!(
                recovered.attempts[0]
                    .commit_result
                    .as_ref()
                    .map(|result| result.commit.as_str()),
                Some(commit.as_str())
            );
            assert_eq!(
                std::fs::read(project.path().join("file.txt")).expect("target file"),
                target_bytes
            );
            assert_eq!(
                std::fs::read(project.path().join(".git/index")).expect("installed index"),
                target_index
            );
        } else {
            assert!(recovered.is_active());
            assert_eq!(
                std::fs::read(project.path().join("file.txt")).expect("restored file"),
                b"initial\n"
            );
            assert_eq!(git_text(project.path(), &["rev-parse", "HEAD"]), old);
            assert_eq!(
                std::fs::read(project.path().join(".git/index")).expect("restored index"),
                original_index
            );
        }
        assert!(state.commit_journals.load(run_id, attempt).is_err());
    }
}

fn git_text(project: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(project)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .to_owned()
}

fn git_env_text(project: &std::path::Path, args: &[&str], index: Option<(&str, &str)>) -> String {
    let mut command = std::process::Command::new("git");
    command.args(args).current_dir(project);
    if let Some((path, _)) = index {
        command.env("GIT_INDEX_FILE", path);
    }
    let output = command.output().expect("git");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .to_owned()
}

fn git_with_input(project: &std::path::Path, args: &[&str], input: &[u8]) -> String {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = std::process::Command::new("git")
        .args(args)
        .current_dir(project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("git");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input)
        .expect("write");
    let output = child.wait_with_output().expect("output");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .to_owned()
}

fn git_commit_tree(project: &std::path::Path, tree: &str, parent: &str) -> String {
    let output = std::process::Command::new("git")
        .args([
            "commit-tree",
            tree,
            "-p",
            parent,
            "-m",
            "Apply Power Plant workflow candidate",
        ])
        .current_dir(project)
        .env("GIT_AUTHOR_NAME", "Power Plant")
        .env("GIT_AUTHOR_EMAIL", "powerplant@localhost")
        .env("GIT_COMMITTER_NAME", "Power Plant")
        .env("GIT_COMMITTER_EMAIL", "powerplant@localhost")
        .env("GIT_AUTHOR_DATE", "1700000000 +0000")
        .env("GIT_COMMITTER_DATE", "1700000000 +0000")
        .output()
        .expect("commit tree");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .to_owned()
}

fn candidate_record(
    run: &crate::workflows::WorkflowRun,
    candidate: &crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
    store: &crate::workflows::WorkflowArtefactRepository,
    initial: bool,
) -> crate::workflows::artefacts::ArtefactRecord {
    let bytes = candidate.manifest_bytes().expect("manifest");
    let object = store.publish(&bytes).expect("publish candidate");
    let id = crate::workflows::ArtefactId::generate().expect("artefact");
    crate::workflows::artefacts::ArtefactRecord {
        id,
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
            run_id: run.id,
            producer: if initial {
                crate::workflows::artefacts::ArtefactProducer::RunSourceCapture
            } else {
                crate::workflows::artefacts::ArtefactProducer::StepAttempt {
                    attempt_id: crate::workflows::AttemptId::generate().expect("producer"),
                    step: crate::workflows::definition::StepKey::parse("implementer")
                        .expect("step"),
                    output: Some(
                        crate::workflows::definition::OutputKey::parse("candidate")
                            .expect("output"),
                    ),
                    disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
                }
            },
            inputs: Vec::new(),
        },
        summary: crate::workflows::artefacts::ArtefactSummary::Candidate {
            candidate: candidate.candidate_hash,
            entries: candidate.entries.len() as u64,
            bytes: 0,
            disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
        },
    }
}

fn review_record(
    run: &crate::workflows::WorkflowRun,
    candidate: crate::workflows::artefacts::CandidateHash,
    store: &crate::workflows::WorkflowArtefactRepository,
) -> crate::workflows::artefacts::ArtefactRecord {
    let (bytes, object, hash) = crate::workflows::artefacts::payload::encode_review(
        candidate,
        crate::workflows::artefacts::ReviewVerdict::Approved,
        "approved",
        None,
    )
    .expect("review");
    store.publish(&bytes).expect("publish review");
    crate::workflows::artefacts::ArtefactRecord {
        id: crate::workflows::ArtefactId::generate().expect("review id"),
        kind: crate::workflows::definition::ArtefactKind::ReviewReport,
        artefact_hash: hash,
        object_hash: object,
        payload_bytes: bytes.len() as u64,
        created_at_ms: 1,
        provenance: crate::workflows::artefacts::ArtefactProvenance {
            run_id: run.id,
            producer: crate::workflows::artefacts::ArtefactProducer::StepAttempt {
                attempt_id: crate::workflows::AttemptId::generate().expect("producer"),
                step: crate::workflows::definition::StepKey::parse("reviewer").expect("step"),
                output: Some(
                    crate::workflows::definition::OutputKey::parse("review").expect("output"),
                ),
                disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
            },
            inputs: Vec::new(),
        },
        summary: crate::workflows::artefacts::ArtefactSummary::Review {
            candidate,
            verdict: crate::workflows::artefacts::ReviewVerdict::Approved,
        },
    }
}
