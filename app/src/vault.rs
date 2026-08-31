use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::providers::{
    AuthMethod, MAXIMUM_FAVOURITES, ProviderConnection, ProviderKind, SecretString,
    api_key_is_bounded, effective_plan_model, model_is_bounded,
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
    auth: AuthMethod,
    api_key: SecretString,
    model: String,
    favourites: Vec<String>,
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
    auth: String,
    #[serde(default)]
    api_key: String,
    model: String,
    #[serde(default)]
    favourites: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VaultError {
    Corrupt,
    Persist,
}

impl VaultError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Corrupt => "The provider vault is unreadable.",
            Self::Persist => "Power Plant could not store the provider vault. Try again.",
        }
    }
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for VaultError {}

#[derive(Debug)]
pub(crate) enum FavouriteError {
    Provider,
    Full,
    Persist(VaultError),
}

impl std::fmt::Display for FavouriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Provider => "provider is not stored",
            Self::Full => "favourite list is full",
            Self::Persist(_) => "provider vault persist failed",
        })
    }
}

impl std::error::Error for FavouriteError {}

pub(crate) struct DeskProvider {
    pub(crate) kind: ProviderKind,
    pub(crate) auth: AuthMethod,
    pub(crate) model: String,
    pub(crate) selected: bool,
    pub(crate) favourites: Vec<String>,
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

    pub(crate) fn open(path: PathBuf) -> Result<Self, VaultError> {
        let state = load(&path)?;
        Ok(Self {
            path: Some(path),
            inner: Mutex::new(state),
        })
    }

    pub(crate) fn has_providers(&self) -> bool {
        !self.lock().providers.is_empty()
    }

    pub(crate) fn contains(&self, kind: ProviderKind) -> bool {
        self.lock().providers.contains_key(&kind)
    }

    pub(crate) fn selected_connection(&self) -> Option<ProviderConnection> {
        let state = self.lock();
        connection_from(self.path.as_deref(), &state, state.selected?)
    }

    pub(crate) fn connections(&self) -> Vec<ProviderConnection> {
        let state = self.lock();
        ProviderKind::ALL
            .into_iter()
            .filter_map(|kind| connection_from(self.path.as_deref(), &state, kind))
            .collect()
    }

    pub(crate) fn desk_providers(&self) -> Vec<DeskProvider> {
        let state = self.lock();
        ProviderKind::ALL
            .into_iter()
            .filter_map(|kind| {
                state.providers.get(&kind).map(|stored| DeskProvider {
                    kind,
                    auth: stored.auth,
                    model: stored_model(kind, stored),
                    selected: state.selected == Some(kind),
                    favourites: stored.favourites.clone(),
                })
            })
            .collect()
    }

    pub(crate) fn put(&self, connection: ProviderConnection) -> Result<(), VaultError> {
        self.mutate(|state| {
            let (model, favourites) = state
                .providers
                .get(&connection.kind)
                .map(|stored| (stored.model.clone(), stored.favourites.clone()))
                .unwrap_or((connection.model, Vec::new()));
            if connection.auth == AuthMethod::ApiKey {
                delete_plan_file(self.path.as_deref(), connection.kind);
            }
            state.providers.insert(
                connection.kind,
                StoredProvider {
                    auth: connection.auth,
                    api_key: connection.api_key,
                    model,
                    favourites,
                },
            );
            state.selected = Some(connection.kind);
        })
    }

    pub(crate) fn plan_file(&self, kind: ProviderKind) -> Option<PathBuf> {
        plan_file_path(self.path.as_deref(), kind)
    }

