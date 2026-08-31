use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::agents::{AccessMode, DirectoryPolicy, LeaseGuard, PolicyGrant};
use crate::projects::ProjectId;
use crate::providers::{ChatTurn, ProviderConnection};
use crate::sandbox::{CommandEvent, GUEST_PROJECT, GuestExec, GuestSandbox};
use crate::sessions::{Job, JobStatus, SessionId};
use crate::slices::{AgentOutcome, AgentRunSpec};
use crate::state::AppState;

use super::definition::{
    AgentAuthority, AgentStep, CandidateAuthority, StepAction, StepDefinition, SystemCommandId,
};
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

pub(crate) struct WorkflowContinuationRegistry {
    inner: std::sync::Mutex<std::collections::BTreeMap<RunId, WorkflowJob>>,
}

impl WorkflowContinuationRegistry {
    pub(crate) fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    pub(crate) fn insert(&self, job: WorkflowJob) -> bool {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.contains_key(&job.run_id) {
            return false;
        }
        inner.insert(job.run_id, job);
        true
    }

    pub(crate) fn take(&self, run: &RunId) -> Option<WorkflowJob> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(run)
    }

    pub(crate) fn available(&self, run: &RunId, session: &SessionId) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(run)
            .is_some_and(|job| job.session_id == *session)
    }

    pub(crate) fn put_back(&self, job: WorkflowJob) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(job.run_id, job);
    }

    fn take_provider(&self, provider: crate::providers::ProviderKind) -> Vec<WorkflowJob> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ids: Vec<_> = inner
            .iter()
            .filter(|(_, job)| job.connection.kind == provider)
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter().filter_map(|id| inner.remove(&id)).collect()
    }

    fn take_session(&self, session: SessionId) -> Vec<WorkflowJob> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ids: Vec<_> = inner
            .iter()
            .filter(|(_, job)| job.session_id == session)
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter().filter_map(|id| inner.remove(&id)).collect()
    }
}

pub(crate) struct WorkflowJob {
    pub(crate) run_id: RunId,
    pub(crate) session_id: SessionId,
    pub(crate) project_id: ProjectId,
    pub(crate) agent_id: crate::agents::AgentId,
    pub(crate) agent_revision: u32,
    pub(crate) grant_alias: String,
    pub(crate) grant_access: AccessMode,
    pub(crate) connection: ProviderConnection,
    pub(crate) host_policy: DirectoryPolicy,
    pub(crate) turns: Vec<ChatTurn>,
    pub(crate) job: Arc<Job>,
    pub(crate) eligible_reply: Arc<std::sync::Mutex<String>>,
}

pub(crate) fn interrupt_provider_continuations(
    state: &AppState,
    provider: crate::providers::ProviderKind,
) -> Result<(), StoreError> {
    interrupt_continuations(state, state.gate_continuations.take_provider(provider))
}

pub(crate) fn interrupt_session_continuations(
    state: &AppState,
    session: SessionId,
) -> Result<(), StoreError> {
    interrupt_continuations(state, state.gate_continuations.take_session(session))
}

