use std::sync::Arc;

use crate::{
    agents::{AgentLeaseCoordinator, AgentStore},
    assets::AssetPaths,
    config::{RuntimeConfig, StartupConfig},
    environments::{
        EnvironmentCatalogue, EnvironmentPreparationScheduler, EnvironmentSnapshotRepository,
    },
    models::ModelCatalogue,
    plan_login::PlanLogin,
    providers::ChatBackend,
    sandbox::SandboxFleet,
    sessions::SessionStore,
    vault::ProviderVault,
    workflows::{
        WorkflowArtefactRepository, WorkflowCatalogue, WorkflowExecution, WorkflowRunStore,
        workspace::WorkflowWorkspaces,
    },
};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) config: Arc<RuntimeConfig>,
    pub(crate) assets: Arc<AssetPaths>,
    pub(crate) sessions: Arc<SessionStore>,
    pub(crate) vault: Arc<ProviderVault>,
    pub(crate) chat: Arc<ChatBackend>,
    pub(crate) models: Arc<ModelCatalogue>,
    pub(crate) plan_login: Arc<PlanLogin>,
    pub(crate) agents: Arc<AgentStore>,
    pub(crate) sandboxes: Arc<SandboxFleet>,
    pub(crate) agent_leases: Arc<AgentLeaseCoordinator>,
    pub(crate) workflows: Arc<WorkflowCatalogue>,
    pub(crate) workflow_runs: Arc<WorkflowRunStore>,
    pub(crate) workflow_artefacts: Arc<WorkflowArtefactRepository>,
    pub(crate) workflow_execution: Arc<WorkflowExecution>,
    pub(crate) workflow_workspaces: Arc<WorkflowWorkspaces>,
    pub(crate) environments: Arc<EnvironmentCatalogue>,
    pub(crate) environment_snapshots: Arc<EnvironmentSnapshotRepository>,
    pub(crate) environment_preparations: Arc<EnvironmentPreparationScheduler>,
    #[cfg(test)]
    pub(crate) scratch: Arc<std::sync::Mutex<Vec<tempfile::TempDir>>>,
}

pub(crate) async fn build(config: StartupConfig, assets: AssetPaths) -> Result<AppState, String> {
    let agents = AgentStore::open(
        config.data_dir.join("agents"),
        &config.data_dir.join("project.json"),
    )
    .map_err(|error| error.message().to_owned())?;
    let environments = EnvironmentCatalogue::open(
        config.data_dir.join("environments.json"),
        config.data_dir.join("environment-preparation-logs"),
    )
    .map_err(|error| error.message().to_owned())?;
    let seeds = environments
        .seed_id(crate::environments::seeds::ALPINE_GIT_V1)
        .map(crate::workflows::seeds::production_seeds)
        .unwrap_or_default();
    let workflows =
        WorkflowCatalogue::open_with_seeds(config.data_dir.join("workflows.json"), &seeds)
            .map_err(|error| error.message().to_owned())?;
    let workflow_artefacts =
        WorkflowArtefactRepository::open(config.data_dir.join("workflow-artefacts"))
            .map_err(|error| error.message().to_owned())?;
    let workflow_runs = WorkflowRunStore::open(config.data_dir.join("workflow-runs"))
        .map_err(|error| error.message().to_owned())?;
    let environment_snapshots =
        EnvironmentSnapshotRepository::open(config.data_dir.join("environment-snapshots"))
            .map_err(|_| "The environment snapshot store is unreadable.".to_owned())?;
    let sandboxes = SandboxFleet::prepare().await;
    let guest_recovery = sandboxes.recover_transient_guests().await;
    for environment in environments.list() {
        let Some(ready) = environment.ready_preparation else {
            continue;
        };
        let Some(preparation) = environments.preparation(&ready) else {
            continue;
        };
        let Some(snapshot) = preparation.snapshot else {
            continue;
        };
        let _ = environment_snapshots.inspect(&snapshot).await;
    }
    let environments = Arc::new(environments);
    let environment_snapshots = Arc::new(environment_snapshots);
    let environment_preparations =
        EnvironmentPreparationScheduler::start(environments.clone(), environment_snapshots.clone());
    environment_preparations.wake();
    let workflow_workspaces = WorkflowWorkspaces::open(config.data_dir.join("workflow-workspaces"))
        .map_err(|_| "The workflow workspace store is unreadable.".to_owned())?;
    let workspace_recovery = workflow_workspaces
        .recover_leftovers(
            |run, attempt| {
                workflow_runs
                    .get(run)
                    .is_some_and(|record| record.active_attempt() == Some(*attempt))
            },
            |run, attempt| {
                !guest_recovery.inventory_complete
                    || guest_recovery.attempts_remaining.contains(attempt)
                    || guest_recovery.runs_remaining.contains(run)
            },
        )
        .map_err(|_| "Power Plant could not recover workflow workspaces.".to_owned())?;
    for (run_id, attempt_id) in workflow_runs.pending_cleanup_attempts() {
        let cleanup = recovered_cleanup_record(
            guest_recovery.inventory_complete,
            &guest_recovery.attempts_remaining,
            &guest_recovery.runs_remaining,
            &workspace_recovery,
            run_id,
            attempt_id,
        );
        workflow_runs
            .mutate(&run_id, |run| run.record_cleanup(attempt_id, cleanup))
            .map_err(|_| "Power Plant could not record workflow recovery.".to_owned())?;
    }
    Ok(AppState {
        config: Arc::new(config.runtime),
        assets: Arc::new(assets),
        sessions: Arc::new(SessionStore::new()),
        vault: Arc::new(ProviderVault::open(config.data_dir.join("providers.json"))),
        chat: Arc::new(ChatBackend::Rig),
        models: Arc::new(ModelCatalogue::default()),
        plan_login: Arc::new(PlanLogin::new()),
        agents: Arc::new(agents),
        sandboxes: Arc::new(sandboxes),
        agent_leases: Arc::new(AgentLeaseCoordinator::new()),
        workflows: Arc::new(workflows),
        workflow_runs: Arc::new(workflow_runs),
        workflow_artefacts: Arc::new(workflow_artefacts),
        workflow_execution: Arc::new(WorkflowExecution::new()),
        workflow_workspaces: Arc::new(workflow_workspaces),
        environments,
        environment_snapshots,
        environment_preparations,
        #[cfg(test)]
        scratch: Arc::new(std::sync::Mutex::new(Vec::new())),
    })
}

