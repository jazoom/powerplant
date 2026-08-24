use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::providers::{
    ProviderConnection, ProviderKind, SecretString, api_key_is_bounded, model_is_bounded,
};

#[cfg(test)]
mod tests;

const VAULT_VERSION: u32 = 1;

#[derive(Clone, Default)]
struct VaultState {
    selected: Option<ProviderKind>,
    providers: HashMap<ProviderKind, StoredProvider>,
}

#[derive(Clone)]
struct StoredProvider {
    api_key: SecretString,
    model: String,
}

#[derive(Deserialize, Serialize)]
struct VaultFile {
    version: u32,
    selected: Option<String>,
    providers: Vec<VaultFileProvider>,
}

#[derive(Deserialize, Serialize)]
struct VaultFileProvider {
    kind: String,
    api_key: String,
    model: String,
}

#[derive(Debug)]
pub(crate) struct VaultError;

impl std::fmt::Display for VaultError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("provider vault persist failed")
    }
}

impl std::error::Error for VaultError {}

pub(crate) struct DeskProvider {
    pub(crate) kind: ProviderKind,
    pub(crate) model: String,
    pub(crate) selected: bool,
}

pub(crate) struct ProviderVault {
    path: Option<PathBuf>,
    inner: Mutex<VaultState>,
}

impl ProviderVault {
    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            inner: Mutex::new(VaultState::default()),
        }
    }

    pub(crate) fn open(path: PathBuf) -> Self {
        let state = load(&path).unwrap_or_default();
        Self {
            path: Some(path),
            inner: Mutex::new(state),
        }
    }

    pub(crate) fn has_providers(&self) -> bool {
        !self.lock().providers.is_empty()
    }

    pub(crate) fn contains(&self, kind: ProviderKind) -> bool {
        self.lock().providers.contains_key(&kind)
    }

    pub(crate) fn selected_connection(&self) -> Option<ProviderConnection> {
        let state = self.lock();
        connection_from(&state, state.selected?)
    }

    pub(crate) fn desk_providers(&self) -> Vec<DeskProvider> {
        let state = self.lock();
        ProviderKind::ALL
            .into_iter()
            .filter_map(|kind| {
                state.providers.get(&kind).map(|stored| DeskProvider {
                    kind,
                    model: stored.model.clone(),
                    selected: state.selected == Some(kind),
                })
            })
            .collect()
    }

    pub(crate) fn put(&self, connection: ProviderConnection) -> Result<(), VaultError> {
        self.mutate(|state| {
            let model = state
                .providers
                .get(&connection.kind)
                .map(|stored| stored.model.clone())
                .unwrap_or(connection.model);
            state.providers.insert(
                connection.kind,
                StoredProvider {
                    api_key: connection.api_key,
                    model,
                },
            );
            state.selected = Some(connection.kind);
        })
    }

    pub(crate) fn forget(&self, kind: ProviderKind) -> Result<(), VaultError> {
        self.mutate(|state| {
            state.providers.remove(&kind);
            if state.selected == Some(kind) {
                state.selected = ProviderKind::ALL
                    .into_iter()
                    .find(|candidate| state.providers.contains_key(candidate));
            }
        })
    }

    pub(crate) fn select(&self, kind: ProviderKind, model: String) -> Result<(), VaultError> {
        self.mutate(|state| {
            if let Some(stored) = state.providers.get_mut(&kind) {
                stored.model = model;
                state.selected = Some(kind);
            }
        })
    }

    fn mutate(&self, edit: impl FnOnce(&mut VaultState)) -> Result<(), VaultError> {
        let mut state = self.lock();
        let previous = state.clone();
        edit(&mut state);
        if let Err(error) = persist(self.path.as_deref(), &state) {
            *state = previous;
            return Err(error);
        }
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, VaultState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn connection_from(state: &VaultState, kind: ProviderKind) -> Option<ProviderConnection> {
    state.providers.get(&kind).map(|stored| ProviderConnection {
        kind,
        api_key: stored.api_key.clone(),
        model: stored.model.clone(),
    })
}

fn load(path: &Path) -> Option<VaultState> {
    let bytes = fs::read(path).ok()?;
    let file: VaultFile = serde_json::from_slice(&bytes).ok()?;
    if file.version != VAULT_VERSION {
        return None;
    }
    let mut state = VaultState::default();
    for entry in file.providers {
        let Some(kind) = ProviderKind::parse(&entry.kind) else {
            continue;
        };
        if !api_key_is_bounded(&entry.api_key) || !model_is_bounded(&entry.model) {
            continue;
        }
        let model = if entry.model.trim().is_empty() {
            kind.default_model().to_owned()
        } else {
            entry.model.trim().to_owned()
        };
        state.providers.insert(
            kind,
            StoredProvider {
                api_key: SecretString::new(entry.api_key),
                model,
            },
        );
    }
    state.selected = file
        .selected
        .as_deref()
        .and_then(ProviderKind::parse)
        .filter(|kind| state.providers.contains_key(kind))
        .or_else(|| {
            ProviderKind::ALL
                .into_iter()
                .find(|kind| state.providers.contains_key(kind))
        });
    Some(state)
}

fn persist(path: Option<&Path>, state: &VaultState) -> Result<(), VaultError> {
    let Some(path) = path else {
        return Ok(());
    };
    if state.providers.is_empty() {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(VaultError),
        }
    }
    let file = VaultFile {
        version: VAULT_VERSION,
        selected: state.selected.map(ProviderKind::as_str).map(str::to_owned),
        providers: ProviderKind::ALL
            .into_iter()
            .filter_map(|kind| {
                state.providers.get(&kind).map(|stored| VaultFileProvider {
                    kind: kind.as_str().to_owned(),
                    api_key: stored.api_key.expose().to_owned(),
                    model: stored.model.clone(),
                })
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|_| VaultError)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| VaultError)?;
    }
    write_private(path, &bytes)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    let tmp = path.with_extension("json.tmp");
    let result = (|| {
        let mut file = File::create(&tmp)?;
        restrict_permissions(&file)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        if let Ok(file) = File::open(path) {
            restrict_permissions(&file)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|_: io::Error| VaultError)
}

fn restrict_permissions(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_mode(0o600);
        file.set_permissions(permissions)?;
    }
    Ok(())
}