fn interrupt_continuations(state: &AppState, jobs: Vec<WorkflowJob>) -> Result<(), StoreError> {
    let mut jobs = jobs.into_iter();
    while let Some(job) = jobs.next() {
        if state
            .workflow_runs
            .mutate(&job.run_id, |run| run.interrupt(now_ms()))
            .is_err()
        {
            state.gate_continuations.put_back(job);
            for unprocessed in jobs {
                state.gate_continuations.put_back(unprocessed);
            }
            return Err(StoreError::Persist);
        }
        let _ =
            state
                .sessions
                .fail_turn(&job.session_id, &job.agent_id, &job.job.id(), String::new());
        job.job.finish(JobStatus::Cancelled, None);
    }
    Ok(())
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
        job.job.set_step_label(active_step_label(&run, &step));
        let inputs = match resolve_inputs(&run, &step) {
            Ok(inputs) => inputs,
            Err(error) => {
                settle_job(&state, &job, JobStatus::Failed, Some(error));
                return;
            }
        };
        if let Err(error) = crate::workflows::input_context::verify_inputs(
            &run,
            &step,
            &inputs,
            &state.workflow_artefacts,
        ) {
            settle_job(&state, &job, JobStatus::Failed, Some(error.message()));
            return;
        }
        if matches!(step.action, StepAction::HumanGate(_)) {
            let Some(candidate) = inputs
                .iter()
                .find(|input| {
                    input.artefact.kind
                        == crate::workflows::definition::ArtefactKind::CandidateRevision
                })
                .map(|input| input.artefact.clone())
            else {
                settle_job(
                    &state,
                    &job,
                    JobStatus::Failed,
                    Some("A human gate needs a candidate input."),
                );
                return;
            };
            let crate::workflows::RunSource::Captured { source } = &run.source else {
                settle_job(
                    &state,
                    &job,
                    JobStatus::Failed,
                    Some(OPERATIONAL_STORE_ERROR),
                );
                return;
            };
            let Ok(gate_id) = crate::workflows::GateId::generate() else {
                settle_job(
                    &state,
                    &job,
                    JobStatus::Failed,
                    Some(OPERATIONAL_STORE_ERROR),
                );
                return;
            };
            let opened = state.workflow_runs.mutate(&job.run_id, |run| {
                run.open_gate(gate_id, candidate.clone(), source.initial.clone(), now_ms())
                    .map(|_| ())
            });
            if opened.is_err() {
                settle_job(
                    &state,
                    &job,
                    JobStatus::Failed,
                    Some(OPERATIONAL_STORE_ERROR),
                );
                return;
            }
            job.job.set_step_label("Awaiting decision".to_owned());
            let _ = job.job.set_awaiting_decision();
            if !state.gate_continuations.insert(job) {
                let _ = state
                    .workflow_runs
                    .mutate(&run.id, |run| run.interrupt(now_ms()));
            }
            return;
        }
        let commit_step = matches!(
            &step.action,
            crate::workflows::definition::StepAction::SystemCommand(action)
                if action.command == crate::workflows::commands::SystemCommandId::CommitCandidate
        );
        let commit_precondition = if commit_step {
            crate::workflows::commit::require_approved_review(
                &run,
                &step,
                &inputs,
                &state.workflow_artefacts,
            )
            .and_then(|_| {
                reject_stale_assurance(&state, &run, &inputs)
                    .map_err(|_| crate::workflows::commit::CommitError::Assurance)
            })
            .err()
        } else {
            if let Err(error) = reject_stale_assurance(&state, &run, &inputs) {
                settle_job(&state, &job, JobStatus::Failed, Some(error));
                return;
            }
            None
        };
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
        if let Some(error) = commit_precondition {
            let stored = persist_cleanup(
                &state,
                &job.run_id,
                attempt_id,
                crate::workflows::run::AttemptCleanupRecord::Complete,
            )
            .and_then(|_| {
                persist_fail(
                    &state,
                    &job.run_id,
                    Some(attempt_id),
                    FailureCategory::Assurance,
                )
            });
            if stored.is_err() {
                fail_operational(&state, &job);
            } else {
                settle_job(&state, &job, JobStatus::Failed, Some(error.message()));
            }
            return;
        }
        let isolated =
            isolate_and_run(&state, &job, &step, attempt_id, &inputs, &capabilities).await;
        let (mut outcome, mut cleanup, drafts, captured) = match isolated {
            IsolatedRun::Finished {
                outcome,
                cleanup,
                drafts,
                captured,
            } => (outcome, cleanup, drafts, captured),
        };
        let recovery_pending = state.workflow_runs.get(&job.run_id).is_some_and(|run| {
            run.attempts
                .iter()
                .find(|attempt| attempt.id == attempt_id)
                .and_then(|attempt| attempt.commit_transaction.as_ref())
                .is_some_and(|transaction| {
                    matches!(
                        transaction.state,
                        crate::workflows::commit::CommitTransactionState::ReferenceUpdated { .. }
                    )
                })
                && !matches!(outcome, StepOutcome::Completed)
        });
        if recovery_pending {
            if cleanup == crate::workflows::run::AttemptCleanupRecord::Complete
                && recover_commit_transactions(&state).is_ok()
                && state
                    .workflow_runs
                    .get(&job.run_id)
                    .is_some_and(|run| run.is_terminal())
            {
                settle_job(&state, &job, JobStatus::Completed, None);
            } else {
                settle_job(
                    &state,
                    &job,
                    JobStatus::Failed,
                    Some("Power Plant must recover the Git commit before this run can continue."),
                );
            }
            return;
        }
        let atomic_agent_publication = matches!(step.action, StepAction::Agent(_))
            && matches!(outcome, StepOutcome::Completed)
            && cleanup == crate::workflows::run::AttemptCleanupRecord::Complete;
        if atomic_agent_publication
            && persist_cleanup(&state, &job.run_id, attempt_id, cleanup.clone()).is_err()
        {
            fail_operational(&state, &job);
            return;
        }
        let mut published = false;
        if matches!(outcome, StepOutcome::Completed) {
            match publish_success(
                &state,
                &job,
                &step,
                SuccessAttempt {
                    id: attempt_id,
                    complete: atomic_agent_publication,
                },
                &inputs,
                &drafts,
                captured.as_ref(),
            ) {
                Ok(()) => published = true,
                Err(error) => {
                    outcome = StepOutcome::Failed {
                        category: FailureCategory::Definition,
                        error: Some(error.to_owned()),
                    };
                }
            }
        }
        if matches!(
            &step.action,
            StepAction::SystemCommand(action)
                if action.command == SystemCommandId::CommitCandidate
        ) {
            let retain_journal = state
                .workflow_runs
                .get(&job.run_id)
                .and_then(|run| {
                    run.attempts
                        .iter()
                        .find(|attempt| attempt.id == attempt_id)
                        .and_then(|attempt| attempt.commit_transaction.as_ref())
                        .map(|transaction| {
                            matches!(
                                transaction.state,
                                crate::workflows::commit::CommitTransactionState::ReferenceUpdated { .. }
                            ) && !matches!(outcome, StepOutcome::Completed)
                        })
                })
                .unwrap_or(false);
            let journal_gone =
                !retain_journal && state.commit_journals.remove(job.run_id, attempt_id).is_ok();
            if !journal_gone {
                cleanup = match cleanup {
                    crate::workflows::run::AttemptCleanupRecord::Orphaned {
                        sandbox,
                        workspace,
                        ..
                    } => crate::workflows::run::AttemptCleanupRecord::Orphaned {
                        sandbox,
                        workspace,
                        journal: true,
                    },
                    _ => crate::workflows::run::AttemptCleanupRecord::Orphaned {
                        sandbox: false,
                        workspace: false,
                        journal: true,
                    },
                };
                outcome = StepOutcome::Failed {
                    category: FailureCategory::Cleanup,
                    error: Some("Power Plant could not clean up the commit journal.".to_owned()),
                };
            }
        }
        if !atomic_agent_publication
            && persist_cleanup(&state, &job.run_id, attempt_id, cleanup).is_err()
        {
            fail_operational(&state, &job);
            return;
        }
        if atomic_agent_publication && published {
            if let Some(run) = state.workflow_runs.get(&job.run_id)
                && run.is_terminal()
            {
                settle_terminal_job(&state, &job, &run);
                return;
            }
            continue;
        }
        if let Err(error) = finalise_attempt(
            &state,
            &job,
            &step,
            attempt_id,
            &inputs,
            captured.as_ref(),
            &outcome,
            published,
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
                    settle_terminal_job(&state, &job, &run);
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

fn settle_terminal_job(state: &AppState, job: &WorkflowJob, run: &crate::workflows::WorkflowRun) {
    match &run.state {
        crate::workflows::run::RunState::Escalated { reason, .. } => {
            let message = match reason {
                crate::workflows::run::EscalationReason::Blocked => {
                    "The review blocked this workflow run."
                }
                crate::workflows::run::EscalationReason::AttemptLimit => {
                    "The review attempt limit escalated this workflow run."
                }
            };
            settle_job(state, job, JobStatus::Failed, Some(message));
        }
        _ => settle_job(state, job, JobStatus::Completed, None),
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
                    journal: false,
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
    let commit_attempt = capabilities.source_location
        == crate::workflows::capabilities::PrimarySourceLocation::UserProject;
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
    let outcome = if commit_attempt {
        run_commit_transaction(
            state,
            job,
            step,
            attempt_id,
            inputs,
            &user_project,
            &candidate_input,
            &sandbox,
        )
        .await
    } else {
        dispatch_step(state, job, step, &sandbox, drafts.clone()).await
    };
    job.job.set_step_label("Capturing outputs".to_owned());
    let stopped = sandbox.stop().await.is_ok();
    let captured = if stopped && commit_attempt {
        let first = crate::workflows::artefacts::CandidateCapture::capture_host(
            &user_project,
            &state.workflow_artefacts,
        )
        .ok();
        let second = crate::workflows::artefacts::CandidateCapture::capture_host(
            &user_project,
            &state.workflow_artefacts,
        )
        .ok();
        match (first, second) {
            (Some(first), Some(second)) if first == second => Some(first),
            _ => None,
        }
    } else if stopped {
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
    if commit_attempt && matches!(outcome, StepOutcome::Completed) {
        let commit = state.workflow_runs.get(&job.run_id).and_then(|run| {
            run.attempts
                .iter()
                .find(|attempt| attempt.id == attempt_id)
                .and_then(|attempt| attempt.commit_transaction.as_ref())
                .and_then(|transaction| transaction.expected_commit.clone())
        });
        let verified = captured
            .as_ref()
            .zip(commit.as_ref())
            .is_some_and(|(captured, commit)| {
                captured.candidate_hash == candidate_input.artefact.candidate_hash
                    && captured
                        .repository
                        .head
                        .as_ref()
                        .map(|head| head.0.as_str())
                        == Some(commit.as_str())
            });
        let recorded = verified
            && commit.as_ref().is_some_and(|commit| {
                let transaction_result = state.workflow_runs.mutate(&job.run_id, |run| {
                    let mut transaction = run
                        .attempts
                        .iter()
                        .find(|attempt| attempt.id == attempt_id)
                        .and_then(|attempt| attempt.commit_transaction.clone())
                        .ok_or(crate::workflows::run::TransitionError::Invalid)?;
                    transaction.state =
                        crate::workflows::commit::CommitTransactionState::Verified {
                            commit: commit.clone(),
                        };
                    run.record_commit_transaction(attempt_id, transaction)
                });
                transaction_result.is_ok()
                    && state
                        .workflow_runs
                        .mutate(&job.run_id, |run| {
                            run.record_commit_result(
                                attempt_id,
                                crate::workflows::commit::CommitResult {
                                    commit: commit.clone(),
                                },
                            )
                        })
                        .is_ok()
            });
        if !recorded {
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
                    journal: true,
                }
            };
            return IsolatedRun::Finished {
                outcome: StepOutcome::Failed {
                    category: FailureCategory::Commit,
                    error: Some("Power Plant could not verify the Git commit.".to_owned()),
                },
                cleanup,
                drafts,
                captured,
            };
        }
    }
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
            journal: false,
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
            journal: false,
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
            journal: false,
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

#[allow(clippy::too_many_arguments)]
async fn run_commit_transaction(
    state: &AppState,
    job: &WorkflowJob,
    _step: &StepDefinition,
    attempt_id: AttemptId,
    inputs: &[super::run::AttemptArtefactInput],
    user_project: &std::path::Path,
    target: &LoadedCandidate,
    sandbox: &GuestSandbox,
) -> StepOutcome {
    let result = execute_commit_transaction(
        state,
        job,
        attempt_id,
        inputs,
        user_project,
        target,
        sandbox,
    )
    .await;
    let temporary_index = user_project
        .join(".git")
        .join(format!("powerplant-commit-index-{}", attempt_id.as_hex()));
    match std::fs::remove_file(temporary_index) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) if result.is_ok() => {
            return StepOutcome::Failed {
                category: FailureCategory::Cleanup,
                error: Some("Power Plant could not clean up the temporary Git index.".to_owned()),
            };
        }
        Err(_) => {}
    }
    match result {
        Ok(()) => StepOutcome::Completed,
        Err(_) if job.job.cancel_requested() => StepOutcome::Cancelled,
        Err(error) => StepOutcome::Failed {
            category: error.category(),
            error: Some(error.message().to_owned()),
        },
    }
}

async fn execute_commit_transaction(
    state: &AppState,
    job: &WorkflowJob,
    attempt_id: AttemptId,
    inputs: &[super::run::AttemptArtefactInput],
    user_project: &std::path::Path,
    target: &LoadedCandidate,
    sandbox: &GuestSandbox,
) -> Result<(), crate::workflows::commit::CommitError> {
    use crate::workflows::commit::{CommitError, CommitTransaction, CommitTransactionState};

    let run = state
        .workflow_runs
        .get(&job.run_id)
        .ok_or(CommitError::Operational)?;
    let crate::workflows::RunSource::Captured { source } = &run.source else {
        return Err(CommitError::Operational);
    };
    let initial_record = run
        .artefact(&source.initial.id)
        .ok_or(CommitError::Operational)?;
    let initial_bytes = state
        .workflow_artefacts
        .get(&initial_record.object_hash)
        .map_err(|_| CommitError::Operational)?;
    let initial =
        crate::workflows::artefacts::candidate::CandidateRevisionArtefact::from_manifest_bytes(
            &initial_bytes,
        )
        .ok_or(CommitError::Operational)?;
    let live = crate::workflows::artefacts::CandidateCapture::capture_host(
        user_project,
        &state.workflow_artefacts,
    )
    .map_err(|_| CommitError::Preflight)?;
    if live != initial
        || target.artefact.repository != initial.repository
        || target.artefact.git_admin != initial.git_admin
    {
        return Err(CommitError::Preflight);
    }
    let expected_reference = current_reference(user_project)?;
    let candidate = inputs
        .iter()
        .find(|input| {
            input.artefact.kind == crate::workflows::definition::ArtefactKind::CandidateRevision
        })
        .map(|input| input.artefact.clone())
        .ok_or(CommitError::Assurance)?;
    let reviews: Vec<_> = inputs
        .iter()
        .filter(|input| {
            input.artefact.kind == crate::workflows::definition::ArtefactKind::ReviewReport
        })
        .map(|input| input.artefact.clone())
        .collect();
    if reviews.is_empty() {
        return Err(CommitError::Assurance);
    }
    let approval = inputs
        .iter()
        .find(|input| {
            input.artefact.kind == crate::workflows::definition::ArtefactKind::HumanDecision
        })
        .map(|input| input.artefact.clone());
    let timestamp = crate::workflows::commit::utc_timestamp(now_ms());
    let mut transaction = CommitTransaction {
        state: CommitTransactionState::Prepared,
        candidate,
        reviews,
        approval,
        expected_reference,
        old_object: initial
            .repository
            .head
            .as_ref()
            .map(|object| object.0.clone()),
        target_tree: None,
        expected_commit: None,
        timestamp: timestamp.clone(),
    };
    let journal = state
        .commit_journals
        .create(job.run_id, attempt_id)
        .map_err(|_| CommitError::Operational)?;
    let live_index = user_project.join(".git/index");
    let original_index = match std::fs::read(&live_index) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return Err(CommitError::Preflight),
    };
    journal
        .write_index_backup("original.index", &original_index)
        .map_err(|_| CommitError::Operational)?;
    journal.flush().map_err(|_| CommitError::Operational)?;
    persist_transaction(state, job.run_id, attempt_id, transaction.clone())?;

    let index_guest = crate::workflows::commit::temporary_index_guest(attempt_id);
    let index_host = user_project
        .join(".git")
        .join(format!("powerplant-commit-index-{}", attempt_id.as_hex()));
    if index_host.exists() {
        return Err(CommitError::Preflight);
    }
    run_git_capture(
        sandbox,
        &job.job,
        crate::workflows::commit::read_tree_empty_command(&index_guest, &timestamp),
        true,
    )
    .await?;
    let mut index_info = Vec::new();
    for entry in &target.artefact.entries {
        let object = match &entry.kind {
            crate::workflows::artefacts::candidate::CandidateEntryKind::Regular {
                blob, ..
            }
            | crate::workflows::artefacts::candidate::CandidateEntryKind::Symlink {
                blob, ..
            } => {
                let bytes = state
                    .workflow_artefacts
                    .get(blob)
                    .map_err(|_| CommitError::Operational)?;
                let output = run_git_capture(
                    sandbox,
                    &job.job,
                    crate::workflows::commit::hash_object_command(bytes, &timestamp),
                    true,
                )
                .await?;
                crate::workflows::commit::parse_object_id(
                    &output,
                    target.artefact.repository.object_format,
                )?
                .0
            }
            crate::workflows::artefacts::candidate::CandidateEntryKind::Gitlink { commit } => {
                crate::workflows::commit::parse_object_id(
                    &commit.0,
                    target.artefact.repository.object_format,
                )?
                .0
            }
        };
        index_info.extend(crate::workflows::commit::index_info_record(entry, &object)?);
    }
    run_git_capture(
        sandbox,
        &job.job,
        crate::workflows::commit::index_info_command(index_info, &index_guest, &timestamp),
        true,
    )
    .await?;
    let tree = run_git_capture(
        sandbox,
        &job.job,
        crate::workflows::commit::write_tree_command(&index_guest, &timestamp),
        true,
    )
    .await?;
    let tree =
        crate::workflows::commit::parse_object_id(&tree, target.artefact.repository.object_format)?
            .0;
    if let Some(old) = transaction.old_object.as_deref() {
        let old_tree = git_host_text(user_project, &["rev-parse", &format!("{old}^{{tree}}")])?;
        if old_tree == tree {
            return Err(CommitError::Preflight);
        }
    }
    transaction.target_tree = Some(tree.clone());
    persist_transaction(state, job.run_id, attempt_id, transaction.clone())?;
    let commit = run_git_capture(
        sandbox,
        &job.job,
        crate::workflows::commit::commit_tree_command(
            &tree,
            transaction.old_object.as_deref(),
            &timestamp,
        ),
        true,
    )
    .await?;
    let commit = crate::workflows::commit::parse_object_id(
        &commit,
        target.artefact.repository.object_format,
    )?
    .0;
    let target_index = std::fs::read(&index_host).map_err(|_| CommitError::Command)?;
    journal
        .write_index_backup("target.index", &target_index)
        .map_err(|_| CommitError::Operational)?;
    journal.flush().map_err(|_| CommitError::Operational)?;
    std::fs::remove_file(&index_host).map_err(|_| CommitError::Operational)?;
    transaction.expected_commit = Some(commit.clone());
    persist_transaction(state, job.run_id, attempt_id, transaction.clone())?;
    if job.job.cancel_requested() {
        return Err(CommitError::Operational);
    }

    job.job.set_step_label("Apply candidate".to_owned());
    crate::workflows::artefacts::CandidateApply::apply(
        user_project,
        &initial,
        &target.artefact,
        target.artefact_hash,
        &state.workflow_artefacts,
    )
    .map_err(map_apply_error)?;
    transaction.state = CommitTransactionState::WorktreeApplied;
    persist_transaction(state, job.run_id, attempt_id, transaction.clone())?;

    let old_guard = transaction.old_object.as_deref().unwrap_or({
        match target.artefact.repository.object_format {
            crate::workflows::artefacts::candidate::GitObjectFormat::Sha1 => {
                "0000000000000000000000000000000000000000"
            }
            crate::workflows::artefacts::candidate::GitObjectFormat::Sha256 => {
                "0000000000000000000000000000000000000000000000000000000000000000"
            }
        }
    });
    if run_git_capture(
        sandbox,
        &job.job,
        crate::workflows::commit::update_ref_command(
            &transaction.expected_reference,
            &commit,
            Some(old_guard),
            &timestamp,
        ),
        false,
    )
    .await
    .is_err()
    {
        restore_before_reference(state, user_project, &initial, &target.artefact, &journal)?;
        return Err(CommitError::Command);
    }
    transaction.state = CommitTransactionState::ReferenceUpdated {
        commit: commit.clone(),
    };
    persist_transaction(state, job.run_id, attempt_id, transaction.clone())?;
    crate::storage::write_private(&live_index, &target_index)
        .map_err(|_| CommitError::Operational)?;
    Ok(())
}

