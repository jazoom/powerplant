use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::agents::{AccessMode, DirectoryPolicy, LeaseGuard, PolicyGrant};
use crate::providers::{ChatTurn, ProviderConnection};
use crate::sandbox::{CommandEvent, GUEST_PROJECT, GuestExec, GuestSandbox};
use crate::sessions::{Job, JobStatus, SessionId};
use crate::slices::{AgentOutcome, AgentRunSpec};
use crate::state::AppState;

use super::definition::{AgentAuthority, AgentStep, StepAction, StepDefinition, SystemCommandId};
use super::execution::ExecutionGuard;
use super::id::{AttemptId, RunId};
use super::run::{FailureCategory, now_ms};
use super::store::StoreError;

pub(crate) const OPERATIONAL_STORE_ERROR: &str =
    "Power Plant could not store the workflow run. Try again.";

const COMMAND_DEADLINE: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(10)
};
const COMMAND_OUTPUT_LIMIT: usize = 64 * 1024;

pub(crate) struct WorkflowJob {
    pub(crate) run_id: RunId,
    pub(crate) session_id: SessionId,
    pub(crate) agent_id: crate::agents::AgentId,
    pub(crate) connection: ProviderConnection,
    pub(crate) host_policy: DirectoryPolicy,
    pub(crate) turns: Vec<ChatTurn>,
    pub(crate) job: Arc<Job>,
}

pub(crate) async fn execute_run(
    state: AppState,
    job: WorkflowJob,
    _agent_lease: LeaseGuard,
    _execution_lease: ExecutionGuard,
) {
    loop {
        let Some(run) = state.workflow_runs.get(&job.run_id) else {
            fail_operational(&state, &job);
            return;
        };
        if run.is_terminal() {
            fail_operational(&state, &job);
            return;
        }
        if job.job.cancel_requested() {
            if persist_cancel(&state, &job.run_id).is_err() {
                fail_operational(&state, &job);
            } else {
                settle_job(&state, &job, JobStatus::Cancelled, None);
            }
            return;
        }
        if matches!(run.source, crate::workflows::run::RunSource::Pending) {
            job.job.set_step_label("Source capture".to_owned());
            if let Err(error) = capture_initial_source(&state, &job).await {
                if persist_initial_fail(&state, &job.run_id).is_err() {
                    fail_operational(&state, &job);
                } else {
                    settle_job(&state, &job, JobStatus::Failed, Some(&error));
                }
                return;
            }
            continue;
        }
        let Some(step_key) = run.ready_step().cloned() else {
            fail_operational(&state, &job);
            return;
        };
        let Some(step) = run.pinned.definition.step(&step_key).cloned() else {
            fail_operational(&state, &job);
            return;
        };
        job.job.set_step_label(step.name.clone());
        let inputs = match resolve_inputs(&run, &step) {
            Ok(inputs) => inputs,
            Err(error) => {
                settle_job(&state, &job, JobStatus::Failed, Some(error));
                return;
            }
        };
        if let Err(error) = reject_stale_assurance(&state, &run, &inputs) {
            if persist_initial_fail(&state, &job.run_id).is_err() {
                fail_operational(&state, &job);
            } else {
                settle_job(&state, &job, JobStatus::Failed, Some(error));
            }
            return;
        }
        let attempt_id = match AttemptId::generate() {
            Ok(id) => id,
            Err(_) => {
                fail_operational(&state, &job);
                return;
            }
        };
        let Some(agent) = state.agents.get(&job.agent_id) else {
            fail_operational(&state, &job);
            return;
        };
        let capabilities = match crate::workflows::capabilities::AttemptCapabilities::derive(
            &step,
            &agent,
            &job.connection,
        ) {
            Ok(capabilities) => capabilities,
            Err(error) => {
                settle_job(&state, &job, JobStatus::Failed, Some(error.message()));
                return;
            }
        };
        let Some(snapshot_digest) = run
            .environments
            .steps
            .iter()
            .find(|item| item.step == step.key)
            .map(|item| item.snapshot_digest.clone())
        else {
            fail_operational(&state, &job);
            return;
        };
        let sandbox_record = crate::workflows::run::AttemptSandboxRecord {
            kind: crate::workflows::run::AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest,
        };
        if persist_start(
            &state,
            &job.run_id,
            attempt_id,
            inputs.clone(),
            capabilities.clone(),
            sandbox_record,
        )
        .is_err()
        {
            fail_operational(&state, &job);
            return;
        }
        let isolated =
            isolate_and_run(&state, &job, &step, attempt_id, &inputs, &capabilities).await;
        let (outcome, cleanup, drafts, captured) = match isolated {
            IsolatedRun::Finished {
                outcome,
                cleanup,
                drafts,
                captured,
            } => (outcome, cleanup, drafts, captured),
        };
        if persist_cleanup(&state, &job.run_id, attempt_id, cleanup).is_err() {
            fail_operational(&state, &job);
            return;
        }
        if let Err(error) = finalise_attempt(
            &state,
            &job,
            &step,
            attempt_id,
            &inputs,
            &drafts,
            captured.as_ref(),
            &outcome,
        )
        .await
        {
            fail_operational(&state, &job);
            let _ = error;
            return;
        }
        match outcome {
            StepOutcome::Completed => {
                if let Some(run) = state.workflow_runs.get(&job.run_id)
                    && run.is_terminal()
                {
                    settle_job(&state, &job, JobStatus::Completed, None);
                    return;
                }
            }
            StepOutcome::Failed { error, .. } => {
                settle_job(&state, &job, JobStatus::Failed, error.as_deref());
                return;
            }
            StepOutcome::Cancelled => {
                settle_job(&state, &job, JobStatus::Cancelled, None);
                return;
            }
        }
    }
}

enum StepOutcome {
    Completed,
    Failed {
        category: FailureCategory,
        error: Option<String>,
    },
    Cancelled,
}

