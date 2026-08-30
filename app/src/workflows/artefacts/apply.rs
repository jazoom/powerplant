use std::path::{Path, PathBuf};

use super::candidate::{
    CandidateEntry, CandidateEntryKind, CandidateRevisionArtefact, CaptureError, hash_entries,
};
use super::confine::{WorkspaceDir, WorkspaceKind, split_relative};
use super::id::ObjectHash;
use super::store::WorkflowArtefactRepository;
use super::{ArtefactHash, artefact_hash_for};
use crate::workflows::definition::ArtefactKind;

pub(crate) struct CandidateApply;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApplyError {
    Conflict,
    Escape,
    Unsupported,
    Drift,
    Write,
    Integrity,
}

impl CandidateApply {
    pub(crate) fn apply(
        project: &Path,
        initial: &CandidateRevisionArtefact,
        target: &CandidateRevisionArtefact,
        expected_target_hash: ArtefactHash,
        store: &WorkflowArtefactRepository,
    ) -> Result<(), ApplyError> {
        preflight(project, initial, target, expected_target_hash, store)?;
        if let Err(error) = apply_changes(project, initial, target, store) {
            restore_after_failure(project, initial, target, store)?;
            return Err(error);
        }
        if let Err(error) = verify_target(project, target, store) {
            restore_after_failure(project, initial, target, store)?;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn rollback(
        project: &Path,
        initial: &CandidateRevisionArtefact,
        target: &CandidateRevisionArtefact,
        store: &WorkflowArtefactRepository,
    ) -> Result<(), ApplyError> {
        apply_changes(project, target, initial, store)?;
        verify_target(project, initial, store)
    }
}

fn preflight(
    project: &Path,
    initial: &CandidateRevisionArtefact,
    target: &CandidateRevisionArtefact,
    expected_target_hash: ArtefactHash,
    store: &WorkflowArtefactRepository,
) -> Result<(), ApplyError> {
    let initial_bytes = initial
        .manifest_bytes()
        .map_err(|_| ApplyError::Integrity)?;
    validate_artefact(
        initial,
        artefact_hash_for(
            ArtefactKind::CandidateRevision,
            initial.format_version,
            &initial_bytes,
        ),
    )?;
    validate_artefact(target, expected_target_hash)?;
    let first =
        super::candidate::CandidateCapture::capture_host(project, store).map_err(map_capture)?;
    let second =
        super::candidate::CandidateCapture::capture_host(project, store).map_err(map_capture)?;
    if first != *initial || second != first {
        return Err(ApplyError::Drift);
    }
    for artefact in [initial, target] {
        for entry in &artefact.entries {
            split_relative(&entry.path).map_err(|_| ApplyError::Escape)?;
        }
    }
    let workspace = WorkspaceDir::open(project).map_err(map_capture)?;
    let host_leaves = workspace.collect_leaf_paths().map_err(map_capture)?;
    let initial_paths: Vec<_> = initial
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    let target_paths: Vec<_> = target
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    for path in &host_leaves {
        if path_under_symlink(&workspace, path) {
            return Err(ApplyError::Unsupported);
        }
        if initial_paths.contains(&path.as_str()) {
            continue;
        }
        if target_paths.iter().any(|target| {
            *target == path.as_str()
                || target.starts_with(&format!("{path}/"))
                || path.starts_with(&format!("{target}/"))
        }) {
            return Err(ApplyError::Conflict);
        }
    }
    for entry in &target.entries {
        if path_under_symlink(&workspace, &entry.path) {
            return Err(ApplyError::Unsupported);
        }
        match &entry.kind {
            CandidateEntryKind::Regular { bytes, blob, .. } => {
                let data = store.get(blob).map_err(|_| ApplyError::Integrity)?;
                if data.len() as u64 != *bytes || ObjectHash::of(&data) != *blob {
                    return Err(ApplyError::Integrity);
                }
            }
            CandidateEntryKind::Symlink { target, blob } => {
                let data = store.get(blob).map_err(|_| ApplyError::Integrity)?;
                if data.as_slice() != target.as_bytes() || ObjectHash::of(&data) != *blob {
                    return Err(ApplyError::Integrity);
                }
            }
            CandidateEntryKind::Gitlink { .. } => {}
        }
    }
    Ok(())
}

fn restore_after_failure(
    project: &Path,
    initial: &CandidateRevisionArtefact,
    target: &CandidateRevisionArtefact,
    store: &WorkflowArtefactRepository,
) -> Result<(), ApplyError> {
    apply_changes(project, target, initial, store).map_err(|_| ApplyError::Write)?;
    verify_target(project, initial, store).map_err(|_| ApplyError::Write)
}

fn apply_changes(
    project: &Path,
    from: &CandidateRevisionArtefact,
    to: &CandidateRevisionArtefact,
    store: &WorkflowArtefactRepository,
) -> Result<(), ApplyError> {
    let workspace = WorkspaceDir::open(project).map_err(map_capture)?;
    let mut removals: Vec<&str> = from
        .entries
        .iter()
        .filter(|entry| to.entries.iter().all(|item| item.path != entry.path))
        .map(|entry| entry.path.as_str())
        .collect();
    removals.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
    for path in removals {
        if workspace.exists(path) {
            workspace.remove_leaf(path).map_err(map_capture)?;
            prune_empty_parents(&workspace, path, to)?;
        }
    }
    let mut additions = to.entries.clone();
    additions.sort_by_key(|entry| entry.path.matches('/').count());
    for entry in additions {
        write_entry(&workspace, store, &entry)?;
    }
    Ok(())
}

fn write_entry(
    workspace: &WorkspaceDir,
    store: &WorkflowArtefactRepository,
    entry: &CandidateEntry,
) -> Result<(), ApplyError> {
    match &entry.kind {
        CandidateEntryKind::Regular {
            executable,
            bytes,
            blob,
        } => {
            let data = store.get(blob).map_err(|_| ApplyError::Integrity)?;
            if data.len() as u64 != *bytes {
                return Err(ApplyError::Integrity);
            }
            if workspace.exists(&entry.path) {
                workspace
                    .replace_file(&entry.path, &data, *executable)
                    .map_err(map_capture)?;
            } else {
                workspace
                    .write_file(&entry.path, &data, *executable)
                    .map_err(map_capture)?;
            }
        }
        CandidateEntryKind::Symlink { target, .. } => {
            if workspace.exists(&entry.path) {
                workspace.remove_leaf(&entry.path).map_err(map_capture)?;
            }
            workspace
                .create_symlink(&entry.path, target)
                .map_err(map_capture)?;
        }
        CandidateEntryKind::Gitlink { .. } => {
            if workspace.exists(&entry.path) {
                match workspace.kind(&entry.path).map_err(map_capture)? {
                    WorkspaceKind::Directory => {}
                    _ => {
                        workspace.remove_leaf(&entry.path).map_err(map_capture)?;
                        workspace
                            .create_placeholder_dir(&entry.path)
                            .map_err(map_capture)?;
                    }
                }
            } else {
                workspace
                    .create_placeholder_dir(&entry.path)
                    .map_err(map_capture)?;
            }
        }
    }
    Ok(())
}

fn prune_empty_parents(
    workspace: &WorkspaceDir,
    path: &str,
    target: &CandidateRevisionArtefact,
) -> Result<(), ApplyError> {
    let mut current = PathBuf::from(path);
    while let Some(parent) = current.parent() {
        if parent.as_os_str().is_empty() {
            break;
        }
        let relative = parent.to_string_lossy().into_owned();
        if target
            .entries
            .iter()
            .any(|entry| entry.path == relative || entry.path.starts_with(&format!("{relative}/")))
        {
            break;
        }
        if workspace.exists(&relative)
            && matches!(
                workspace.kind(&relative).map_err(map_capture)?,
                WorkspaceKind::Directory
            )
            && workspace.dir_is_empty(&relative).map_err(map_capture)?
        {
            workspace.remove_leaf(&relative).map_err(map_capture)?;
        } else {
            break;
        }
        current = parent.to_path_buf();
    }
    Ok(())
}

fn verify_target(
    project: &Path,
    target: &CandidateRevisionArtefact,
    store: &WorkflowArtefactRepository,
) -> Result<(), ApplyError> {
    let workspace = WorkspaceDir::open(project).map_err(map_capture)?;
    let mut reread = Vec::new();
    for entry in &target.entries {
        reread.push(reread_entry(&workspace, store, entry)?);
    }
    if hash_entries(&reread) != target.candidate_hash {
        return Err(ApplyError::Drift);
    }
    Ok(())
}

fn reread_entry(
    workspace: &WorkspaceDir,
    store: &WorkflowArtefactRepository,
    entry: &CandidateEntry,
) -> Result<CandidateEntry, ApplyError> {
    match workspace.kind(&entry.path).map_err(map_capture)? {
        WorkspaceKind::File { executable } => {
            let (bytes, opened_executable, size) =
                workspace.read_file(&entry.path).map_err(map_capture)?;
            if opened_executable != executable {
                return Err(ApplyError::Drift);
            }
            let blob = store.publish(&bytes).map_err(|_| ApplyError::Write)?;
            Ok(CandidateEntry {
                path: entry.path.clone(),
                kind: CandidateEntryKind::Regular {
                    executable,
                    bytes: size,
                    blob,
                },
            })
        }
        WorkspaceKind::Symlink => {
            let target = workspace.read_link(&entry.path).map_err(map_capture)?;
            let blob = store
                .publish(target.as_bytes())
                .map_err(|_| ApplyError::Write)?;
            Ok(CandidateEntry {
                path: entry.path.clone(),
                kind: CandidateEntryKind::Symlink { target, blob },
            })
        }
        WorkspaceKind::Directory => {
            let CandidateEntryKind::Gitlink { commit } = &entry.kind else {
                return Err(ApplyError::Drift);
            };
            Ok(CandidateEntry {
                path: entry.path.clone(),
                kind: CandidateEntryKind::Gitlink {
                    commit: commit.clone(),
                },
            })
        }
        WorkspaceKind::Other => Err(ApplyError::Unsupported),
    }
}

fn validate_artefact(
    artefact: &CandidateRevisionArtefact,
    expected: ArtefactHash,
) -> Result<(), ApplyError> {
    let bytes = artefact
        .manifest_bytes()
        .map_err(|_| ApplyError::Integrity)?;
    let hash = artefact_hash_for(
        ArtefactKind::CandidateRevision,
        artefact.format_version,
        &bytes,
    );
    if hash != expected || hash_entries(&artefact.entries) != artefact.candidate_hash {
        return Err(ApplyError::Integrity);
    }
    Ok(())
}

fn path_under_symlink(workspace: &WorkspaceDir, path: &str) -> bool {
    let mut current = PathBuf::new();
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 {
        return false;
    }
    for part in &parts[..parts.len() - 1] {
        current.push(part);
        let relative = current.to_string_lossy();
        if workspace.exists(relative.as_ref())
            && workspace.kind(relative.as_ref()).ok() == Some(WorkspaceKind::Symlink)
        {
            return true;
        }
    }
    false
}

fn map_capture(error: CaptureError) -> ApplyError {
    match error {
        CaptureError::SourceUnsupported => ApplyError::Unsupported,
        CaptureError::SourceChanged | CaptureError::SourceRead => ApplyError::Drift,
        CaptureError::ArtefactIntegrity => ApplyError::Integrity,
        _ => ApplyError::Write,
    }
}

#[cfg(test)]
mod tests;