fn persist_transaction(
    state: &AppState,
    run_id: RunId,
    attempt_id: AttemptId,
    transaction: crate::workflows::commit::CommitTransaction,
) -> Result<(), crate::workflows::commit::CommitError> {
    state
        .workflow_runs
        .mutate(&run_id, |run| {
            run.record_commit_transaction(attempt_id, transaction)
        })
        .map(|_| ())
        .map_err(|_| crate::workflows::commit::CommitError::Operational)
}

fn map_apply_error(
    error: crate::workflows::artefacts::apply::ApplyError,
) -> crate::workflows::commit::CommitError {
    match error {
        crate::workflows::artefacts::apply::ApplyError::Drift
        | crate::workflows::artefacts::apply::ApplyError::Conflict => {
            crate::workflows::commit::CommitError::Preflight
        }
        crate::workflows::artefacts::apply::ApplyError::Integrity => {
            crate::workflows::commit::CommitError::Operational
        }
        _ => crate::workflows::commit::CommitError::Apply,
    }
}

fn restore_before_reference(
    state: &AppState,
    user_project: &std::path::Path,
    initial: &crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
    target: &crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
    journal: &crate::workflows::commit::CommitJournal,
) -> Result<(), crate::workflows::commit::CommitError> {
    crate::workflows::artefacts::CandidateApply::rollback(
        user_project,
        initial,
        target,
        &state.workflow_artefacts,
    )
    .map_err(|_| crate::workflows::commit::CommitError::Operational)?;
    let original = journal
        .read_index_backup("original.index")
        .map_err(|_| crate::workflows::commit::CommitError::Operational)?;
    let index = user_project.join(".git/index");
    if original.is_empty() {
        match std::fs::remove_file(index) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(crate::workflows::commit::CommitError::Operational),
        }
    } else {
        crate::storage::write_private(&index, &original)
            .map_err(|_| crate::workflows::commit::CommitError::Operational)?;
    }
    Ok(())
}

