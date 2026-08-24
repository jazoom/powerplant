use std::sync::Arc;

use crate::{
    assets::AssetPaths,
    config::{RuntimeConfig, StartupConfig},
    providers::ChatBackend,
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
}

pub(crate) fn build(config: StartupConfig, assets: AssetPaths) -> AppState {
    AppState {
        config: Arc::new(config.runtime),
        assets: Arc::new(assets),
        sessions: Arc::new(SessionStore::new()),
        vault: Arc::new(ProviderVault::open(config.data_dir.join("providers.json"))),
        chat: Arc::new(ChatBackend::Rig),
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
    }
}