enum IsolatedRun {
    Finished {
        outcome: StepOutcome,
        cleanup: crate::workflows::run::AttemptCleanupRecord,
        drafts: std::sync::Arc<std::sync::Mutex<crate::workflows::artefacts::output::OutputDrafts>>,
        captured: Option<crate::workflows::artefacts::candidate::CandidateRevisionArtefact>,
    },
}

async fn isolate_and_run(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
    attempt_id: AttemptId,
    inputs: &[super::run::AttemptArtefactInput],
    capabilities: &crate::workflows::capabilities::AttemptCapabilities,
) -> IsolatedRun {
    let drafts = std::sync::Arc::new(std::sync::Mutex::new(
        crate::workflows::artefacts::output::OutputDrafts::default(),
    ));
    job.job.set_step_label("Materialising source".to_owned());
    if job.job.cancel_requested() {
        return IsolatedRun::Finished {
            outcome: StepOutcome::Cancelled,
            cleanup: crate::workflows::run::AttemptCleanupRecord::Complete,
            drafts,
            captured: None,
        };
    }
    let Some(candidate_input) = load_candidate_input(state, job, inputs) else {
        return IsolatedRun::Finished {
            outcome: StepOutcome::Failed {
                category: FailureCategory::Definition,
                error: Some("A sandbox-backed step needs a candidate input.".to_owned()),
            },
            cleanup: crate::workflows::run::AttemptCleanupRecord::Complete,
            drafts,
            captured: None,
        };
    };
    let workspace = match state
        .workflow_workspaces
        .create_attempt(job.run_id, attempt_id)
    {
        Ok(workspace) => workspace,
        Err(error) => {
            let cleanup = if error.orphaned {
                crate::workflows::run::AttemptCleanupRecord::Orphaned {
                    sandbox: false,
                    workspace: true,
                }
            } else {
                crate::workflows::run::AttemptCleanupRecord::Complete
            };
            return IsolatedRun::Finished {
                outcome: fail_for_orphan(
                    StepOutcome::Failed {
                        category: FailureCategory::Operational,
                        error: Some(
                            "Power Plant could not create the attempt workspace.".to_owned(),
                        ),
                    },
                    &cleanup,
                ),
                cleanup,
                drafts,
                captured: None,
            };
        }
    };
    let hash = candidate_input.artefact_hash;
    if crate::workflows::artefacts::CandidateMaterialise::into_workspace(
        &workspace.project,
        &candidate_input.artefact,
        hash,
        &state.workflow_artefacts,
    )
    .is_err()
    {
        let (outcome, cleanup) = finish_workspace_only(
            workspace,
            StepOutcome::Failed {
                category: FailureCategory::Operational,
                error: Some("Power Plant could not materialise the source tree.".to_owned()),
            },
        );
        return IsolatedRun::Finished {
            outcome,
            cleanup,
            drafts,
            captured: None,
        };
    }
    let user_project = match job
        .host_policy
        .grants()
        .iter()
        .find(|grant| grant.alias == job.host_policy.primary_alias())
    {
        Some(grant) => grant.host_path.clone(),
        None => {
            let (outcome, cleanup) = finish_workspace_only(
                workspace,
                StepOutcome::Failed {
                    category: FailureCategory::Operational,
                    error: Some(OPERATIONAL_STORE_ERROR.to_owned()),
                },
            );
            return IsolatedRun::Finished {
                outcome,
                cleanup,
                drafts,
                captured: None,
            };
        }
    };
    let git_dir = user_project.join(".git");
    if candidate_input.artefact.git_admin
        != match crate::workflows::artefacts::candidate::git_fingerprint(&git_dir) {
            Ok(value) => value,
            Err(_) => {
                let (outcome, cleanup) = finish_workspace_only(
                    workspace,
                    StepOutcome::Failed {
                        category: FailureCategory::Operational,
                        error: Some("The Git directory changed before that step.".to_owned()),
                    },
                );
                return IsolatedRun::Finished {
                    outcome,
                    cleanup,
                    drafts,
                    captured: None,
                };
            }
        }
    {
        let (outcome, cleanup) = finish_workspace_only(
            workspace,
            StepOutcome::Failed {
                category: FailureCategory::Operational,
                error: Some("The Git directory changed before that step.".to_owned()),
            },
        );
        return IsolatedRun::Finished {
            outcome,
            cleanup,
            drafts,
            captured: None,
        };
    }
    let sandbox = state.sandboxes.attempt_handle(job.run_id, attempt_id);
    if let Err(error) =
        start_attempt_sandbox(state, job, step, capabilities, &workspace, sandbox.clone()).await
    {
        let outcome = if job.job.cancel_requested() {
            StepOutcome::Cancelled
        } else {
            StepOutcome::Failed {
                category: FailureCategory::Operational,
                error: Some(error.to_owned()),
            }
        };
        let (outcome, cleanup) =
            cleanup_after_start_failure(state, attempt_id, sandbox, workspace, outcome).await;
        return IsolatedRun::Finished {
            outcome,
            cleanup,
            drafts,
            captured: None,
        };
    }
    let outcome = dispatch_step(state, job, step, &sandbox, drafts.clone()).await;
    job.job.set_step_label("Capturing outputs".to_owned());
    let stopped = sandbox.stop().await.is_ok();
    let captured = if stopped {
        crate::workflows::artefacts::CandidateCapture::capture_worktree(
            &workspace.project,
            &git_dir,
            &candidate_input.artefact.git_admin,
            &state.workflow_artefacts,
        )
        .ok()
    } else {
        None
    };
    job.job.set_step_label("Cleaning up".to_owned());
    let sandbox_gone = if stopped {
        sandbox.remove().await.is_ok()
    } else {
        false
    };
    if sandbox_gone {
        state.sandboxes.drop_attempt(attempt_id);
    } else {
        state.sandboxes.expose_orphan(sandbox.name().to_owned());
    }
    let workspace_gone = if sandbox_gone {
        workspace.destroy().is_ok()
    } else {
        false
    };
    let cleanup = if sandbox_gone && workspace_gone {
        crate::workflows::run::AttemptCleanupRecord::Complete
    } else {
        crate::workflows::run::AttemptCleanupRecord::Orphaned {
            sandbox: !sandbox_gone,
            workspace: !workspace_gone,
        }
    };
    let mut outcome = match (outcome, stopped, captured.is_some()) {
        (StepOutcome::Completed, true, true) => StepOutcome::Completed,
        (StepOutcome::Completed, _, _) => StepOutcome::Failed {
            category: FailureCategory::Operational,
            error: Some("Power Plant could not capture isolated outputs.".to_owned()),
        },
        (other, _, _) => other,
    };
    outcome = fail_for_orphan(outcome, &cleanup);
    IsolatedRun::Finished {
        outcome,
        cleanup,
        drafts,
        captured,
    }
}

