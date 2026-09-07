use std::path::Path;

use super::candidate::{
    CandidateEntry, CandidateEntryKind, CandidateRevisionArtefact, CaptureError, hash_candidate,
};
use super::confine::{WorkspaceDir, WorkspaceKind, split_relative};
use super::id::ObjectHash;
use super::store::WorkflowArtefactRepository;
use super::{ArtefactHash, artefact_hash_for};
use crate::workflows::definition::ArtefactKind;

pub(crate) struct CandidateMaterialise;

impl CandidateMaterialise {
    pub(crate) fn into_workspace(
        destination: &Path,
        artefact: &CandidateRevisionArtefact,
        expected_artefact_hash: ArtefactHash,
        store: &WorkflowArtefactRepository,
    ) -> Result<(), CaptureError> {
        validate_manifest(artefact, expected_artefact_hash)?;
        let workspace = WorkspaceDir::create_empty(destination)?;
        write_entries(&workspace, artefact, store)?;
        verify_workspace(&workspace, artefact, store)
    }
}

fn validate_manifest(
    artefact: &CandidateRevisionArtefact,
    expected_artefact_hash: ArtefactHash,
) -> Result<(), CaptureError> {
    if artefact.format_version != super::candidate::CANDIDATE_SCHEMA {
        return Err(CaptureError::SourceUnsupported);
    }
    if artefact.entries.len() > super::candidate::MAXIMUM_ENTRIES {
        return Err(CaptureError::SourceTooLarge);
    }
    super::candidate::validate_candidate_shape(artefact)?;
    let mut seen = Vec::new();
    let mut total_bytes = 0u64;
    for entry in &artefact.entries {
        split_relative(&entry.path)?;
        if entry.path.len() > super::candidate::MAXIMUM_PATH_BYTES {
            return Err(CaptureError::SourceTooLarge);
        }
        if seen.iter().any(|path: &String| path == &entry.path) {
            return Err(CaptureError::SourceUnsupported);
        }
        if let CandidateEntryKind::Regular { bytes, .. } = &entry.kind {
            total_bytes = total_bytes
                .checked_add(*bytes)
                .ok_or(CaptureError::SourceTooLarge)?;
            if total_bytes > super::candidate::MAXIMUM_TOTAL_BYTES {
                return Err(CaptureError::SourceTooLarge);
            }
        }
        seen.push(entry.path.clone());
    }
    if hash_candidate(&artefact.entries, &artefact.exclusions) != artefact.candidate_hash {
        return Err(CaptureError::ArtefactIntegrity);
    }
    let bytes = artefact
        .manifest_bytes()
        .map_err(|_| CaptureError::ArtefactWrite)?;
    let hash = artefact_hash_for(
        ArtefactKind::CandidateRevision,
        artefact.format_version,
        &bytes,
    );
    if hash != expected_artefact_hash {
        return Err(CaptureError::ArtefactIntegrity);
    }
    Ok(())
}

fn write_entries(
    workspace: &WorkspaceDir,
    artefact: &CandidateRevisionArtefact,
    store: &WorkflowArtefactRepository,
) -> Result<(), CaptureError> {
    for entry in &artefact.entries {
        match &entry.kind {
            CandidateEntryKind::Regular {
                mode, bytes, blob, ..
            } => {
                let data = match store.get(blob) {
                    Ok(data) => data,
                    Err(_) => return Err(CaptureError::ArtefactIntegrity),
                };
                if data.len() as u64 != *bytes || ObjectHash::of(&data) != *blob {
                    return Err(CaptureError::ArtefactIntegrity);
                }
                if *bytes > super::candidate::MAXIMUM_FILE_BYTES {
                    return Err(CaptureError::SourceTooLarge);
                }
                workspace.write_file_mode(&entry.path, &data, *mode)?;
            }
            CandidateEntryKind::Symlink { target, blob } => {
                let data = match store.get(blob) {
                    Ok(data) => data,
                    Err(_) => return Err(CaptureError::ArtefactIntegrity),
                };
                if data.as_slice() != target.as_bytes() || ObjectHash::of(&data) != *blob {
                    return Err(CaptureError::ArtefactIntegrity);
                }
                workspace.create_symlink(&entry.path, target)?;
            }
            CandidateEntryKind::Directory { .. } => {
                workspace.create_placeholder_dir(&entry.path)?;
            }
            CandidateEntryKind::Gitlink { .. } => {
                workspace.create_placeholder_dir(&entry.path)?;
            }
        }
    }
    for entry in artefact.entries.iter().rev() {
        if let CandidateEntryKind::Directory { mode } = &entry.kind {
            workspace.set_entry_mode(&entry.path, *mode)?;
        }
    }
    Ok(())
}

fn verify_workspace(
    workspace: &WorkspaceDir,
    artefact: &CandidateRevisionArtefact,
    store: &WorkflowArtefactRepository,
) -> Result<(), CaptureError> {
    let found = if artefact.ordinary {
        workspace.collect_paths_excluding(&[])?
    } else {
        workspace.collect_leaf_paths()?
    };
    let expected: Vec<String> = artefact
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    if found != expected {
        return Err(CaptureError::SourceChanged);
    }
    let mut reread = Vec::new();
    for entry in &artefact.entries {
        reread.push(reread_entry(workspace, store, entry)?);
    }
    if hash_candidate(&reread, &artefact.exclusions) != artefact.candidate_hash {
        return Err(CaptureError::SourceChanged);
    }
    Ok(())
}

fn reread_entry(
    workspace: &WorkspaceDir,
    store: &WorkflowArtefactRepository,
    entry: &CandidateEntry,
) -> Result<CandidateEntry, CaptureError> {
    match workspace.kind(&entry.path)? {
        WorkspaceKind::File { executable } => {
            let (bytes, opened_executable, size) = workspace.read_file(&entry.path)?;
            if opened_executable != executable {
                return Err(CaptureError::SourceChanged);
            }
            let blob = store.publish(&bytes).map_err(map_store)?;
            Ok(CandidateEntry {
                path: entry.path.clone(),
                kind: CandidateEntryKind::Regular {
                    executable,
                    mode: workspace.mode(&entry.path)?,
                    bytes: size,
                    blob,
                },
            })
        }
        WorkspaceKind::Symlink => {
            let target = workspace.read_link(&entry.path)?;
            let blob = store.publish(target.as_bytes()).map_err(map_store)?;
            Ok(CandidateEntry {
                path: entry.path.clone(),
                kind: CandidateEntryKind::Symlink { target, blob },
            })
        }
        WorkspaceKind::Directory => {
            let kind = match &entry.kind {
                CandidateEntryKind::Directory { .. } => CandidateEntryKind::Directory {
                    mode: workspace.mode(&entry.path)?,
                },
                CandidateEntryKind::Gitlink { commit } => {
                    if !workspace.dir_is_empty(&entry.path)? {
                        return Err(CaptureError::SourceChanged);
                    }
                    CandidateEntryKind::Gitlink {
                        commit: commit.clone(),
                    }
                }
                _ => return Err(CaptureError::SourceChanged),
            };
            Ok(CandidateEntry {
                path: entry.path.clone(),
                kind,
            })
        }
        WorkspaceKind::Other => Err(CaptureError::SourceUnsupported),
    }
}

fn map_store(error: super::store::ArtefactStoreError) -> CaptureError {
    match error {
        super::store::ArtefactStoreError::Integrity => CaptureError::ArtefactIntegrity,
        super::store::ArtefactStoreError::Persist | super::store::ArtefactStoreError::Missing => {
            CaptureError::ArtefactWrite
        }
    }
}

#[cfg(test)]
mod tests;
