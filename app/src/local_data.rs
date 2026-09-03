//! Private ownership of the process data root and restart-applied local reset.
//!
//! The owned root is the only deletion target. Reset never takes a path from a
//! query, form, cookie, or route parameter.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::config::StartupConfig;
use crate::storage::{self, PersistError};

const OWNERSHIP_MARKER_NAME: &str = ".powerplant-data-root";
const OWNERSHIP_CONTENTS: &[u8] = b"powerplant-data-root-v1\n";
const RESET_MARKER_NAME: &str = ".powerplant-reset";
const RESET_CONTENTS: &[u8] = b"powerplant-reset-v1\n";

const LEGACY_ENTRIES: &[&str] = &[
    "agents",
    "project.json",
    "projects.json",
    "environments.json",
    "environment-preparation-logs",
    "environment-snapshots",
    "workflows.json",
    "workflow-artefacts",
    "workflow-runs",
    "workflow-workspaces",
    "workflow-commit-journals",
    "providers.json",
    "preferences.json",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum ResetRequest {
    Recorded,
    Pending,
}

#[derive(Clone)]
pub(crate) struct LocalDataReset {
    root: PathBuf,
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    pending: bool,
}

#[derive(Clone, Copy)]
enum PrepareError {
    ConfiguredPath,
    UnusableDirectory,
    UnownedRoot,
    Prepare,
    Reset,
}

impl PrepareError {
    fn message(self) -> &'static str {
        match self {
            Self::ConfiguredPath => {
                "POWERPLANT_DATA_DIR must name a directory with a final path component."
            }
            Self::UnusableDirectory => "The Power Plant data directory is not a usable directory.",
            Self::UnownedRoot => "The Power Plant data directory is not a private owned root.",
            Self::Prepare => "Power Plant could not prepare the data directory.",
            Self::Reset => "Power Plant could not reset local data.",
        }
    }
}

pub(crate) fn prepare(
    mut config: StartupConfig,
) -> Result<(StartupConfig, LocalDataReset), String> {
    let root = establish(&config).map_err(|error| error.message().to_owned())?;
    config.data_dir = root.clone();
    Ok((config, LocalDataReset::new(root)))
}

impl LocalDataReset {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            inner: Arc::new(Mutex::new(Inner { pending: false })),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn is_pending(&self) -> bool {
        lock(&self.inner).pending
    }

    #[allow(dead_code)]
    pub(crate) fn request_reset(&self) -> Result<ResetRequest, PersistError> {
        let mut inner = lock(&self.inner);
        if inner.pending {
            return Ok(ResetRequest::Pending);
        }
        let owned = marker_is_exact(&self.root, OWNERSHIP_MARKER_NAME, OWNERSHIP_CONTENTS)
            .map_err(|_| PersistError)?;
        if !owned {
            return Err(PersistError);
        }
        if marker_is_exact(&self.root, RESET_MARKER_NAME, RESET_CONTENTS)
            .map_err(|_| PersistError)?
        {
            inner.pending = true;
            return Ok(ResetRequest::Pending);
        }
        let path = storage::confined_child(&self.root, RESET_MARKER_NAME)?;
        storage::write_private(&path, RESET_CONTENTS)?;
        inner.pending = true;
        Ok(ResetRequest::Recorded)
    }
}

fn establish(config: &StartupConfig) -> Result<PathBuf, PrepareError> {
    let configured = &config.data_dir;
    if !usable_configured_path(configured) {
        return Err(PrepareError::ConfiguredPath);
    }
    let protected = expand_protected(&config.protected_user_roots);
    reject_protected(configured, &protected)?;
    ensure_directory(configured)?;
    let root = canonicalize_directory(configured)?;
    if !usable_configured_path(&root) {
        return Err(PrepareError::ConfiguredPath);
    }
    reject_protected(&root, &protected)?;
    let owned = marker_is_exact(&root, OWNERSHIP_MARKER_NAME, OWNERSHIP_CONTENTS)?;
    if !owned {
        ensure_claimable(&root)?;
    }
    storage::ensure_private_dir(&root).map_err(|_| PrepareError::Prepare)?;
    if !owned {
        write_ownership_marker(&root).map_err(|_| PrepareError::Prepare)?;
    }
    if marker_is_exact(&root, RESET_MARKER_NAME, RESET_CONTENTS)? {
        return apply_reset(&root);
    }
    Ok(root)
}