fn finish_workspace_only(
    workspace: crate::workflows::workspace::AttemptWorkspace,
    outcome: StepOutcome,
) -> (StepOutcome, crate::workflows::run::AttemptCleanupRecord) {
    let cleanup = if workspace.destroy().is_ok() {
        crate::workflows::run::AttemptCleanupRecord::Complete
    } else {
        crate::workflows::run::AttemptCleanupRecord::Orphaned {
            sandbox: false,
            workspace: true,
        }
    };
    let outcome = fail_for_orphan(outcome, &cleanup);
    (outcome, cleanup)
}

async fn cleanup_after_start_failure(
    state: &AppState,
    attempt_id: AttemptId,
    sandbox: Arc<GuestSandbox>,
    workspace: crate::workflows::workspace::AttemptWorkspace,
    outcome: StepOutcome,
) -> (StepOutcome, crate::workflows::run::AttemptCleanupRecord) {
    let stopped = sandbox.stop().await.is_ok();
    let sandbox_gone = stopped && sandbox.remove().await.is_ok();
    if sandbox_gone {
        state.sandboxes.drop_attempt(attempt_id);
    } else {
        state.sandboxes.expose_orphan(sandbox.name().to_owned());
    }
    let workspace_gone = sandbox_gone && workspace.destroy().is_ok();
    let cleanup = if sandbox_gone && workspace_gone {
        crate::workflows::run::AttemptCleanupRecord::Complete
    } else {
        crate::workflows::run::AttemptCleanupRecord::Orphaned {
            sandbox: !sandbox_gone,
            workspace: !workspace_gone,
        }
    };
    let outcome = fail_for_orphan(outcome, &cleanup);
    (outcome, cleanup)
}

fn fail_for_orphan(
    outcome: StepOutcome,
    cleanup: &crate::workflows::run::AttemptCleanupRecord,
) -> StepOutcome {
    if matches!(
        cleanup,
        crate::workflows::run::AttemptCleanupRecord::Complete
    ) {
        outcome
    } else {
        StepOutcome::Failed {
            category: FailureCategory::Cleanup,
            error: Some("Power Plant could not clean up the isolated sandbox.".to_owned()),
        }
    }
}

struct LoadedCandidate {
    artefact_hash: crate::workflows::artefacts::ArtefactHash,
    artefact: crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
}

fn load_candidate_input(
    state: &AppState,
    job: &WorkflowJob,
    inputs: &[super::run::AttemptArtefactInput],
) -> Option<LoadedCandidate> {
    let input = inputs.iter().find(|input| {
        input.artefact.kind == crate::workflows::definition::ArtefactKind::CandidateRevision
    })?;
    let run = state.workflow_runs.get(&job.run_id)?;
    let record = run.artefact(&input.artefact.id)?;
    let bytes = state.workflow_artefacts.get(&record.object_hash).ok()?;
    let artefact =
        crate::workflows::artefacts::candidate::CandidateRevisionArtefact::from_manifest_bytes(
            &bytes,
        )?;
    Some(LoadedCandidate {
        artefact_hash: record.artefact_hash,
        artefact,
    })
}

fn persist_cleanup(
    state: &AppState,
    run_id: &RunId,
    attempt_id: AttemptId,
    cleanup: crate::workflows::run::AttemptCleanupRecord,
) -> Result<(), StoreError> {
    state
        .workflow_runs
        .mutate(run_id, |run| run.record_cleanup(attempt_id, cleanup))
        .map(|_| ())
}

async fn dispatch_step(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
    sandbox: &std::sync::Arc<GuestSandbox>,
    drafts: std::sync::Arc<std::sync::Mutex<crate::workflows::artefacts::output::OutputDrafts>>,
) -> StepOutcome {
    match &step.action {
        StepAction::Agent(action) => run_agent_step(state, job, action, sandbox, drafts).await,
        StepAction::SystemCommand(action) => {
            run_system_command_step(sandbox, &job.job, action.command).await
        }
    }
}

