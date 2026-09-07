use std::path::{Path, PathBuf};

use super::candidate::{
    CandidateEntry, CandidateEntryKind, CandidateRevisionArtefact, CaptureError, hash_candidate,
};
use super::confine::{WorkspaceDir, WorkspaceKind, split_relative};
use super::id::ObjectHash;
use super::store::WorkflowArtefactRepository;
use super::{ArtefactHash, artefact_hash_for};
use crate::workflows::definition::ArtefactKind;

pub(crate) struct CandidateApply;

pub(crate) struct CandidateApplicationBinding<'a> {
    pub(crate) initial_hash: ArtefactHash,
    pub(crate) target_hash: ArtefactHash,
    pub(crate) exclusions: &'a [String],
}

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
    pub(crate) fn preflight_bound(
        project: &Path,
        initial: &CandidateRevisionArtefact,
        expected_initial_hash: ArtefactHash,
        target: &CandidateRevisionArtefact,
        expected_target_hash: ArtefactHash,
        expected_exclusions: &[String],
        store: &WorkflowArtefactRepository,
    ) -> Result<(), ApplyError> {
        preflight(
            project,
            initial,
            expected_initial_hash,
            target,
            expected_target_hash,
            expected_exclusions,
            store,
        )
    }

    pub(crate) fn apply(
        project: &Path,
        initial: &CandidateRevisionArtefact,
        target: &CandidateRevisionArtefact,
        expected_target_hash: ArtefactHash,
        store: &WorkflowArtefactRepository,
    ) -> Result<(), ApplyError> {
        let initial_bytes = initial
            .manifest_bytes()
            .map_err(|_| ApplyError::Integrity)?;
        let expected_initial_hash = artefact_hash_for(
            ArtefactKind::CandidateRevision,
            initial.format_version,
            &initial_bytes,
        );
        Self::apply_bound(
            project,
            initial,
            expected_initial_hash,
            target,
            expected_target_hash,
            &initial.exclusions,
            store,
        )
    }

    pub(crate) fn apply_bound(
        project: &Path,
        initial: &CandidateRevisionArtefact,
        expected_initial_hash: ArtefactHash,
        target: &CandidateRevisionArtefact,
        expected_target_hash: ArtefactHash,
        expected_exclusions: &[String],
        store: &WorkflowArtefactRepository,
    ) -> Result<(), ApplyError> {
        preflight(
            project,
            initial,
            expected_initial_hash,
            target,
            expected_target_hash,
            expected_exclusions,
            store,
        )?;
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

    pub(crate) fn apply_journalled(
        project: &Path,
        initial: &CandidateRevisionArtefact,
        target: &CandidateRevisionArtefact,
        binding: CandidateApplicationBinding<'_>,
        store: &WorkflowArtefactRepository,
        mut progress: impl FnMut(usize, &str, bool) -> Result<(), ApplyError>,
    ) -> Result<(), ApplyError> {
        preflight(
            project,
            initial,
            binding.initial_hash,
            target,
            binding.target_hash,
            binding.exclusions,
            store,
        )?;
        let operations = changed_paths(initial, target);
        let mut expected_entries = initial.entries.clone();
        for (index, path) in operations.iter().enumerate() {
            let live = super::candidate::CandidateCapture::capture_directory(
                project,
                binding.exclusions,
                store,
            )
            .map_err(map_capture)?;
            if live.entries != expected_entries
                || live.repository != initial.repository
                || live.git_admin != initial.git_admin
            {
                return Err(ApplyError::Drift);
            }
            progress(index, path, false)?;
            apply_path(project, path, target, store)?;
            expected_entries.retain(|entry| entry.path != *path);
            if let Some(entry) = target.entries.iter().find(|entry| entry.path == *path) {
                expected_entries.push(entry.clone());
                expected_entries
                    .sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
            }
            progress(index + 1, path, true)?;
        }
        verify_target(project, target, store)
    }

    pub(crate) fn recover_to_initial(
        project: &Path,
        initial: &CandidateRevisionArtefact,
        target: &CandidateRevisionArtefact,
        expected_exclusions: &[String],
        started_paths: &[String],
        store: &WorkflowArtefactRepository,
    ) -> Result<(), ApplyError> {
        if initial.exclusions != expected_exclusions || target.exclusions != expected_exclusions {
            return Err(ApplyError::Integrity);
        }
        let live = super::candidate::CandidateCapture::capture_directory(
            project,
            expected_exclusions,
            store,
        )
        .map_err(map_capture)?;
        let mut paths: Vec<_> = initial
            .entries
            .iter()
            .chain(&target.entries)
            .map(|entry| entry.path.clone())
            .collect();
        paths.sort();
        paths.dedup();
        for path in paths {
            let current = live.entries.iter().find(|entry| entry.path == path);
            let before = initial.entries.iter().find(|entry| entry.path == path);
            let after = target.entries.iter().find(|entry| entry.path == path);
            if current != before && (current != after || !started_paths.contains(&path)) {
                return Err(ApplyError::Conflict);
            }
        }
        for entry in &live.entries {
            if initial.entries.iter().all(|item| item.path != entry.path)
                && target.entries.iter().all(|item| item.path != entry.path)
            {
                return Err(ApplyError::Conflict);
            }
        }
        let manifest = live.manifest_bytes().map_err(|_| ApplyError::Integrity)?;
        let initial_manifest = initial
            .manifest_bytes()
            .map_err(|_| ApplyError::Integrity)?;
        Self::apply_journalled(
            project,
            &live,
            initial,
            CandidateApplicationBinding {
                initial_hash: artefact_hash_for(
                    ArtefactKind::CandidateRevision,
                    live.format_version,
                    &manifest,
                ),
                target_hash: artefact_hash_for(
                    ArtefactKind::CandidateRevision,
                    initial.format_version,
                    &initial_manifest,
                ),
                exclusions: expected_exclusions,
            },
            store,
            |_, _, _| Ok(()),
        )
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
    expected_initial_hash: ArtefactHash,
    target: &CandidateRevisionArtefact,
    expected_target_hash: ArtefactHash,
    expected_exclusions: &[String],
    store: &WorkflowArtefactRepository,
) -> Result<(), ApplyError> {
    validate_artefact(initial, expected_initial_hash)?;
    validate_artefact(target, expected_target_hash)?;
    if initial.ordinary != target.ordinary
        || initial.repository != target.repository
        || initial.git_admin != target.git_admin
        || initial.exclusions != expected_exclusions
        || target.exclusions != expected_exclusions
    {
        return Err(ApplyError::Integrity);
    }
    let capture = |root: &Path| {
        if initial.ordinary {
            super::candidate::CandidateCapture::capture_directory(root, expected_exclusions, store)
        } else {
            super::candidate::CandidateCapture::capture_host(root, store)
        }
    };
    let first = capture(project).map_err(map_capture)?;
    let second = capture(project).map_err(map_capture)?;
    if first != *initial || second != first {
        return Err(ApplyError::Drift);
    }
    for artefact in [initial, target] {
        for entry in &artefact.entries {
            split_relative(&entry.path).map_err(|_| ApplyError::Escape)?;
        }
    }
    let workspace = WorkspaceDir::open(project).map_err(map_capture)?;
    let host_leaves = workspace
        .collect_leaf_paths_excluding(expected_exclusions)
        .map_err(map_capture)?;
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
            CandidateEntryKind::Directory { .. } | CandidateEntryKind::Gitlink { .. } => {}
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

pub(crate) fn changed_paths(
    from: &CandidateRevisionArtefact,
    to: &CandidateRevisionArtefact,
) -> Vec<String> {
    let mut removals: Vec<_> = from
        .entries
        .iter()
        .filter(|entry| to.entries.iter().all(|item| item.path != entry.path))
        .map(|entry| entry.path.clone())
        .collect();
    removals.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
    let mut writes: Vec<_> = to
        .entries
        .iter()
        .filter(|entry| from.entries.iter().find(|item| item.path == entry.path) != Some(*entry))
        .map(|entry| entry.path.clone())
        .collect();
    writes.sort_by_key(|path| path.matches('/').count());
    removals.extend(writes);
    removals
}

fn apply_path(
    project: &Path,
    path: &str,
    target: &CandidateRevisionArtefact,
    store: &WorkflowArtefactRepository,
) -> Result<(), ApplyError> {
    let workspace = WorkspaceDir::open(project).map_err(map_capture)?;
    if let Some(entry) = target.entries.iter().find(|entry| entry.path == path) {
        write_entry(&workspace, store, entry)
    } else {
        if workspace.exists(path) {
            workspace.remove_leaf(path).map_err(map_capture)?;
            // Ordinary manifests contain directories as explicit journal operations.
            if !target.ordinary {
                prune_empty_parents(&workspace, path, target)?;
            }
        }
        Ok(())
    }
}

fn apply_changes(
    project: &Path,
    from: &CandidateRevisionArtefact,
    to: &CandidateRevisionArtefact,
    store: &WorkflowArtefactRepository,
) -> Result<(), ApplyError> {
    for path in changed_paths(from, to) {
        apply_path(project, &path, to, store)?;
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
            mode,
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
                workspace
                    .set_entry_mode(&entry.path, *mode)
                    .map_err(map_capture)?;
            } else {
                workspace
                    .write_file_mode(&entry.path, &data, *mode)
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
        CandidateEntryKind::Directory { mode } => {
            if workspace.exists(&entry.path) {
                if !matches!(
                    workspace.kind(&entry.path).map_err(map_capture)?,
                    WorkspaceKind::Directory
                ) {
                    workspace.remove_leaf(&entry.path).map_err(map_capture)?;
                    workspace
                        .create_directory_mode(&entry.path, *mode)
                        .map_err(map_capture)?;
                } else {
                    workspace
                        .set_entry_mode(&entry.path, *mode)
                        .map_err(map_capture)?;
                }
            } else {
                workspace
                    .create_directory_mode(&entry.path, *mode)
                    .map_err(map_capture)?;
            }
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
    let capture = || {
        if target.ordinary {
            super::candidate::CandidateCapture::capture_directory(
                project,
                &target.exclusions,
                store,
            )
        } else {
            super::candidate::CandidateCapture::capture_host(project, store)
        }
    };
    let first = capture().map_err(map_capture)?;
    let second = capture().map_err(map_capture)?;
    if first != *target || second != first {
        return Err(ApplyError::Drift);
    }
    Ok(())
}

fn validate_artefact(
    artefact: &CandidateRevisionArtefact,
    expected: ArtefactHash,
) -> Result<(), ApplyError> {
    super::candidate::validate_candidate_shape(artefact).map_err(|_| ApplyError::Integrity)?;
    let bytes = artefact
        .manifest_bytes()
        .map_err(|_| ApplyError::Integrity)?;
    let hash = artefact_hash_for(
        ArtefactKind::CandidateRevision,
        artefact.format_version,
        &bytes,
    );
    if hash != expected
        || hash_candidate(&artefact.entries, &artefact.exclusions) != artefact.candidate_hash
    {
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