fn usable_configured_path(path: &Path) -> bool {
    path.is_absolute() && path.file_name().is_some()
}

fn expand_protected(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut expanded = Vec::with_capacity(roots.len().saturating_mul(2));
    for root in roots {
        if !root.is_absolute() {
            continue;
        }
        if let Ok(canonical) = fs::canonicalize(root)
            && !expanded.contains(&canonical)
        {
            expanded.push(canonical);
        }
        if !expanded.contains(root) {
            expanded.push(root.clone());
        }
    }
    expanded
}

// Deleting this root would remove a user home or profile directory.
fn reject_protected(data_root: &Path, protected: &[PathBuf]) -> Result<(), PrepareError> {
    if protected
        .iter()
        .any(|root| root.is_absolute() && root.starts_with(data_root))
    {
        Err(PrepareError::UnownedRoot)
    } else {
        Ok(())
    }
}

fn ensure_directory(path: &Path) -> Result<(), PrepareError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(PrepareError::UnusableDirectory)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|_| PrepareError::Prepare)?;
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    Err(PrepareError::UnusableDirectory)
                }
                Ok(_) => Ok(()),
                Err(_) => Err(PrepareError::Prepare),
            }
        }
        Err(_) => Err(PrepareError::Prepare),
    }
}

fn canonicalize_directory(path: &Path) -> Result<PathBuf, PrepareError> {
    let canonical = fs::canonicalize(path).map_err(|_| PrepareError::Prepare)?;
    match fs::symlink_metadata(&canonical) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(PrepareError::UnusableDirectory)
        }
        Ok(_) => Ok(canonical),
        Err(_) => Err(PrepareError::Prepare),
    }
}

fn marker_is_exact(root: &Path, name: &str, expected: &[u8]) -> Result<bool, PrepareError> {
    let path = storage::confined_child(root, name).map_err(|_| PrepareError::UnownedRoot)?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(PrepareError::UnownedRoot),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(PrepareError::UnownedRoot)
        }
        Ok(_) => {
            let bytes = storage::read_private_bounded(&path, expected.len())
                .map_err(|_| PrepareError::UnownedRoot)?;
            if bytes == expected {
                Ok(true)
            } else {
                Err(PrepareError::UnownedRoot)
            }
        }
    }
}

fn ensure_claimable(root: &Path) -> Result<(), PrepareError> {
    let entries = fs::read_dir(root).map_err(|_| PrepareError::Prepare)?;
    for entry in entries {
        let entry = entry.map_err(|_| PrepareError::Prepare)?;
        let Some(expects_directory) = legacy_entry_kind(&entry.file_name()) else {
            return Err(PrepareError::UnownedRoot);
        };
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| PrepareError::Prepare)?;
        if metadata.file_type().is_symlink() || expects_directory != metadata.file_type().is_dir() {
            return Err(PrepareError::UnownedRoot);
        }
    }
    Ok(())
}

fn legacy_entry_kind(name: &std::ffi::OsStr) -> Option<bool> {
    LEGACY_ENTRIES
        .iter()
        .find(|entry| **entry == name)
        .map(|entry| !entry.ends_with(".json"))
}

fn write_ownership_marker(root: &Path) -> Result<(), PersistError> {
    let path = storage::confined_child(root, OWNERSHIP_MARKER_NAME)?;
    storage::write_private(&path, OWNERSHIP_CONTENTS)
}

fn apply_reset(root: &Path) -> Result<PathBuf, PrepareError> {
    storage::remove_tree_nofollow(root).map_err(|_| PrepareError::Reset)?;
    storage::ensure_private_dir(root).map_err(|_| PrepareError::Reset)?;
    let root = canonicalize_directory(root).map_err(|_| PrepareError::Reset)?;
    if !directory_is_empty(&root)? {
        return Err(PrepareError::Reset);
    }
    write_ownership_marker(&root).map_err(|_| PrepareError::Reset)?;
    Ok(root)
}

fn directory_is_empty(root: &Path) -> Result<bool, PrepareError> {
    let mut entries = fs::read_dir(root).map_err(|_| PrepareError::Reset)?;
    match entries.next() {
        None => Ok(true),
        Some(Ok(_)) => Ok(false),
        Some(Err(_)) => Err(PrepareError::Reset),
    }
}

fn lock(mutex: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