async fn run_agent_step(
    state: &AppState,
    job: &WorkflowJob,
    action: &AgentStep,
    sandbox: &std::sync::Arc<GuestSandbox>,
    drafts: std::sync::Arc<std::sync::Mutex<crate::workflows::artefacts::output::OutputDrafts>>,
) -> StepOutcome {
    if let Some(record) = state.agents.get(&job.agent_id) {
        let directories: Vec<(String, AccessMode)> = record
            .directories
            .iter()
            .map(|grant| (grant.alias.clone(), grant.access))
            .collect();
        if !action.authority.allowed_by(
            &record.tools,
            directories
                .iter()
                .map(|(alias, access)| (alias.as_str(), *access)),
        ) {
            return StepOutcome::Failed {
                category: FailureCategory::Authority,
                error: Some(
                    "The pinned step authority exceeds the current agent ceiling.".to_owned(),
                ),
            };
        }
    }
    let policy = match intersect_authority(&action.authority, &job.host_policy) {
        Ok(policy) => policy,
        Err(()) => {
            return StepOutcome::Failed {
                category: FailureCategory::Authority,
                error: Some(
                    "The pinned step authority exceeds the current directory policy.".to_owned(),
                ),
            };
        }
    };
    let Some(role) = state
        .workflow_runs
        .get(&job.run_id)
        .and_then(|run| run.pinned.definition.role(&action.role).cloned())
    else {
        return StepOutcome::Failed {
            category: FailureCategory::Definition,
            error: Some(OPERATIONAL_STORE_ERROR.to_owned()),
        };
    };
    let agent_instructions = state
        .agents
        .get(&job.agent_id)
        .map(|record| record.instructions)
        .unwrap_or_default();
    let instructions = match (
        role.prompt_defaults.trim().is_empty(),
        agent_instructions.trim().is_empty(),
    ) {
        (true, true) => String::new(),
        (false, true) => role.prompt_defaults.clone(),
        (true, false) => agent_instructions,
        (false, false) => format!(
            "{}

{}",
            role.prompt_defaults.trim(),
            agent_instructions.trim()
        ),
    };
    let preamble = crate::agents::compose_role(
        &role.name,
        &role.expertise,
        &instructions,
        &action.authority.tools,
        &policy,
    );
    let spec = AgentRunSpec {
        agent_id: job.agent_id,
        revision: 0,
        preamble,
        tools: crate::tools::definitions_for_step(
            &action.authority.tools,
            &action.required_outputs,
        ),
        tool_ids: action.authority.tools.clone(),
        policy,
        connection: job.connection.clone(),
        sandbox: sandbox.clone(),
        output_drafts: Some(drafts),
        required_outputs: action.required_outputs.clone(),
    };
    let ended = crate::slices::run_agent_action(
        state,
        job.session_id,
        spec,
        job.turns.clone(),
        job.job.clone(),
    )
    .await;
    match ended.outcome {
        AgentOutcome::Completed => StepOutcome::Completed,
        AgentOutcome::ProviderFailure => StepOutcome::Failed {
            category: FailureCategory::Provider,
            error: ended.error,
        },
        AgentOutcome::ToolFailure => StepOutcome::Failed {
            category: FailureCategory::Tool,
            error: ended.error,
        },
        AgentOutcome::Cancelled => StepOutcome::Cancelled,
    }
}

async fn run_system_command_step(
    sandbox: &GuestSandbox,
    job: &Job,
    command: SystemCommandId,
) -> StepOutcome {
    let exec = guest_command(command);
    let mut session = match sandbox.exec_cmd(exec).await {
        Ok(session) => session,
        Err(_) => {
            return StepOutcome::Failed {
                category: FailureCategory::Command,
                error: Some("Power Plant could not run the command. Try again.".to_owned()),
            };
        }
    };
    let deadline = Instant::now() + COMMAND_DEADLINE;
    let mut drained = 0usize;
    let mut exit = None;
    loop {
        if job.cancel_requested() {
            session.kill().await;
            session.close().await;
            return StepOutcome::Cancelled;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            session.kill().await;
            session.close().await;
            return StepOutcome::Failed {
                category: FailureCategory::Command,
                error: Some("The command did not finish in time.".to_owned()),
            };
        }
        let event = tokio::select! {
            biased;
            _ = job.cancelled() => {
                session.kill().await;
                session.close().await;
                return StepOutcome::Cancelled;
            }
            _ = tokio::time::sleep(remaining) => {
                session.kill().await;
                session.close().await;
                return StepOutcome::Failed {
                    category: FailureCategory::Command,
                    error: Some("The command did not finish in time.".to_owned()),
                };
            }
            event = session.recv() => event,
        };
        let Some(event) = event else {
            break;
        };
        match event {
            CommandEvent::Output(text) => {
                drained = drained.saturating_add(text.len());
                if drained > COMMAND_OUTPUT_LIMIT {
                    session.kill().await;
                    session.close().await;
                    return StepOutcome::Failed {
                        category: FailureCategory::Command,
                        error: Some("The command output was too large.".to_owned()),
                    };
                }
            }
            CommandEvent::Exited(code) => {
                exit = Some(code);
                break;
            }
            CommandEvent::Failed => {
                session.close().await;
                return StepOutcome::Failed {
                    category: FailureCategory::Command,
                    error: Some("Power Plant could not run the command. Try again.".to_owned()),
                };
            }
        }
    }
    session.close().await;
    match exit {
        Some(0) => StepOutcome::Completed,
        _ => StepOutcome::Failed {
            category: FailureCategory::Command,
            error: Some("The command did not succeed.".to_owned()),
        },
    }
}

pub(crate) fn guest_command(command: SystemCommandId) -> GuestExec {
    match command {
        SystemCommandId::RepositoryStatus => GuestExec::command(
            "git",
            vec!["status".to_owned(), "--porcelain=v1".to_owned()],
        )
        .in_dir(GUEST_PROJECT),
    }
}

fn intersect_authority(
    authority: &AgentAuthority,
    host: &DirectoryPolicy,
) -> Result<DirectoryPolicy, ()> {
    let mut grants = Vec::new();
    for directory in &authority.directories {
        let Some(host_grant) = host
            .grants()
            .iter()
            .find(|grant| grant.alias == directory.alias)
        else {
            return Err(());
        };
        if directory.access.is_writable() && !host_grant.access.is_writable() {
            return Err(());
        }
        grants.push(PolicyGrant {
            alias: host_grant.alias.clone(),
            guest_path: host_grant.guest_path.clone(),
            host_path: host_grant.host_path.clone(),
            access: min_access(directory.access, host_grant.access),
        });
    }
    if grants.is_empty() {
        return Err(());
    }
    let primary = if grants
        .iter()
        .any(|grant| grant.alias == host.primary_alias())
    {
        host.primary_alias().to_owned()
    } else {
        grants[0].alias.clone()
    };
    Ok(DirectoryPolicy::from_grants(grants, primary))
}

fn min_access(left: AccessMode, right: AccessMode) -> AccessMode {
    if left.is_writable() && right.is_writable() {
        AccessMode::ReadWrite
    } else {
        AccessMode::ReadOnly
    }
}

