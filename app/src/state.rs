use std::sync::Arc;

use crate::{
    agents::{AgentLeaseCoordinator, AgentStore},
    assets::AssetPaths,
    config::{RuntimeConfig, StartupConfig},
    models::ModelCatalogue,
    plan_login::PlanLogin,
    providers::ChatBackend,
    sandbox::SandboxFleet,
    sessions::SessionStore,
    vault::ProviderVault,
    workflows::{WorkflowExecution, WorkflowRunStore},
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
    pub(crate) workflow_runs: Arc<WorkflowRunStore>,
    pub(crate) workflow_execution: Arc<WorkflowExecution>,
    #[cfg(test)]
    pub(crate) scratch: Arc<std::sync::Mutex<Vec<tempfile::TempDir>>>,
}

pub(crate) async fn build(config: StartupConfig, assets: AssetPaths) -> Result<AppState, String> {
    let agents = AgentStore::open(
        config.data_dir.join("agents"),
        &config.data_dir.join("project.json"),
    )
    .map_err(|error| error.message().to_owned())?;
    let workflow_runs = WorkflowRunStore::open(config.data_dir.join("workflow-runs"))
        .map_err(|error| error.message().to_owned())?;
    let sandboxes = SandboxFleet::prepare(&agents).await;
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
        workflow_runs: Arc::new(workflow_runs),
        workflow_execution: Arc::new(WorkflowExecution::new()),
        #[cfg(test)]
        scratch: Arc::new(std::sync::Mutex::new(Vec::new())),
    })
}

#[cfg(test)]
pub(crate) fn for_test(config: RuntimeConfig) -> AppState {
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
        workflow_runs: Arc::new(WorkflowRunStore::in_memory()),
        workflow_execution: Arc::new(WorkflowExecution::new()),
        scratch: Arc::new(std::sync::Mutex::new(Vec::new())),
    }
}
