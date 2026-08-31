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
    #[cfg(test)]
    fail_after_next_persist: Mutex<bool>,
}

impl ProviderVault {
    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            inner: Mutex::new(VaultState::default()),
            fail_after_next_persist: Mutex::new(false),
        }
    }

    pub(crate) fn open(path: PathBuf) -> Result<Self, VaultError> {
        let state = load(&path)?;
        Ok(Self {
            path: Some(path),
            inner: Mutex::new(state),
            #[cfg(test)]
            fail_after_next_persist: Mutex::new(false),
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

    pub(crate) fn insert_api_key(&self, connection: ProviderConnection) -> Result<(), VaultError> {
        if connection.auth != AuthMethod::ApiKey {
            return Err(VaultError::Persist);
        }
        let mut state = self.lock();
        let plan_path = plan_file_path(self.path.as_deref(), connection.kind);
        let previous_plan = plan_path
            .as_deref()
            .map(read_existing_plan)
            .transpose()?
            .flatten();
        let previous = state.clone();
        let (model, favourites) = state
            .providers
            .get(&connection.kind)
            .map(|stored| (stored.model.clone(), stored.favourites.clone()))
            .unwrap_or((connection.model, Vec::new()));
        state.providers.insert(
            connection.kind,
            StoredProvider {
                auth: AuthMethod::ApiKey,
                api_key: connection.api_key,
                model,
                favourites,
            },
        );
        state.selected = Some(connection.kind);
        if self.commit(&state).is_err() {
            *state = previous;
            self.commit(&state)?;
            return Err(VaultError::Persist);
        }
        if let Some(plan_path) = plan_path.as_deref()
            && crate::storage::remove_private(plan_path).is_err()
        {
            restore_plan_file(plan_path, previous_plan.as_deref())?;
            *state = previous;
            self.commit(&state)?;
            return Err(VaultError::Persist);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn put(&self, connection: ProviderConnection) -> Result<(), VaultError> {
        self.insert_api_key(connection)
    }

    pub(crate) fn install_plan(&self, kind: ProviderKind, staged: &Path) -> Result<(), VaultError> {
        if !kind.supports_plan() {
            return Err(VaultError::Persist);
        }
        let final_path = plan_file_path(self.path.as_deref(), kind).ok_or(VaultError::Persist)?;
        let mut state = self.lock();
        let previous = state.clone();
        let previous_plan = stage_existing_plan(&final_path)?;
        if crate::storage::rename_in_dir(staged, &final_path).is_err() {
            return_plan_to_stage_if_moved(&final_path, staged)?;
            restore_plan_backup(&final_path, previous_plan.as_deref())?;
            return Err(VaultError::Persist);
        }
        let (model, favourites) = state
            .providers
            .get(&kind)
            .map(|stored| (stored.model.clone(), stored.favourites.clone()))
            .unwrap_or_else(|| (kind.default_model().to_owned(), Vec::new()));
        state.providers.insert(
            kind,
            StoredProvider {
                auth: AuthMethod::Plan,
                api_key: SecretString::new(String::new()),
                model,
                favourites,
            },
        );
        state.selected = Some(kind);
        if self.commit(&state).is_err() {
            self.rollback_plan_install(
                &mut state,
                previous,
                &final_path,
                staged,
                previous_plan.as_deref(),
            )?;
            return Err(VaultError::Persist);
        }
        if let Some(previous_plan) = previous_plan.as_deref()
            && crate::storage::remove_private(previous_plan).is_err()
        {
            match fs::symlink_metadata(previous_plan) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                _ => {
                    self.rollback_plan_install(
                        &mut state,
                        previous,
                        &final_path,
                        staged,
                        Some(previous_plan),
                    )?;
                    return Err(VaultError::Persist);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn provider_dir(&self) -> Option<PathBuf> {
        Some(self.path.as_ref()?.parent()?.to_path_buf())
    }

    pub(crate) fn plan_file(&self, kind: ProviderKind) -> Option<PathBuf> {
        plan_file_path(self.path.as_deref(), kind)
    }

    #[cfg(test)]
    pub(crate) fn fail_after_next_persist(&self) {
        *self
            .fail_after_next_persist
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
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
        if let Err(error) = self.commit(&state) {
            *state = previous;
            return Err(error);
        }
        Ok(value)
    }

    fn rollback_plan_install(
        &self,
        state: &mut VaultState,
        previous: VaultState,
        final_path: &Path,
        staged: &Path,
        previous_plan: Option<&Path>,
    ) -> Result<(), VaultError> {
        *state = previous;
        if crate::storage::rename_in_dir(final_path, staged).is_err() {
            return_plan_to_stage_if_moved(final_path, staged)?;
        }
        restore_plan_backup(final_path, previous_plan)?;
        self.commit(state)
    }

    fn commit(&self, state: &VaultState) -> Result<(), VaultError> {
        let result = persist(self.path.as_deref(), state);
        #[cfg(test)]
        {
            let mut failure = self
                .fail_after_next_persist
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *failure {
                *failure = false;
                result?;
                return Err(VaultError::Persist);
            }
        }
        result
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

fn read_existing_plan(final_path: &Path) -> Result<Option<Vec<u8>>, VaultError> {
    match fs::symlink_metadata(final_path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(VaultError::Persist),
        Ok(_) => crate::storage::read_private(final_path)
            .map(Some)
            .map_err(|_| VaultError::Persist),
    }
}

fn stage_existing_plan(final_path: &Path) -> Result<Option<PathBuf>, VaultError> {
    let Some(bytes) = read_existing_plan(final_path)? else {
        return Ok(None);
    };
    let dir = final_path.parent().ok_or(VaultError::Persist)?;
    crate::storage::create_unique_private(dir, &bytes)
        .map(Some)
        .map_err(|_| VaultError::Persist)
}

fn return_plan_to_stage_if_moved(final_path: &Path, staged: &Path) -> Result<(), VaultError> {
    match fs::symlink_metadata(staged) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match fs::symlink_metadata(final_path) {
                Ok(_) => crate::storage::rename_in_dir(final_path, staged)
                    .map_err(|_| VaultError::Persist),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(VaultError::Persist),
            }
        }
        Err(_) => Err(VaultError::Persist),
    }
}

fn restore_plan_backup(final_path: &Path, backup: Option<&Path>) -> Result<(), VaultError> {
    if let Some(backup) = backup {
        crate::storage::rename_in_dir(backup, final_path).map_err(|_| VaultError::Persist)?;
    }
    Ok(())
}

fn restore_plan_file(final_path: &Path, bytes: Option<&[u8]>) -> Result<(), VaultError> {
    if let Some(bytes) = bytes {
        crate::storage::write_private(final_path, bytes).map_err(|_| VaultError::Persist)?;
    }
    Ok(())
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