    pub(crate) fn forget(&self, kind: ProviderKind) -> Result<(), VaultError> {
        self.mutate(|state| {
            state.providers.remove(&kind);
            delete_plan_file(self.path.as_deref(), kind);
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

    pub(crate) fn toggle_favourite(
        &self,
        kind: ProviderKind,
        model: &str,
    ) -> Result<bool, FavouriteError> {
        match self.mutate(|state| {
            let Some(stored) = state.providers.get_mut(&kind) else {
                return Err(FavouriteError::Provider);
            };
            let position = stored.favourites.iter().position(|item| item == model);
            if position.is_none() && stored.favourites.len() >= MAXIMUM_FAVOURITES {
                return Err(FavouriteError::Full);
            }
            let favourite = if let Some(position) = position {
                stored.favourites.remove(position);
                false
            } else {
                stored.favourites.push(model.to_owned());
                true
            };
            Ok(favourite)
        }) {
            Ok(Ok(favourite)) => Ok(favourite),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(FavouriteError::Persist(error)),
        }
    }

    fn mutate<R>(&self, edit: impl FnOnce(&mut VaultState) -> R) -> Result<R, VaultError> {
        let mut state = self.lock();
        let previous = state.clone();
        let value = edit(&mut state);
        if let Err(error) = persist(self.path.as_deref(), &state) {
            *state = previous;
            return Err(error);
        }
        Ok(value)
    }

    fn lock(&self) -> MutexGuard<'_, VaultState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn connection_from(
    path: Option<&Path>,
    state: &VaultState,
    kind: ProviderKind,
) -> Option<ProviderConnection> {
    state.providers.get(&kind).map(|stored| ProviderConnection {
        kind,
        auth: stored.auth,
        api_key: stored.api_key.clone(),
        model: stored_model(kind, stored),
        plan_file: (stored.auth == AuthMethod::Plan)
            .then(|| plan_file_path(path, kind))
            .flatten(),
    })
}

fn stored_model(kind: ProviderKind, stored: &StoredProvider) -> String {
    match stored.auth {
        AuthMethod::Plan => effective_plan_model(kind, &stored.model),
        AuthMethod::ApiKey => stored.model.clone(),
    }
}

fn load(path: &Path) -> Result<VaultState, VaultError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(VaultState::default()),
        Err(_) => return Err(VaultError::Corrupt),
    };
    let file: VaultFile = serde_json::from_slice(&bytes).map_err(|_| VaultError::Corrupt)?;
    if file.version != VAULT_VERSION {
        return Err(VaultError::Corrupt);
    }
    let mut state = VaultState::default();
    for entry in file.providers {
        let Some(kind) = ProviderKind::parse(&entry.kind) else {
            return Err(VaultError::Corrupt);
        };
        if state.providers.contains_key(&kind) {
            return Err(VaultError::Corrupt);
        }
        let Some(auth) = AuthMethod::parse(&entry.auth) else {
            return Err(VaultError::Corrupt);
        };
        if !model_is_canonical(&entry.model) {
            return Err(VaultError::Corrupt);
        }
        let api_key = match auth {
            AuthMethod::ApiKey => {
                if entry.api_key.trim() != entry.api_key || !api_key_is_bounded(&entry.api_key) {
                    return Err(VaultError::Corrupt);
                }
                SecretString::new(entry.api_key)
            }
            AuthMethod::Plan => {
                if !kind.supports_plan() || !entry.api_key.is_empty() {
                    return Err(VaultError::Corrupt);
                }
                SecretString::new(String::new())
            }
        };
        if entry.favourites.len() > MAXIMUM_FAVOURITES {
            return Err(VaultError::Corrupt);
        }
        let mut favourites = Vec::with_capacity(entry.favourites.len());
        for item in entry.favourites {
            if !model_is_canonical(&item) || favourites.iter().any(|seen| seen == &item) {
                return Err(VaultError::Corrupt);
            }
            favourites.push(item);
        }
        state.providers.insert(
            kind,
            StoredProvider {
                auth,
                api_key,
                model: entry.model,
                favourites,
            },
        );
    }
    state.selected = match file.selected.as_deref() {
        None if state.providers.is_empty() => None,
        None => return Err(VaultError::Corrupt),
        Some(value) => {
            let Some(kind) = ProviderKind::parse(value) else {
                return Err(VaultError::Corrupt);
            };
            if !state.providers.contains_key(&kind) {
                return Err(VaultError::Corrupt);
            }
            Some(kind)
        }
    };
    Ok(state)
}

fn model_is_canonical(model: &str) -> bool {
    !model.is_empty() && model.trim() == model && model_is_bounded(model)
}

fn persist(path: Option<&Path>, state: &VaultState) -> Result<(), VaultError> {
    let Some(path) = path else {
        return Ok(());
    };
    if state.providers.is_empty() {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(VaultError::Persist),
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
                    auth: stored.auth.as_str().to_owned(),
                    api_key: match stored.auth {
                        AuthMethod::ApiKey => stored.api_key.expose().to_owned(),
                        AuthMethod::Plan => String::new(),
                    },
                    model: stored.model.clone(),
                    favourites: stored.favourites.clone(),
                })
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|_| VaultError::Persist)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| VaultError::Persist)?;
    }
    crate::storage::write_private(path, &bytes).map_err(|_| VaultError::Persist)
}

fn plan_file_path(vault_path: Option<&Path>, kind: ProviderKind) -> Option<PathBuf> {
    let name = kind.plan_file_name()?;
    Some(vault_path?.parent()?.join(name))
}

fn delete_plan_file(vault_path: Option<&Path>, kind: ProviderKind) {
    if let Some(path) = plan_file_path(vault_path, kind) {
        let _ = fs::remove_file(path);
    }
}
