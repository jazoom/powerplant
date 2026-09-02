use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use crate::providers::{ProviderConnection, ProviderKind};
use crate::state::AppState;

#[derive(Default)]
pub(crate) struct ModelCatalogue {
    inner: Mutex<HashMap<ProviderKind, CatalogueEntry>>,
}

#[derive(Default)]
struct CatalogueEntry {
    revision: u64,
    models: Vec<String>,
    pending: bool,
}

impl ModelCatalogue {
    pub(crate) fn list(&self, kind: ProviderKind) -> Vec<String> {
        self.lock()
            .get(&kind)
            .map(|entry| entry.models.clone())
            .unwrap_or_default()
    }

    pub(crate) fn pending(&self, kind: ProviderKind) -> bool {
        self.lock().get(&kind).is_some_and(|entry| entry.pending)
    }

    pub(crate) fn remove(&self, kind: ProviderKind) {
        self.lock().remove(&kind);
    }

    fn begin_refresh(&self, kind: ProviderKind) -> u64 {
        let mut catalogue = self.lock();
        let entry = catalogue.entry(kind).or_default();
        entry.revision = entry.revision.wrapping_add(1);
        entry.pending = true;
        entry.revision
    }

    fn finish(&self, kind: ProviderKind, revision: u64, models: Option<Vec<String>>) {
        let mut catalogue = self.lock();
        if let Some(entry) = catalogue.get_mut(&kind)
            && entry.revision == revision
        {
            entry.pending = false;
            if let Some(models) = models {
                entry.models = models;
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ProviderKind, CatalogueEntry>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) fn refresh(state: AppState, connection: ProviderConnection) {
    let revision = state.models.begin_refresh(connection.kind);
    tokio::spawn(async move {
        let models = state.chat.models(&connection).await.ok();
        state.models.finish(connection.kind, revision, models);
    });
}

pub(crate) fn refresh_all(state: &AppState) {
    for connection in state.vault.connections() {
        refresh(state.clone(), connection);
    }
}

#[cfg(test)]
mod tests;
