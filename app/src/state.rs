use std::sync::Arc;

use crate::{
    assets::AssetPaths,
    config::{RuntimeConfig, StartupConfig},
    models::ModelCatalogue,
    plan_login::PlanLogin,
    providers::ChatBackend,
    sandbox::GuestSandbox,
    sessions::SessionStore,
    vault::ProviderVault,
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
    pub(crate) sandbox: Arc<GuestSandbox>,
}

pub(crate) async fn build(config: StartupConfig, assets: AssetPaths) -> AppState {
    AppState {
        config: Arc::new(config.runtime),
        assets: Arc::new(assets),
        sessions: Arc::new(SessionStore::new()),
        vault: Arc::new(ProviderVault::open(config.data_dir.join("providers.json"))),
        chat: Arc::new(ChatBackend::Rig),
        models: Arc::new(ModelCatalogue::default()),
        plan_login: Arc::new(PlanLogin::new()),
        sandbox: Arc::new(GuestSandbox::prepare().await),
    }
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
        sandbox: Arc::new(GuestSandbox::for_test()),
    }
}
