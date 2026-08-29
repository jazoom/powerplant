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
    pub(crate) sandbox: Arc<GuestSandbox>,
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
        let Some(step_key) = run.ready_step().cloned() else {
            fail_operational(&state, &job);
            return;
        };
        let Some(step) = run.pinned.definition.step(&step_key).cloned() else {
            fail_operational(&state, &job);
            return;
        };
        job.job.set_step_label(step.name.clone());
        let attempt_id = match AttemptId::generate() {
            Ok(id) => id,
            Err(_) => {
                fail_operational(&state, &job);
                return;
            }
        };
        if persist_start(&state, &job.run_id, attempt_id).is_err() {
            fail_operational(&state, &job);
            return;
        }
        let outcome = dispatch_step(&state, &job, &step).await;
        if persist_outcome(&state, &job.run_id, attempt_id, &outcome).is_err() {
            fail_operational(&state, &job);
            return;
        }
        match outcome {
            StepOutcome::Completed { .. } => {
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
    Completed {
        outputs: Vec<String>,
    },
    Failed {
        category: FailureCategory,
        error: Option<String>,
    },
    Cancelled,
}

async fn dispatch_step(state: &AppState, job: &WorkflowJob, step: &StepDefinition) -> StepOutcome {
    match &step.action {
        StepAction::Agent(action) => run_agent_step(state, job, action).await,
        StepAction::SystemCommand(action) => {
            run_system_command_step(&job.sandbox, &job.job, action.command).await
        }
    }
}

async fn run_agent_step(state: &AppState, job: &WorkflowJob, action: &AgentStep) -> StepOutcome {
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
        tools: crate::tools::definitions(&action.authority.tools),
        tool_ids: action.authority.tools.clone(),
        policy,
        connection: job.connection.clone(),
        sandbox: job.sandbox.clone(),
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
        AgentOutcome::Completed => StepOutcome::Completed {
            outputs: action
                .required_outputs
                .iter()
                .map(|output| output.key.as_str().to_owned())
                .collect(),
        },
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
        Some(0) => StepOutcome::Completed {
            outputs: Vec::new(),
        },
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

fn persist_start(
    state: &AppState,
    run_id: &RunId,
    attempt_id: AttemptId,
) -> Result<(), StoreError> {
    let at_ms = now_ms();
    state
        .workflow_runs
        .mutate(run_id, |run| run.start_attempt(attempt_id, at_ms))
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
        StepOutcome::Completed { outputs } => state
            .workflow_runs
            .mutate(run_id, |run| {
                run.complete_attempt(attempt_id, outputs.clone(), at_ms)
            })
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