async fn start_attempt_sandbox(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
    capabilities: &crate::workflows::capabilities::AttemptCapabilities,
    workspace: &crate::workflows::workspace::AttemptWorkspace,
    sandbox: std::sync::Arc<GuestSandbox>,
) -> Result<(), &'static str> {
    job.job.set_step_label("Preparing environment".to_owned());
    if job.job.cancel_requested() {
        return Err("The task was cancelled.");
    }
    let run = state
        .workflow_runs
        .get(&job.run_id)
        .ok_or(OPERATIONAL_STORE_ERROR)?;
    let set = &run.environments;
    let binding = set
        .steps
        .iter()
        .find(|item| item.step == step.key)
        .ok_or(OPERATIONAL_STORE_ERROR)?;
    let environment = set
        .environments
        .iter()
        .find(|item| item.environment_id == binding.environment_id)
        .ok_or(OPERATIONAL_STORE_ERROR)?;
    state
        .environment_snapshots
        .matches_pin(&environment.snapshot)
        .await
        .map_err(|_| "That environment snapshot is unavailable.")?;
    let path = state
        .environment_snapshots
        .restore_path(&environment.snapshot.artifact_key)
        .map_err(|_| "That environment snapshot is unavailable.")?;
    let user_project = job
        .host_policy
        .grants()
        .iter()
        .find(|grant| grant.alias == job.host_policy.primary_alias())
        .ok_or("Choose a project directory.")?;
    let spec = attempt_spec(
        capabilities,
        workspace,
        &user_project.host_path,
        &job.host_policy,
        capabilities.guest_access(&job.connection),
    )?;
    crate::sandbox::reject_user_project_write(&spec, &user_project.host_path)
        .map_err(|error| error.message())?;
    if job.job.cancel_requested() {
        return Err("The task was cancelled.");
    }
    sandbox
        .start_from_snapshot(&path, environment.snapshot.snapshot_digest.as_str(), spec)
        .await
        .map_err(|error| error.message())?;
    job.job.set_step_label(step.name.clone());
    Ok(())
}

fn attempt_spec(
    capabilities: &crate::workflows::capabilities::AttemptCapabilities,
    workspace: &crate::workflows::workspace::AttemptWorkspace,
    user_project: &std::path::Path,
    host: &DirectoryPolicy,
    access: crate::sandbox::GuestAccess,
) -> Result<crate::sandbox::SandboxSpec, &'static str> {
    let mut mounts = Vec::new();
    let Some(primary) = capabilities.primary() else {
        return Err("A sandbox-backed step needs a primary source.");
    };
    mounts.push(crate::sandbox::MountSpec {
        guest: primary.guest_path.clone(),
        host: workspace.project.clone(),
        read_only: !primary.access.is_writable(),
    });
    let git = user_project.join(".git");
    if !git.is_dir() {
        return Err("The project is not a supported Git worktree.");
    }
    mounts.push(crate::sandbox::MountSpec {
        guest: format!("{}/.git", primary.guest_path),
        host: git,
        read_only: true,
    });
    for directory in &capabilities.directories {
        if directory.role != crate::workflows::capabilities::DirectoryRole::SecondaryContext {
            continue;
        }
        let Some(grant) = host
            .grants()
            .iter()
            .find(|grant| grant.alias == directory.alias)
        else {
            return Err("The pinned step authority exceeds the current directory policy.");
        };
        mounts.push(crate::sandbox::MountSpec {
            guest: directory.guest_path.clone(),
            host: grant.host_path.clone(),
            read_only: true,
        });
    }
    Ok(crate::sandbox::SandboxSpec {
        mounts,
        workdir: primary.guest_path.clone(),
        access,
    })
}

async fn capture_initial_source(state: &AppState, job: &WorkflowJob) -> Result<(), String> {
    let host = job
        .host_policy
        .grants()
        .iter()
        .find(|grant| grant.alias == job.host_policy.primary_alias())
        .ok_or_else(|| "Choose a project directory.".to_owned())?;
    let captured = match crate::workflows::artefacts::CandidateCapture::capture_host(
        &host.host_path,
        &state.workflow_artefacts,
    ) {
        Ok(captured) => captured,
        Err(error) => return Err(error.message().to_owned()),
    };
    let bytes = captured
        .manifest_bytes()
        .map_err(|error| error.message().to_owned())?;
    let object = state
        .workflow_artefacts
        .publish(&bytes)
        .map_err(|error| error.message().to_owned())?;
    let artefact_hash = crate::workflows::artefacts::artefact_hash_for(
        crate::workflows::definition::ArtefactKind::CandidateRevision,
        crate::workflows::artefacts::CANDIDATE_SCHEMA,
        &bytes,
    );
    let id = crate::workflows::id::ArtefactId::generate()
        .map_err(|_| OPERATIONAL_STORE_ERROR.to_owned())?;
    let record = crate::workflows::artefacts::ArtefactRecord {
        id,
        kind: crate::workflows::definition::ArtefactKind::CandidateRevision,
        artefact_hash,
        object_hash: object,
        payload_bytes: bytes.len() as u64,
        created_at_ms: now_ms(),
        provenance: crate::workflows::artefacts::ArtefactProvenance {
            run_id: job.run_id,
            producer: crate::workflows::artefacts::ArtefactProducer::RunSourceCapture,
            inputs: Vec::new(),
        },
        summary: crate::workflows::artefacts::ArtefactSummary::Candidate {
            candidate: captured.candidate_hash,
            entries: captured.entries.len() as u64,
            bytes: captured
                .entries
                .iter()
                .map(|entry| match entry.kind {
                    crate::workflows::artefacts::CandidateEntryKind::Regular { bytes, .. } => bytes,
                    _ => 0,
                })
                .sum(),
            disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
        },
    };
    state
        .workflow_runs
        .mutate(&job.run_id, |run| run.record_initial_candidate(record))
        .map(|_| ())
        .map_err(|_| OPERATIONAL_STORE_ERROR.to_owned())
}

fn persist_initial_fail(state: &AppState, run_id: &RunId) -> Result<(), StoreError> {
    let at_ms = now_ms();
    state
        .workflow_runs
        .mutate(run_id, |run| run.fail_before_attempt(at_ms))
        .map(|_| ())
}