fn recovered_cleanup_record(
    inventory_complete: bool,
    guests_remaining: &std::collections::BTreeSet<crate::workflows::AttemptId>,
    runs_remaining: &std::collections::BTreeSet<crate::workflows::RunId>,
    workspaces: &[crate::workflows::workspace::WorkspaceRecovery],
    run: crate::workflows::RunId,
    attempt: crate::workflows::AttemptId,
) -> crate::workflows::run::AttemptCleanupRecord {
    let sandbox =
        !inventory_complete || guests_remaining.contains(&attempt) || runs_remaining.contains(&run);
    let workspace = workspaces
        .iter()
        .any(|item| item.run == run && item.attempt == attempt && item.remains);
    if sandbox || workspace {
        crate::workflows::run::AttemptCleanupRecord::Orphaned { sandbox, workspace }
    } else {
        crate::workflows::run::AttemptCleanupRecord::Complete
    }
}

#[cfg(test)]
pub(crate) fn for_test(config: RuntimeConfig) -> AppState {
    let environments = Arc::new(EnvironmentCatalogue::in_memory());
    let environment_snapshots = Arc::new(EnvironmentSnapshotRepository::in_memory());
    let environment_preparations =
        EnvironmentPreparationScheduler::idle(environments.clone(), environment_snapshots.clone());
    AppState {
        config: Arc::new(config),
        assets: Arc::new(AssetPaths {
            css_path: "/static/test.css".to_owned(),
            js_path: "/static/test.js".to_owned(),
        }),
        sessions: Arc::new(SessionStore::new()),
        vault: Arc::new(ProviderVault::in_memory()),
        chat: Arc::new(ChatBackend::Scripted(
            crate::providers::scripted::ScriptedBackend::accept(),
        )),
        models: Arc::new(ModelCatalogue::default()),
        plan_login: Arc::new(PlanLogin::new()),
        agents: Arc::new(AgentStore::in_memory()),
        sandboxes: Arc::new(SandboxFleet::for_test()),
        agent_leases: Arc::new(AgentLeaseCoordinator::new()),
        workflows: Arc::new(WorkflowCatalogue::in_memory()),
        workflow_runs: Arc::new(WorkflowRunStore::in_memory()),
        workflow_artefacts: Arc::new(WorkflowArtefactRepository::in_memory()),
        workflow_execution: Arc::new(WorkflowExecution::new()),
        workflow_workspaces: Arc::new(WorkflowWorkspaces::in_memory()),
        environments,
        environment_snapshots,
        environment_preparations,
        scratch: Arc::new(std::sync::Mutex::new(Vec::new())),
    }
}

#[cfg(test)]
mod tests;