fn current_reference(
    project: &std::path::Path,
) -> Result<String, crate::workflows::commit::CommitError> {
    let output = std::process::Command::new("git")
        .current_dir(project)
        .args(["symbolic-ref", "--quiet", "HEAD"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .map_err(|_| crate::workflows::commit::CommitError::Preflight)?;
    if output.status.success() {
        let reference = String::from_utf8(output.stdout)
            .map_err(|_| crate::workflows::commit::CommitError::Preflight)?;
        let reference = reference.trim();
        if reference.starts_with("refs/heads/")
            && !reference.contains("..")
            && !reference.contains(['\\', ' ', '~', '^', ':', '?', '*', '['])
        {
            return Ok(reference.to_owned());
        }
        return Err(crate::workflows::commit::CommitError::Preflight);
    }
    Ok("HEAD".to_owned())
}

fn git_host_text(
    project: &std::path::Path,
    args: &[&str],
) -> Result<String, crate::workflows::commit::CommitError> {
    let output = std::process::Command::new("git")
        .current_dir(project)
        .args(["--no-optional-locks", "-c", "core.hooksPath=/dev/null"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|_| crate::workflows::commit::CommitError::Preflight)?;
    if !output.status.success() {
        return Err(crate::workflows::commit::CommitError::Preflight);
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|_| crate::workflows::commit::CommitError::Preflight)
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
        StepAction::SystemCommand(action) => match action.command {
            SystemCommandId::CommitCandidate => StepOutcome::Failed {
                category: FailureCategory::Definition,
                error: Some(OPERATIONAL_STORE_ERROR.to_owned()),
            },
            SystemCommandId::RepositoryStatus => {
                run_system_exec(sandbox, &job.job, guest_command(action.command)).await
            }
        },
        StepAction::HumanGate(_) => StepOutcome::Failed {
            category: FailureCategory::Definition,
            error: Some(OPERATIONAL_STORE_ERROR.to_owned()),
        },
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
    let policy = match intersect_authority(
        action.candidate_authority,
        &action.authority,
        &job.host_policy,
    ) {
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
    let context = state.workflow_runs.get(&job.run_id).and_then(|run| {
        let step = run.pinned.definition.step(&run.attempts.last()?.step)?;
        let inputs = run.attempts.last()?.inputs.clone();
        let verified = crate::workflows::input_context::verify_inputs(
            &run,
            step,
            &inputs,
            &state.workflow_artefacts,
        )
        .ok()?;
        Some(crate::workflows::input_context::format_agent_context(
            &verified,
            step.writes_primary_source(),
        ))
    });
    let composed = crate::agents::compose_role(
        &role.name,
        &role.expertise,
        &instructions,
        &action.authority.tools,
        &policy,
    );
    let preamble = match context {
        Some(context) if !context.is_empty() => format!("{composed}\n\n{context}"),
        _ => composed,
    };
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
    if ended.outcome == AgentOutcome::Completed {
        *job.eligible_reply
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = ended.reply.clone();
    }
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

async fn run_git_capture(
    sandbox: &GuestSandbox,
    job: &Job,
    exec: GuestExec,
    cancellable: bool,
) -> Result<String, crate::workflows::commit::CommitError> {
    let mut session = sandbox
        .exec_cmd(exec)
        .await
        .map_err(|_| crate::workflows::commit::CommitError::Command)?;
    let deadline = Instant::now() + COMMAND_DEADLINE;
    let mut output = String::new();
    let mut exit = None;
    loop {
        if cancellable && job.cancel_requested() {
            session.kill().await;
            session.close().await;
            return Err(crate::workflows::commit::CommitError::Operational);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            session.kill().await;
            session.close().await;
            return Err(crate::workflows::commit::CommitError::Command);
        }
        let event = if cancellable {
            tokio::select! {
                _ = job.cancelled() => {
                    session.kill().await;
                    session.close().await;
                    return Err(crate::workflows::commit::CommitError::Operational);
                }
                _ = tokio::time::sleep(remaining) => {
                    session.kill().await;
                    session.close().await;
                    return Err(crate::workflows::commit::CommitError::Command);
                }
                event = session.recv() => event,
            }
        } else {
            tokio::select! {
                _ = tokio::time::sleep(remaining) => {
                    session.kill().await;
                    session.close().await;
                    return Err(crate::workflows::commit::CommitError::Command);
                }
                event = session.recv() => event,
            }
        };
        let Some(event) = event else {
            break;
        };
        match event {
            CommandEvent::Output(text) => {
                if output.len().saturating_add(text.len()) > COMMAND_OUTPUT_LIMIT {
                    session.kill().await;
                    session.close().await;
                    return Err(crate::workflows::commit::CommitError::Command);
                }
                output.push_str(&text);
            }
            CommandEvent::Exited(code) => exit = Some(code),
            CommandEvent::Failed => {
                session.close().await;
                return Err(crate::workflows::commit::CommitError::Command);
            }
        }
    }
    session.close().await;
    if exit != Some(0) {
        return Err(crate::workflows::commit::CommitError::Command);
    }
    Ok(output)
}

async fn run_system_exec(sandbox: &GuestSandbox, job: &Job, exec: GuestExec) -> StepOutcome {
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
        SystemCommandId::CommitCandidate => {
            GuestExec::command("git", Vec::new()).in_dir(GUEST_PROJECT)
        }
    }
}

fn intersect_authority(
    candidate_authority: CandidateAuthority,
    authority: &AgentAuthority,
    host: &DirectoryPolicy,
) -> Result<DirectoryPolicy, ()> {
    let primary = host
        .grants()
        .iter()
        .find(|grant| grant.alias == host.primary_alias())
        .ok_or(())?;
    if candidate_authority.access().is_writable() && !primary.access.is_writable() {
        return Err(());
    }
    let mut grants = vec![PolicyGrant {
        alias: primary.alias.clone(),
        guest_path: primary.guest_path.clone(),
        host_path: primary.host_path.clone(),
        access: candidate_authority.access(),
    }];
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
    Ok(DirectoryPolicy::from_grants(
        grants,
        host.primary_alias().to_owned(),
    ))
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
    let access = capabilities.guest_access(&job.connection);
    let spec = if capabilities.source_location
        == crate::workflows::capabilities::PrimarySourceLocation::UserProject
    {
        commit_attempt_spec(capabilities, &user_project.host_path, access)?
    } else {
        let spec = attempt_spec(
            capabilities,
            workspace,
            &user_project.host_path,
            &job.host_policy,
            access,
        )?;
        crate::sandbox::reject_user_project_write(&spec, &user_project.host_path)
            .map_err(|error| error.message())?;
        spec
    };
    if job.job.cancel_requested() {
        return Err("The task was cancelled.");
    }
    sandbox
        .start_from_snapshot(&path, environment.snapshot.snapshot_digest.as_str(), spec)
        .await
        .map_err(|error| error.message())?;
    job.job.set_step_label(active_step_label(&run, step));
    Ok(())
}

fn active_step_label(run: &crate::workflows::WorkflowRun, step: &StepDefinition) -> String {
    let steps = run.pinned.definition.steps();
    let position = steps
        .iter()
        .position(|item| item.key == step.key)
        .map(|index| format!("{} of {}", index + 1, steps.len()))
        .unwrap_or_default();
    let action = match &step.action {
        StepAction::SystemCommand(action) if action.command == SystemCommandId::CommitCandidate => {
            "Create commit"
        }
        StepAction::Agent(action)
            if action
                .required_outputs
                .iter()
                .any(|output| output.kind == crate::workflows::definition::OutputKind::Plan)
                && !step.writes_primary_source() =>
        {
            "Plan"
        }
        StepAction::Agent(action)
            if action.required_outputs.iter().any(|output| {
                output.kind == crate::workflows::definition::OutputKind::ReviewReport
            }) =>
        {
            "Review"
        }
        StepAction::Agent(_) if step.writes_primary_source() => "Implement",
        _ => step.name.as_str(),
    };
    let action = if let Some(policy) = &step.review {
        let phase = run.pinned.definition.review_phase(&step.key).unwrap_or(1);
        let ordinal = run
            .attempts
            .iter()
            .filter(|attempt| attempt.step == step.key)
            .count()
            + 1;
        format!(
            "{action} phase {phase} · attempt {ordinal} of {}",
            policy.attempt_limit
        )
    } else {
        action.to_owned()
    };
    if position.is_empty() {
        action
    } else {
        format!("{action} · {position}")
    }
}

fn commit_attempt_spec(
    capabilities: &crate::workflows::capabilities::AttemptCapabilities,
    user_project: &std::path::Path,
    access: crate::sandbox::GuestAccess,
) -> Result<crate::sandbox::SandboxSpec, &'static str> {
    let Some(primary) = capabilities.primary() else {
        return Err("A sandbox-backed step needs a primary source.");
    };
    let git = user_project.join(".git");
    if !git.is_dir() {
        return Err("The project is not a supported Git worktree.");
    }
    Ok(crate::sandbox::SandboxSpec {
        mounts: vec![
            crate::sandbox::MountSpec {
                guest: primary.guest_path.clone(),
                host: user_project.to_path_buf(),
                read_only: false,
            },
            crate::sandbox::MountSpec {
                guest: format!("{}/.git", primary.guest_path),
                host: git,
                read_only: false,
            },
        ],
        workdir: primary.guest_path.clone(),
        access,
    })
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

fn confirm_run_authority(
    state: &AppState,
    job: &WorkflowJob,
) -> Result<std::path::PathBuf, String> {
    let Some(project) = state.projects.get(&job.project_id) else {
        return Err("That project is not in the catalogue.".to_owned());
    };
    let Some(agent) = state.agents.get(&job.agent_id) else {
        return Err("That agent is not in the catalogue.".to_owned());
    };
    if agent.revision != job.agent_revision {
        return Err("The agent configuration changed. Try again.".to_owned());
    }
    let Some(grant) = crate::projects::exact_grant(&agent, &project) else {
        return Err("This agent no longer has access to that project.".to_owned());
    };
    if grant.alias != job.grant_alias || grant.access != job.grant_access {
        return Err("This agent no longer has access to that project.".to_owned());
    }
    if !project.host_path_is_available() || grant.host_path != project.host_path {
        return Err("A granted directory is no longer at the saved path.".to_owned());
    }
    Ok(project.host_path)
}

async fn capture_initial_source(state: &AppState, job: &WorkflowJob) -> Result<(), String> {
    let host_path = confirm_run_authority(state, job)?;
    let captured = match crate::workflows::artefacts::CandidateCapture::capture_host(
        &host_path,
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
            crate::workflows::definition::ArtefactSource::RunCurrentCandidate => {
                let crate::workflows::RunSource::Captured { source } = &run.source else {
                    return Err("Source capture has not finished.");
                };
                source.accepted.clone()
            }
            crate::workflows::definition::ArtefactSource::StepOutput {
                step: source_step,
                output,
            } => {
                let found = if let Some(attempt) = run.attempts.iter().rev().find(|attempt| {
                    attempt.step == *source_step
                        && matches!(
                            attempt.result,
                            Some(crate::workflows::run::AttemptResult::Completed { .. })
                        )
                }) {
                    attempt
                        .outputs
                        .iter()
                        .find(|item| item.key == *output)
                        .map(|item| item.artefact.clone())
                } else {
                    run.gates
                        .iter()
                        .rev()
                        .find(|gate| gate.step == *source_step && gate.output == *output)
                        .and_then(|gate| gate.decision.clone())
                };
                let Some(found) = found else {
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
        crate::workflows::artefacts::ArtefactSummary::HumanDecision { candidate, .. } => {
            Some(*candidate)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn finalise_attempt(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
    attempt_id: AttemptId,
    inputs: &[super::run::AttemptArtefactInput],
    captured: Option<&crate::workflows::artefacts::candidate::CandidateRevisionArtefact>,
    outcome: &StepOutcome,
    published: bool,
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
        StepOutcome::Completed if published => {
            persist_outcome(state, &job.run_id, attempt_id, outcome)
        }
        StepOutcome::Completed => persist_fail(
            state,
            &job.run_id,
            Some(attempt_id),
            FailureCategory::Definition,
        ),
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

struct SuccessAttempt {
    id: AttemptId,
    complete: bool,
}

fn publish_success(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
    attempt: SuccessAttempt,
    inputs: &[super::run::AttemptArtefactInput],
    drafts: &std::sync::Mutex<crate::workflows::artefacts::output::OutputDrafts>,
    captured: Option<&crate::workflows::artefacts::candidate::CandidateRevisionArtefact>,
) -> Result<(), &'static str> {
    let SuccessAttempt {
        id: attempt_id,
        complete: complete_attempt,
    } = attempt;
    let captured = captured.ok_or("Power Plant could not capture isolated outputs.")?;
    let writes = step.writes_primary_source();
    let produces_candidate = writes
        || step.command_source_effect()
            == Some(crate::workflows::commands::CommandSourceEffect::Commit);
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
    if !produces_candidate && expected.is_some_and(|hash| hash != captured.candidate_hash) {
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
    if produces_candidate {
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
        let fixing_report_inputs;
        let provenance_inputs = if output.kind
            == crate::workflows::definition::OutputKind::ReviewReport
            && step.writes_primary_source()
        {
            let produced = accepted
                .clone()
                .ok_or("A fixing review needs a produced candidate revision.")?;
            fixing_report_inputs = std::iter::once(super::run::AttemptArtefactInput {
                key: crate::workflows::definition::InputKey::parse("candidate")
                    .map_err(|_| OPERATIONAL_STORE_ERROR)?,
                artefact: produced,
            })
            .chain(
                inputs
                    .iter()
                    .filter(|input| {
                        input.artefact.kind
                            != crate::workflows::definition::ArtefactKind::CandidateRevision
                    })
                    .cloned(),
            )
            .collect::<Vec<_>>();
            fixing_report_inputs.as_slice()
        } else {
            inputs
        };
        let record = publish_draft(
            state,
            job,
            attempt_id,
            step,
            output,
            draft,
            candidate_hash,
            provenance_inputs,
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
            run.record_attempt_outputs(attempt_id, artefacts, outputs, accepted, observed)?;
            if complete_attempt {
                run.complete_attempt(attempt_id, now_ms())?;
            }
            Ok(())
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

pub(crate) fn settle_completed_job(state: &AppState, workflow: &WorkflowJob) {
    settle_job(state, workflow, JobStatus::Completed, None);
}

fn settle_job(state: &AppState, workflow: &WorkflowJob, status: JobStatus, error: Option<&str>) {
    let eligible = workflow
        .eligible_reply
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let reply = if eligible.is_empty() {
        workflow.job.snapshot().output
    } else {
        eligible
    };
    settle_with_reply(
        state,
        &workflow.session_id,
        &workflow.agent_id,
        &workflow.job,
        status,
        error,
        &reply,
    );
}

fn settle_with_reply(
    state: &AppState,
    session_id: &SessionId,
    agent_id: &crate::agents::AgentId,
    job: &Job,
    status: JobStatus,
    error: Option<&str>,
    reply: &str,
) {
    let reply = crate::slices::bound_reply(reply).to_owned();
    match status {
        JobStatus::Completed => {
            let _ = state
                .sessions
                .finish_turn(session_id, agent_id, &job.id(), reply);
        }
        _ => {
            let _ = state
                .sessions
                .fail_turn(session_id, agent_id, &job.id(), reply);
        }
    }
    let _ = job.finish(status, error);
}

fn recovery_project_path(
    state: &AppState,
    run: &crate::workflows::WorkflowRun,
) -> Result<std::path::PathBuf, &'static str> {
    let error = "Power Plant could not recover a commit transaction.";
    let Some(project) = state.projects.get(&run.project_id) else {
        return Err(error);
    };
    let Some(agent) = state.agents.get(&run.agent_id) else {
        return Err(error);
    };
    let Some(grant) = crate::projects::exact_grant(&agent, &project) else {
        return Err(error);
    };
    if !grant.access.is_writable()
        || !project.host_path_is_available()
        || crate::workflows::artefacts::inspect_supported_worktree(&project.host_path).is_err()
    {
        return Err(error);
    }
    Ok(project.host_path)
}

pub(crate) fn recover_commit_transactions(state: &AppState) -> Result<(), &'static str> {
    for run in state.workflow_runs.active_runs() {
        let Some(attempt_id) = run.active_attempt() else {
            continue;
        };
        let Some(attempt) = run.attempts.iter().find(|attempt| attempt.id == attempt_id) else {
            return Err("Power Plant could not recover a commit transaction.");
        };
        let Some(transaction) = attempt.commit_transaction.clone() else {
            continue;
        };
        let project = recovery_project_path(state, &run)?;
        if current_reference(&project).ok().as_deref() != Some(&transaction.expected_reference) {
            return Err("Power Plant could not recover a commit transaction.");
        }
        let initial_ref = match &run.source {
            crate::workflows::RunSource::Captured { source } => &source.initial,
            crate::workflows::RunSource::Pending => {
                return Err("Power Plant could not recover a commit transaction.");
            }
        };
        let initial = load_candidate_reference(state, &run, initial_ref)?;
        let target = load_candidate_reference(state, &run, &transaction.candidate)?;
        if target.repository != initial.repository || target.git_admin != initial.git_admin {
            return Err("Power Plant could not recover a commit transaction.");
        }
        let head = current_head(&project)?;
        let old = transaction.old_object.as_deref();
        let expected = transaction.expected_commit.as_deref();
        if head.as_deref() == old {
            let live = crate::workflows::artefacts::CandidateCapture::capture_host(
                &project,
                &state.workflow_artefacts,
            )
            .map_err(|_| "Power Plant could not recover a commit transaction.")?;
            if live.candidate_hash == target.candidate_hash
                && live.repository == initial.repository
                && live.git_admin == initial.git_admin
            {
                let journal = state
                    .commit_journals
                    .load(run.id, attempt_id)
                    .map_err(|_| "Power Plant could not recover a commit transaction.")?;
                restore_before_reference(state, &project, &initial, &target, &journal)
                    .map_err(|_| "Power Plant could not recover a commit transaction.")?;
            } else if live != initial {
                return Err("Power Plant could not recover a commit transaction.");
            }
            remove_commit_journal(state, run.id, attempt_id)?;
            continue;
        }
        if head.as_deref() != expected || expected.is_none() {
            return Err("Power Plant could not recover a commit transaction.");
        }
        let journal = state
            .commit_journals
            .load(run.id, attempt_id)
            .map_err(|_| "Power Plant could not recover a commit transaction.")?;
        let target_index = journal
            .read_index_backup("target.index")
            .map_err(|_| "Power Plant could not recover a commit transaction.")?;
        crate::storage::write_private(&project.join(".git/index"), &target_index)
            .map_err(|_| "Power Plant could not recover a commit transaction.")?;
        let captured = crate::workflows::artefacts::CandidateCapture::capture_host(
            &project,
            &state.workflow_artefacts,
        )
        .map_err(|_| "Power Plant could not recover a commit transaction.")?;
        let commit = expected.expect("checked").to_owned();
        if captured.candidate_hash != target.candidate_hash
            || captured
                .repository
                .head
                .as_ref()
                .map(|head| head.0.as_str())
                != Some(commit.as_str())
        {
            return Err("Power Plant could not recover a commit transaction.");
        }
        let mut verified = transaction;
        verified.state = crate::workflows::commit::CommitTransactionState::Verified {
            commit: commit.clone(),
        };
        state
            .workflow_runs
            .mutate(&run.id, |run| {
                run.record_commit_transaction(attempt_id, verified)
            })
            .map_err(|_| "Power Plant could not recover a commit transaction.")?;
        state
            .workflow_runs
            .mutate(&run.id, |run| {
                run.record_commit_result(
                    attempt_id,
                    crate::workflows::commit::CommitResult {
                        commit: commit.clone(),
                    },
                )
            })
            .map_err(|_| "Power Plant could not recover a commit transaction.")?;
        publish_recovered_commit(state, &run, attempt_id, &captured)?;
        remove_commit_journal(state, run.id, attempt_id)?;
        state
            .workflow_runs
            .mutate(&run.id, |run| {
                run.record_cleanup(
                    attempt_id,
                    crate::workflows::run::AttemptCleanupRecord::Complete,
                )
            })
            .map_err(|_| "Power Plant could not recover a commit transaction.")?;
        state
            .workflow_runs
            .mutate(&run.id, |run| run.complete_attempt(attempt_id, now_ms()))
            .map_err(|_| "Power Plant could not recover a commit transaction.")?;
    }
    Ok(())
}

fn load_candidate_reference(
    state: &AppState,
    run: &crate::workflows::WorkflowRun,
    reference: &crate::workflows::artefacts::ArtefactReference,
) -> Result<crate::workflows::artefacts::candidate::CandidateRevisionArtefact, &'static str> {
    let record = run
        .artefact(&reference.id)
        .filter(|record| record.artefact_hash == reference.artefact_hash)
        .ok_or("Power Plant could not recover a commit transaction.")?;
    let bytes = state
        .workflow_artefacts
        .get(&record.object_hash)
        .map_err(|_| "Power Plant could not recover a commit transaction.")?;
    crate::workflows::artefacts::candidate::CandidateRevisionArtefact::from_manifest_bytes(&bytes)
        .ok_or("Power Plant could not recover a commit transaction.")
}

fn current_head(project: &std::path::Path) -> Result<Option<String>, &'static str> {
    let output = std::process::Command::new("git")
        .current_dir(project)
        .args(["rev-parse", "--verify", "HEAD"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .map_err(|_| "Power Plant could not recover a commit transaction.")?;
    if !output.status.success() {
        return Ok(None);
    }
    String::from_utf8(output.stdout)
        .map(|head| Some(head.trim().to_owned()))
        .map_err(|_| "Power Plant could not recover a commit transaction.")
}

fn remove_commit_journal(
    state: &AppState,
    run_id: RunId,
    attempt_id: AttemptId,
) -> Result<(), &'static str> {
    state
        .commit_journals
        .remove(run_id, attempt_id)
        .map_err(|_| "Power Plant could not recover a commit transaction.")
}

fn publish_recovered_commit(
    state: &AppState,
    run: &crate::workflows::WorkflowRun,
    attempt_id: AttemptId,
    captured: &crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
) -> Result<(), &'static str> {
    let attempt = run
        .attempts
        .iter()
        .find(|attempt| attempt.id == attempt_id)
        .ok_or("Power Plant could not recover a commit transaction.")?;
    let step = run
        .pinned
        .definition
        .step(&attempt.step)
        .ok_or("Power Plant could not recover a commit transaction.")?;
    let output = step
        .required_outputs()
        .iter()
        .find(|output| output.kind == crate::workflows::definition::OutputKind::CandidateRevision)
        .ok_or("Power Plant could not recover a commit transaction.")?;
    if !attempt.outputs.is_empty() {
        let existing = attempt
            .outputs
            .iter()
            .find(|existing| existing.key == output.key)
            .ok_or("Power Plant could not recover a commit transaction.")?;
        let stored = load_candidate_reference(state, run, &existing.artefact)?;
        if attempt.outputs.len() == 1 && stored == *captured {
            return Ok(());
        }
        return Err("Power Plant could not recover a commit transaction.");
    }
    let bytes = captured
        .manifest_bytes()
        .map_err(|_| "Power Plant could not recover a commit transaction.")?;
    let object = state
        .workflow_artefacts
        .publish(&bytes)
        .map_err(|_| "Power Plant could not recover a commit transaction.")?;
    let artefact_hash = crate::workflows::artefacts::artefact_hash_for(
        crate::workflows::definition::ArtefactKind::CandidateRevision,
        captured.format_version,
        &bytes,
    );
    let id = crate::workflows::ArtefactId::generate()
        .map_err(|_| "Power Plant could not recover a commit transaction.")?;
    let record = crate::workflows::artefacts::ArtefactRecord {
        id,
        kind: crate::workflows::definition::ArtefactKind::CandidateRevision,
        artefact_hash,
        object_hash: object,
        payload_bytes: bytes.len() as u64,
        created_at_ms: now_ms(),
        provenance: crate::workflows::artefacts::ArtefactProvenance {
            run_id: run.id,
            producer: crate::workflows::artefacts::ArtefactProducer::StepAttempt {
                attempt_id,
                step: step.key.clone(),
                output: Some(output.key.clone()),
                disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
            },
            inputs: attempt
                .inputs
                .iter()
                .map(|input| input.artefact.clone())
                .collect(),
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
    let reference = crate::workflows::artefacts::ArtefactReference {
        id,
        kind: record.kind,
        artefact_hash,
    };
    state
        .workflow_runs
        .mutate(&run.id, |run| {
            run.record_attempt_outputs(
                attempt_id,
                vec![record],
                vec![crate::workflows::run::AttemptArtefactOutput {
                    key: output.key.clone(),
                    artefact: reference.clone(),
                }],
                Some(reference.clone()),
                crate::workflows::run::ObservedCandidate::Exact {
                    artefact: reference,
                },
            )
        })
        .map(|_| ())
        .map_err(|_| "Power Plant could not recover a commit transaction.")
}

#[cfg(test)]
mod tests;