fn resolve_inputs(
    run: &crate::workflows::WorkflowRun,
    step: &StepDefinition,
) -> Result<Vec<super::run::AttemptArtefactInput>, &'static str> {
    let mut inputs = Vec::new();
    for input in &step.inputs {
        let artefact = match &input.source {
            crate::workflows::definition::ArtefactSource::RunInitialCandidate => {
                let crate::workflows::RunSource::Captured { source } = &run.source else {
                    return Err("Source capture has not finished.");
                };
                source.initial.clone()
            }
            crate::workflows::definition::ArtefactSource::StepOutput {
                step: source_step,
                output,
            } => {
                let Some(attempt) = run.attempts.iter().rev().find(|attempt| {
                    attempt.step == *source_step
                        && matches!(
                            attempt.result,
                            Some(crate::workflows::run::AttemptResult::Completed { .. })
                        )
                }) else {
                    return Err("That input step has no successful output.");
                };
                let Some(found) = attempt
                    .outputs
                    .iter()
                    .find(|item| item.key == *output)
                    .map(|item| item.artefact.clone())
                else {
                    if output.as_str() == crate::workflows::definition::ASSISTANT_REPLY {
                        return Err("Assistant replies cannot be artefact inputs.");
                    }
                    return Err("That input names an unknown output.");
                };
                found
            }
        };
        if artefact.kind != input.kind {
            return Err("That input kind does not match the named output.");
        }
        inputs.push(super::run::AttemptArtefactInput {
            key: input.key.clone(),
            artefact,
        });
    }
    Ok(inputs)
}

fn reject_stale_assurance(
    state: &AppState,
    run: &crate::workflows::WorkflowRun,
    inputs: &[super::run::AttemptArtefactInput],
) -> Result<(), &'static str> {
    let Some(candidate) = inputs
        .iter()
        .find(|input| {
            input.artefact.kind == crate::workflows::definition::ArtefactKind::CandidateRevision
        })
        .and_then(|input| run.artefact(&input.artefact.id))
        .and_then(candidate_hash_of)
    else {
        return Ok(());
    };
    for input in inputs {
        if !input.artefact.kind.is_assurance() {
            continue;
        }
        let Some(record) = run.artefact(&input.artefact.id) else {
            return Err("That assurance artefact is missing.");
        };
        let Ok(bytes) = state.workflow_artefacts.get(&record.object_hash) else {
            return Err("That assurance artefact is missing.");
        };
        let Ok(payload) = crate::workflows::artefacts::parse_typed_payload(record.kind, &bytes)
        else {
            return Err("That assurance artefact is unreadable.");
        };
        let Some(bound) = crate::workflows::artefacts::assurance::candidate_constraint(&payload)
        else {
            return Err("That assurance artefact is unreadable.");
        };
        if bound != candidate {
            return Err("That assurance artefact is stale.");
        }
    }
    Ok(())
}

