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
    pub(crate) access: crate::sandbox::GuestAccess,
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
                let _ = job.sandbox.remove().await;
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
                let _ = job.sandbox.remove().await;
                return;
            }
        };
        if let Err(error) = reject_stale_assurance(&state, &run, &inputs) {
            if persist_initial_fail(&state, &job.run_id).is_err() {
                fail_operational(&state, &job);
            } else {
                settle_job(&state, &job, JobStatus::Failed, Some(error));
            }
            let _ = job.sandbox.remove().await;
            return;
        }
        let attempt_id = match AttemptId::generate() {
            Ok(id) => id,
            Err(_) => {
                fail_operational(&state, &job);
                return;
            }
        };
        if persist_start(&state, &job.run_id, attempt_id, inputs.clone()).is_err() {
            fail_operational(&state, &job);
            return;
        }
        if let Err(error) = ensure_run_sandbox(&state, &job, &step).await {
            if job.job.cancel_requested() {
                if persist_cancel(&state, &job.run_id).is_err() {
                    fail_operational(&state, &job);
                } else {
                    settle_job(&state, &job, JobStatus::Cancelled, None);
                }
            } else if persist_fail(
                &state,
                &job.run_id,
                Some(attempt_id),
                FailureCategory::Operational,
            )
            .is_err()
            {
                fail_operational(&state, &job);
            } else {
                settle_job(&state, &job, JobStatus::Failed, Some(error));
            }
            let _ = job.sandbox.remove().await;
            return;
        }
        if let Err(error) = capture_matches_input(&state, &job, &inputs) {
            if persist_fail(
                &state,
                &job.run_id,
                Some(attempt_id),
                FailureCategory::Operational,
            )
            .is_err()
            {
                fail_operational(&state, &job);
            } else {
                settle_job(&state, &job, JobStatus::Failed, Some(error));
            }
            let _ = job.sandbox.remove().await;
            return;
        }
        let drafts = std::sync::Arc::new(std::sync::Mutex::new(
            crate::workflows::artefacts::output::OutputDrafts::default(),
        ));
        let outcome = dispatch_step(&state, &job, &step, drafts.clone()).await;
        if let Err(error) =
            finalise_attempt(&state, &job, &step, attempt_id, &inputs, &drafts, &outcome).await
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
                    let _ = job.sandbox.remove().await;
                    settle_job(&state, &job, JobStatus::Completed, None);
                    return;
                }
            }
            StepOutcome::Failed { error, .. } => {
                let _ = job.sandbox.remove().await;
                settle_job(&state, &job, JobStatus::Failed, error.as_deref());
                return;
            }
            StepOutcome::Cancelled => {
                let _ = job.sandbox.remove().await;
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

async fn dispatch_step(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
    drafts: std::sync::Arc<std::sync::Mutex<crate::workflows::artefacts::output::OutputDrafts>>,
) -> StepOutcome {
    match &step.action {
        StepAction::Agent(action) => run_agent_step(state, job, action, drafts).await,
        StepAction::SystemCommand(action) => {
            run_system_command_step(&job.sandbox, &job.job, action.command).await
        }
    }
}

async fn run_agent_step(
    state: &AppState,
    job: &WorkflowJob,
    action: &AgentStep,
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
        sandbox: job.sandbox.clone(),
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

async fn ensure_run_sandbox(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
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
    let policy = match &step.action {
        StepAction::Agent(action) => intersect_authority(&action.authority, &job.host_policy)
            .map_err(|_| "The pinned step authority exceeds the current directory policy.")?,
        StepAction::SystemCommand(_) => job.host_policy.clone(),
    };
    let spec = protect_git_mount(
        crate::sandbox::SandboxSpec::from_policy(&policy, job.access.clone()),
        &policy,
    )?;
    if job.job.cancel_requested() {
        return Err("The task was cancelled.");
    }
    job.sandbox
        .start_from_snapshot(&path, environment.snapshot.snapshot_digest.as_str(), spec)
        .await
        .map_err(|error| error.message())?;
    job.job.set_step_label(step.name.clone());
    Ok(())
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

fn protect_git_mount(
    mut spec: crate::sandbox::SandboxSpec,
    policy: &DirectoryPolicy,
) -> Result<crate::sandbox::SandboxSpec, &'static str> {
    let Some(primary) = policy
        .grants()
        .iter()
        .find(|grant| grant.alias == policy.primary_alias())
    else {
        return Ok(spec);
    };
    if !primary.access.is_writable() {
        return Ok(spec);
    }
    let git = primary.host_path.join(".git");
    if !git.is_dir() {
        return Ok(spec);
    }
    spec.mounts.push(crate::sandbox::MountSpec {
        guest: format!("{}/.git", primary.guest_path),
        host: git,
        read_only: true,
    });
    Ok(spec)
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

fn capture_matches_input(
    state: &AppState,
    job: &WorkflowJob,
    inputs: &[super::run::AttemptArtefactInput],
) -> Result<(), &'static str> {
    let Some(expected) = inputs
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
        .and_then(|record| candidate_hash_of(&record))
    else {
        return Ok(());
    };
    let captured = capture_project(state, job)?;
    if captured.candidate_hash != expected {
        return Err("The project changed before that step.");
    }
    Ok(())
}

fn capture_project(
    state: &AppState,
    job: &WorkflowJob,
) -> Result<crate::workflows::artefacts::candidate::CandidateRevisionArtefact, &'static str> {
    let host = job
        .host_policy
        .grants()
        .iter()
        .find(|grant| grant.alias == job.host_policy.primary_alias())
        .ok_or("Choose a project directory.")?;
    match crate::workflows::artefacts::CandidateCapture::capture_host(
        &host.host_path,
        &state.workflow_artefacts,
    ) {
        Ok(captured) => Ok(captured),
        Err(error) => Err(error.message()),
    }
}

async fn finalise_attempt(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
    attempt_id: AttemptId,
    inputs: &[super::run::AttemptArtefactInput],
    drafts: &std::sync::Mutex<crate::workflows::artefacts::output::OutputDrafts>,
    outcome: &StepOutcome,
) -> Result<(), StoreError> {
    match outcome {
        StepOutcome::Cancelled => persist_cancel(state, &job.run_id),
        StepOutcome::Failed { category, .. } => {
            if step.writes_primary_source() {
                let _ = record_observed(state, job, attempt_id, step, inputs);
            }
            persist_fail(state, &job.run_id, Some(attempt_id), *category)
        }
        StepOutcome::Completed => {
            if let Err(error) = publish_success(state, job, step, attempt_id, inputs, drafts) {
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
) -> Result<(), &'static str> {
    let captured = capture_project(state, job)?;
    let record = publish_candidate(
        state,
        job,
        &captured,
        crate::workflows::artefacts::ArtefactProducer::StepAttempt {
            attempt_id,
            step: step.key.clone(),
            output: None,
            disposition: crate::workflows::artefacts::ProductionDisposition::ObservedAfterFailure,
        },
        inputs,
    )?;
    let reference = super::artefacts::ArtefactReference {
        id: record.id,
        kind: record.kind,
        artefact_hash: record.artefact_hash,
    };
    state
        .workflow_runs
        .mutate(&job.run_id, |run| {
            run.record_attempt_outputs(
                attempt_id,
                vec![record],
                Vec::new(),
                None,
                super::run::ObservedCandidate::Exact {
                    artefact: reference,
                },
            )
        })
        .map(|_| ())
        .map_err(|_| OPERATIONAL_STORE_ERROR)
}

fn publish_success(
    state: &AppState,
    job: &WorkflowJob,
    step: &StepDefinition,
    attempt_id: AttemptId,
    inputs: &[super::run::AttemptArtefactInput],
    drafts: &std::sync::Mutex<crate::workflows::artefacts::output::OutputDrafts>,
) -> Result<(), &'static str> {
    let captured = capture_project(state, job)?;
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
            &captured,
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
) -> Result<(), StoreError> {
    let at_ms = now_ms();
    state
        .workflow_runs
        .mutate(run_id, |run| {
            run.start_attempt_with_inputs(attempt_id, inputs, at_ms)
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
    let sandbox = workflow.sandbox.clone();
    tokio::spawn(async move {
        let _ = sandbox.remove().await;
    });
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