fn candidate_hash_of(
    record: &crate::workflows::artefacts::ArtefactRecord,
) -> Option<crate::workflows::artefacts::CandidateHash> {
    match &record.summary {
        crate::workflows::artefacts::ArtefactSummary::Candidate { candidate, .. }
        | crate::workflows::artefacts::ArtefactSummary::Review { candidate, .. }
        | crate::workflows::artefacts::ArtefactSummary::Test { candidate, .. } => Some(*candidate),
        crate::workflows::artefacts::ArtefactSummary::Plan { .. } => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn finalise_attempt(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
    attempt_id: AttemptId,
    inputs: &[super::run::AttemptArtefactInput],
    drafts: &std::sync::Mutex<crate::workflows::artefacts::output::OutputDrafts>,
    captured: Option<&crate::workflows::artefacts::candidate::CandidateRevisionArtefact>,
    outcome: &StepOutcome,
) -> Result<(), StoreError> {
    match outcome {
        StepOutcome::Cancelled => persist_cancel(state, &job.run_id),
        StepOutcome::Failed { category, .. } => {
            if step.writes_primary_source() {
                record_observed(state, job, attempt_id, step, inputs, captured)
                    .map_err(|_| StoreError::Persist)?;
            }
            persist_fail(state, &job.run_id, Some(attempt_id), *category)
        }
        StepOutcome::Completed => {
            if let Err(error) =
                publish_success(state, job, step, attempt_id, inputs, drafts, captured)
            {
                persist_fail(
                    state,
                    &job.run_id,
                    Some(attempt_id),
                    FailureCategory::Definition,
                )?;
                settle_job(state, job, JobStatus::Failed, Some(error));
                return Ok(());
            }
            persist_outcome(state, &job.run_id, attempt_id, outcome)
        }
    }
}

fn record_observed(
    state: &AppState,
    job: &WorkflowJob,
    attempt_id: AttemptId,
    step: &StepDefinition,
    inputs: &[super::run::AttemptArtefactInput],
    captured: Option<&crate::workflows::artefacts::candidate::CandidateRevisionArtefact>,
) -> Result<(), &'static str> {
    let Some(captured) = captured else {
        return record_unknown_observed(state, &job.run_id, attempt_id)
            .map_err(|_| OPERATIONAL_STORE_ERROR);
    };
    let record = publish_candidate(
        state,
        job,
        captured,
        crate::workflows::artefacts::ArtefactProducer::StepAttempt {
            attempt_id,
            step: step.key.clone(),
            output: None,
            disposition: crate::workflows::artefacts::ProductionDisposition::ObservedAfterFailure,
        },
        inputs,
    )?;
    let observed = super::run::ObservedCandidate::Exact {
        artefact: super::artefacts::ArtefactReference {
            id: record.id,
            kind: record.kind,
            artefact_hash: record.artefact_hash,
        },
    };
    state
        .workflow_runs
        .mutate(&job.run_id, |run| {
            run.record_attempt_outputs(attempt_id, vec![record], Vec::new(), None, observed)
        })
        .map(|_| ())
        .map_err(|_| OPERATIONAL_STORE_ERROR)
}

fn record_unknown_observed(
    state: &AppState,
    run_id: &RunId,
    attempt_id: AttemptId,
) -> Result<(), StoreError> {
    state
        .workflow_runs
        .mutate(run_id, |run| {
            run.record_attempt_outputs(
                attempt_id,
                Vec::new(),
                Vec::new(),
                None,
                super::run::ObservedCandidate::Unknown,
            )
        })
        .map(|_| ())
}

fn publish_success(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
    attempt_id: AttemptId,
    inputs: &[super::run::AttemptArtefactInput],
    drafts: &std::sync::Mutex<crate::workflows::artefacts::output::OutputDrafts>,
    captured: Option<&crate::workflows::artefacts::candidate::CandidateRevisionArtefact>,
) -> Result<(), &'static str> {
    let captured = captured.ok_or("Power Plant could not capture isolated outputs.")?;
    let writes = step.writes_primary_source();
    let expected = inputs
        .iter()
        .find(|input| {
            input.artefact.kind == crate::workflows::definition::ArtefactKind::CandidateRevision
        })
        .and_then(|input| {
            state
                .workflow_runs
                .get(&job.run_id)
                .and_then(|run| run.artefact(&input.artefact.id).cloned())
        })
        .and_then(|record| candidate_hash_of(&record));
    if !writes && expected.is_some_and(|hash| hash != captured.candidate_hash) {
        return Err("The project changed during that step.");
    }
    let mut artefacts = Vec::new();
    let mut outputs = Vec::new();
    let mut accepted = None;
    let mut observed = match inputs.iter().find(|input| {
        input.artefact.kind == crate::workflows::definition::ArtefactKind::CandidateRevision
    }) {
        Some(input) => super::run::ObservedCandidate::Exact {
            artefact: input.artefact.clone(),
        },
        None => super::run::ObservedCandidate::Unknown,
    };
    if writes {
        let output = step
            .required_outputs()
            .iter()
            .find(|item| item.kind == crate::workflows::definition::OutputKind::CandidateRevision)
            .ok_or("A source-write step must produce exactly one candidate revision.")?;
        let record = publish_candidate(
            state,
            job,
            captured,
            crate::workflows::artefacts::ArtefactProducer::StepAttempt {
                attempt_id,
                step: step.key.clone(),
                output: Some(output.key.clone()),
                disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
            },
            inputs,
        )?;
        let reference = super::artefacts::ArtefactReference {
            id: record.id,
            kind: record.kind,
            artefact_hash: record.artefact_hash,
        };
        accepted = Some(reference.clone());
        observed = super::run::ObservedCandidate::Exact {
            artefact: reference.clone(),
        };
        outputs.push(super::run::AttemptArtefactOutput {
            key: output.key.clone(),
            artefact: reference,
        });
        artefacts.push(record);
    }
    let candidate_hash = captured.candidate_hash;
    let secret = match &job.connection.auth {
        crate::providers::AuthMethod::ApiKey => Some(job.connection.api_key.expose().to_owned()),
        crate::providers::AuthMethod::Plan => None,
    };
    let mut held = drafts
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for output in step.required_outputs() {
        if matches!(
            output.kind,
            crate::workflows::definition::OutputKind::AssistantReply
                | crate::workflows::definition::OutputKind::CandidateRevision
        ) {
            continue;
        }
        let Some(draft) = held.take(&output.key) else {
            return Err("A required output is missing.");
        };
        let record = publish_draft(
            state,
            job,
            attempt_id,
            step,
            output,
            draft,
            candidate_hash,
            inputs,
            secret.as_deref(),
        )?;
        outputs.push(super::run::AttemptArtefactOutput {
            key: output.key.clone(),
            artefact: super::artefacts::ArtefactReference {
                id: record.id,
                kind: record.kind,
                artefact_hash: record.artefact_hash,
            },
        });
        artefacts.push(record);
    }
    drop(held);
    state
        .workflow_runs
        .mutate(&job.run_id, |run| {
            run.record_attempt_outputs(attempt_id, artefacts, outputs, accepted, observed)
        })
        .map(|_| ())
        .map_err(|_| OPERATIONAL_STORE_ERROR)
}

fn publish_candidate(
    state: &AppState,
    job: &WorkflowJob,
    captured: &crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
    producer: crate::workflows::artefacts::ArtefactProducer,
    inputs: &[super::run::AttemptArtefactInput],
) -> Result<crate::workflows::artefacts::ArtefactRecord, &'static str> {
    let bytes = captured
        .manifest_bytes()
        .map_err(|_| OPERATIONAL_STORE_ERROR)?;
    let object = state
        .workflow_artefacts
        .publish(&bytes)
        .map_err(|_| OPERATIONAL_STORE_ERROR)?;
    let artefact_hash = crate::workflows::artefacts::artefact_hash_for(
        crate::workflows::definition::ArtefactKind::CandidateRevision,
        crate::workflows::artefacts::CANDIDATE_SCHEMA,
        &bytes,
    );
    let id = crate::workflows::ArtefactId::generate().map_err(|_| OPERATIONAL_STORE_ERROR)?;
    Ok(crate::workflows::artefacts::ArtefactRecord {
        id,
        kind: crate::workflows::definition::ArtefactKind::CandidateRevision,
        artefact_hash,
        object_hash: object,
        payload_bytes: bytes.len() as u64,
        created_at_ms: now_ms(),
        provenance: crate::workflows::artefacts::ArtefactProvenance {
            run_id: job.run_id,
            producer: producer.clone(),
            inputs: inputs.iter().map(|input| input.artefact.clone()).collect(),
        },
        summary: crate::workflows::artefacts::ArtefactSummary::Candidate {
            candidate: captured.candidate_hash,
            entries: captured.entries.len() as u64,
            bytes: captured
                .entries
                .iter()
                .map(|entry| match entry.kind {
                    crate::workflows::artefacts::CandidateEntryKind::Regular { bytes, .. } => bytes,
                    _ => 0,
                })
                .sum(),
            disposition: match producer {
                crate::workflows::artefacts::ArtefactProducer::StepAttempt {
                    disposition, ..
                } => disposition,
                _ => crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
            },
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn publish_draft(
    state: &AppState,
    job: &WorkflowJob,
    attempt_id: AttemptId,
    step: &StepDefinition,
    output: &crate::workflows::definition::RequiredOutput,
    draft: crate::workflows::artefacts::output::OutputDraft,
    candidate: crate::workflows::artefacts::CandidateHash,
    inputs: &[super::run::AttemptArtefactInput],
    secret: Option<&str>,
) -> Result<crate::workflows::artefacts::ArtefactRecord, &'static str> {
    use crate::workflows::artefacts::output::OutputDraft;
    let (bytes, object, artefact_hash, kind, summary) = match draft {
        OutputDraft::Plan { markdown } => {
            let (bytes, object, hash) =
                crate::workflows::artefacts::payload::encode_plan(&markdown, secret)
                    .map_err(|_| "That plan output is not valid.")?;
            (
                bytes,
                object,
                hash,
                crate::workflows::definition::ArtefactKind::Plan,
                crate::workflows::artefacts::ArtefactSummary::Plan {
                    markdown_bytes: markdown.len() as u64,
                },
            )
        }
        OutputDraft::Review { verdict, markdown } => {
            let (bytes, object, hash) = crate::workflows::artefacts::payload::encode_review(
                candidate, verdict, &markdown, secret,
            )
            .map_err(|_| "That review output is not valid.")?;
            (
                bytes,
                object,
                hash,
                crate::workflows::definition::ArtefactKind::ReviewReport,
                crate::workflows::artefacts::ArtefactSummary::Review { candidate, verdict },
            )
        }
        OutputDraft::Test { outcome, markdown } => {
            let (bytes, object, hash) = crate::workflows::artefacts::payload::encode_test(
                candidate, outcome, &markdown, secret,
            )
            .map_err(|_| "That test output is not valid.")?;
            (
                bytes,
                object,
                hash,
                crate::workflows::definition::ArtefactKind::TestReport,
                crate::workflows::artefacts::ArtefactSummary::Test { candidate, outcome },
            )
        }
    };
    state
        .workflow_artefacts
        .publish(&bytes)
        .map_err(|_| OPERATIONAL_STORE_ERROR)?;
    let id = crate::workflows::ArtefactId::generate().map_err(|_| OPERATIONAL_STORE_ERROR)?;
    Ok(crate::workflows::artefacts::ArtefactRecord {
        id,
        kind,
        artefact_hash,
        object_hash: object,
        payload_bytes: bytes.len() as u64,
        created_at_ms: now_ms(),
        provenance: crate::workflows::artefacts::ArtefactProvenance {
            run_id: job.run_id,
            producer: crate::workflows::artefacts::ArtefactProducer::StepAttempt {
                attempt_id,
                step: step.key.clone(),
                output: Some(output.key.clone()),
                disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
            },
            inputs: inputs.iter().map(|input| input.artefact.clone()).collect(),
        },
        summary,
    })
}

fn persist_start(
    state: &AppState,
    run_id: &RunId,
    attempt_id: AttemptId,
    inputs: Vec<super::run::AttemptArtefactInput>,
    capabilities: crate::workflows::capabilities::AttemptCapabilities,
    sandbox: crate::workflows::run::AttemptSandboxRecord,
) -> Result<(), StoreError> {
    let at_ms = now_ms();
    state
        .workflow_runs
        .mutate(run_id, |run| {
            run.start_attempt(attempt_id, inputs, capabilities, sandbox, at_ms)
        })
        .map(|_| ())
}

fn persist_outcome(
    state: &AppState,
    run_id: &RunId,
    attempt_id: AttemptId,
    outcome: &StepOutcome,
) -> Result<(), StoreError> {
    let at_ms = now_ms();
    match outcome {
        StepOutcome::Completed => state
            .workflow_runs
            .mutate(run_id, |run| run.complete_attempt(attempt_id, at_ms))
            .map(|_| ()),
        StepOutcome::Failed { category, .. } => {
            persist_fail(state, run_id, Some(attempt_id), *category)
        }
        StepOutcome::Cancelled => persist_cancel(state, run_id),
    }
}

fn persist_fail(
    state: &AppState,
    run_id: &RunId,
    attempt_id: Option<AttemptId>,
    category: FailureCategory,
) -> Result<(), StoreError> {
    let at_ms = now_ms();
    state
        .workflow_runs
        .mutate(run_id, |run| {
            if let Some(attempt_id) = attempt_id.or(run.active_attempt()) {
                run.fail_attempt(attempt_id, category, at_ms)
            } else {
                Err(crate::workflows::run::TransitionError::Invalid)
            }
        })
        .map(|_| ())
}

fn persist_cancel(state: &AppState, run_id: &RunId) -> Result<(), StoreError> {
    let at_ms = now_ms();
    state
        .workflow_runs
        .mutate(run_id, |run| run.cancel(at_ms))
        .map(|_| ())
}

fn fail_operational(state: &AppState, workflow: &WorkflowJob) {
    settle_job(
        state,
        workflow,
        JobStatus::Failed,
        Some(OPERATIONAL_STORE_ERROR),
    );
}

fn settle_job(state: &AppState, workflow: &WorkflowJob, status: JobStatus, error: Option<&str>) {
    settle_transient_job(
        state,
        &workflow.session_id,
        &workflow.agent_id,
        &workflow.job,
        status,
        error,
    );
}

fn settle_transient_job(
    state: &AppState,
    session_id: &SessionId,
    agent_id: &crate::agents::AgentId,
    job: &Job,
    status: JobStatus,
    error: Option<&str>,
) {
    let _ = state
        .sessions
        .fail_turn(session_id, agent_id, &job.id(), String::new());
    let _ = job.finish(status, error);
}

#[cfg(test)]
mod tests;
