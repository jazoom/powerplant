use std::path::{Path, PathBuf};

use super::id::{CandidateHash, ObjectHash};
use super::store::{ArtefactStoreError, WorkflowArtefactRepository};

pub(crate) const CANDIDATE_SCHEMA: u32 = 1;
pub(crate) const MAXIMUM_ENTRIES: usize = 100_000;
pub(crate) const MAXIMUM_PATH_BYTES: usize = 4_096;
pub(crate) const MAXIMUM_FILE_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAXIMUM_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub(crate) const MAXIMUM_MANIFEST_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAXIMUM_PREVIEW_PATHS: usize = 200;
pub(crate) const MAXIMUM_PREVIEW_BYTES: usize = 1024 * 1024;
const CANDIDATE_DOMAIN: &[u8] = b"powerplant.candidate.v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidateRevisionArtefact {
    pub(crate) format_version: u32,
    pub(crate) candidate_hash: CandidateHash,
    pub(crate) repository: RepositoryAnchor,
    pub(crate) entries: Vec<CandidateEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RepositoryAnchor {
    pub(crate) object_format: GitObjectFormat,
    pub(crate) head: Option<GitObjectId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GitObjectFormat {
    Sha1,
    Sha256,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GitObjectId(String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidateEntry {
    pub(crate) path: String,
    pub(crate) kind: CandidateEntryKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CandidateEntryKind {
    Regular {
        executable: bool,
        bytes: u64,
        blob: ObjectHash,
    },
    Symlink {
        target: String,
        blob: ObjectHash,
    },
    Gitlink {
        commit: GitObjectId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CaptureError {
    SourceNotGit,
    SourceRoot,
    SourceUnsupported,
    SourceTooLarge,
    SourceChanged,
    SourceRead,
    ArtefactWrite,
    ArtefactIntegrity,
}

impl CaptureError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::SourceNotGit => "The project is not a supported Git worktree.",
            Self::SourceRoot => "The project directory is not the Git worktree root.",
            Self::SourceUnsupported => "That project state is not supported for source capture.",
            Self::SourceTooLarge => "The project is too large to capture.",
            Self::SourceChanged => "The project changed during source capture.",
            Self::SourceRead => "Power Plant could not read the project files.",
            Self::ArtefactWrite => "Power Plant could not store the candidate. Try again.",
            Self::ArtefactIntegrity => "The stored candidate failed an integrity check.",
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
struct CandidateManifestFile {
    format_version: u32,
    candidate_hash: String,
    entries: Vec<CandidateEntryFile>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum CandidateEntryFile {
    Regular {
        path: String,
        executable: bool,
        bytes: u64,
        blob: String,
    },
    Symlink {
        path: String,
        target: String,
        blob: String,
    },
    Gitlink {
        path: String,
        commit: String,
    },
}

impl CandidateRevisionArtefact {
    pub(crate) fn manifest_bytes(&self) -> Result<Vec<u8>, CaptureError> {
        let file = CandidateManifestFile {
            format_version: self.format_version,
            candidate_hash: self.candidate_hash.as_str(),
            entries: self
                .entries
                .iter()
                .map(|entry| match &entry.kind {
                    CandidateEntryKind::Regular {
                        executable,
                        bytes,
                        blob,
                    } => CandidateEntryFile::Regular {
                        path: entry.path.clone(),
                        executable: *executable,
                        bytes: *bytes,
                        blob: blob.as_str(),
                    },
                    CandidateEntryKind::Symlink { target, blob } => CandidateEntryFile::Symlink {
                        path: entry.path.clone(),
                        target: target.clone(),
                        blob: blob.as_str(),
                    },
                    CandidateEntryKind::Gitlink { commit } => CandidateEntryFile::Gitlink {
                        path: entry.path.clone(),
                        commit: commit.0.clone(),
                    },
                })
                .collect(),
        };
        let bytes = serde_json::to_vec(&file).map_err(|_| CaptureError::ArtefactWrite)?;
        if bytes.len() > MAXIMUM_MANIFEST_BYTES {
            return Err(CaptureError::SourceTooLarge);
        }
        Ok(bytes)
    }

    pub(crate) fn from_manifest_bytes(bytes: &[u8]) -> Option<Vec<CandidateEntry>> {
        let file: CandidateManifestFile = serde_json::from_slice(bytes).ok()?;
        let mut entries = Vec::new();
        for entry in file.entries {
            entries.push(match entry {
                CandidateEntryFile::Regular {
                    path,
                    executable,
                    bytes,
                    blob,
                } => CandidateEntry {
                    path,
                    kind: CandidateEntryKind::Regular {
                        executable,
                        bytes,
                        blob: ObjectHash::parse(&blob)?,
                    },
                },
                CandidateEntryFile::Symlink { path, target, blob } => CandidateEntry {
                    path,
                    kind: CandidateEntryKind::Symlink {
                        target,
                        blob: ObjectHash::parse(&blob)?,
                    },
                },
                CandidateEntryFile::Gitlink { path, commit } => CandidateEntry {
                    path,
                    kind: CandidateEntryKind::Gitlink {
                        commit: GitObjectId(commit),
                    },
                },
            });
        }
        Some(entries)
    }
}

pub(crate) struct CandidateCapture;

impl CandidateCapture {
    pub(crate) fn capture_host(
        root: &Path,
        store: &WorkflowArtefactRepository,
    ) -> Result<CandidateRevisionArtefact, CaptureError> {
        let first = discover_host(root, store)?;
        let second = discover_host(root, store)?;
        if first != second {
            return Err(CaptureError::SourceChanged);
        }
        Ok(first)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CandidateChange {
    Added,
    Removed,
    Modified,
    ModeChanged,
    LinkChanged,
    GitlinkChanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidatePreview {
    pub(crate) changes: Vec<PreviewRow>,
    pub(crate) omitted_paths: usize,
    pub(crate) truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreviewRow {
    pub(crate) path: String,
    pub(crate) change: CandidateChange,
    pub(crate) detail: String,
}

pub(crate) fn hash_entries(entries: &[CandidateEntry]) -> CandidateHash {
    let mut encoded = Vec::from(CANDIDATE_DOMAIN);
    encoded.push(0);
    encoded.extend_from_slice(&CANDIDATE_SCHEMA.to_be_bytes());
    encoded.extend_from_slice(&(entries.len() as u64).to_be_bytes());
    for entry in entries {
        let path = entry.path.as_bytes();
        encoded.extend_from_slice(&(path.len() as u32).to_be_bytes());
        encoded.extend_from_slice(path);
        match &entry.kind {
            CandidateEntryKind::Regular {
                executable,
                bytes,
                blob,
            } => {
                encoded.push(1);
                encoded.push(if *executable { 1 } else { 0 });
                encoded.extend_from_slice(&bytes.to_be_bytes());
                encoded.extend_from_slice(blob.bytes());
            }
            CandidateEntryKind::Symlink { target, blob } => {
                encoded.push(2);
                let bytes = target.as_bytes();
                encoded.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                encoded.extend_from_slice(bytes);
                encoded.extend_from_slice(blob.bytes());
            }
            CandidateEntryKind::Gitlink { commit } => {
                encoded.push(3);
                let bytes = commit.0.as_bytes();
                encoded.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                encoded.extend_from_slice(bytes);
            }
        }
    }
    CandidateHash::of(&encoded)
}

pub(crate) fn compare_candidates(
    before: &[CandidateEntry],
    after: &[CandidateEntry],
) -> Vec<(String, CandidateChange)> {
    let mut changes = Vec::new();
    let mut before_index = 0;
    let mut after_index = 0;
    while before_index < before.len() || after_index < after.len() {
        match (before.get(before_index), after.get(after_index)) {
            (Some(left), Some(right)) if left.path == right.path => {
                if left.kind != right.kind {
                    changes.push((left.path.clone(), change_kind(&left.kind, &right.kind)));
                }
                before_index += 1;
                after_index += 1;
            }
            (Some(left), Some(right)) if left.path.as_bytes() < right.path.as_bytes() => {
                changes.push((left.path.clone(), CandidateChange::Removed));
                before_index += 1;
            }
            (Some(_), Some(right)) => {
                changes.push((right.path.clone(), CandidateChange::Added));
                after_index += 1;
            }
            (Some(left), None) => {
                changes.push((left.path.clone(), CandidateChange::Removed));
                before_index += 1;
            }
            (None, Some(right)) => {
                changes.push((right.path.clone(), CandidateChange::Added));
                after_index += 1;
            }
            (None, None) => break,
        }
    }
    changes
}

pub(crate) fn preview_changes(
    store: &WorkflowArtefactRepository,
    before: &[CandidateEntry],
    after: &[CandidateEntry],
) -> CandidatePreview {
    let changes = compare_candidates(before, after);
    let omitted = changes.len().saturating_sub(MAXIMUM_PREVIEW_PATHS);
    let mut rows = Vec::new();
    let mut rendered = 0usize;
    let mut truncated = omitted > 0;
    for (path, change) in changes.into_iter().take(MAXIMUM_PREVIEW_PATHS) {
        let detail = preview_detail(store, before, after, &path, change, &mut rendered);
        if rendered > MAXIMUM_PREVIEW_BYTES {
            truncated = true;
        }
        rows.push(PreviewRow {
            path,
            change,
            detail,
        });
        if truncated && rendered > MAXIMUM_PREVIEW_BYTES {
            break;
        }
    }
    CandidatePreview {
        changes: rows,
        omitted_paths: omitted,
        truncated,
    }
}

pub(crate) fn preview_plain(
    store: &WorkflowArtefactRepository,
    before: &[CandidateEntry],
    after: &[CandidateEntry],
) -> (String, bool) {
    let preview = preview_changes(store, before, after);
    let mut text = String::new();
    for row in &preview.changes {
        text.push_str(&row.path);
        text.push(' ');
        text.push_str(&row.detail);
        text.push('\n');
    }
    if preview.omitted_paths > 0 {
        text.push_str(&format!(
            "Omitted {} changed paths.\n",
            preview.omitted_paths
        ));
    }
    (text, preview.truncated)
}

fn preview_detail(
    store: &WorkflowArtefactRepository,
    before: &[CandidateEntry],
    after: &[CandidateEntry],
    path: &str,
    change: CandidateChange,
    rendered: &mut usize,
) -> String {
    let left = before.iter().find(|entry| entry.path == path);
    let right = after.iter().find(|entry| entry.path == path);
    match (
        left.map(|entry| &entry.kind),
        right.map(|entry| &entry.kind),
    ) {
        (
            Some(CandidateEntryKind::Regular { blob: old, .. }),
            Some(CandidateEntryKind::Regular {
                blob: new, bytes, ..
            }),
        ) if change == CandidateChange::Modified => {
            let Ok(old_bytes) = store.get(old) else {
                return "Binary or unread file".to_owned();
            };
            let Ok(new_bytes) = store.get(new) else {
                return "Binary or unread file".to_owned();
            };
            if !is_text(&old_bytes) || !is_text(&new_bytes) {
                return format!("Binary file ({bytes} bytes)");
            }
            let old_text = String::from_utf8_lossy(&old_bytes);
            let new_text = String::from_utf8_lossy(&new_bytes);
            let diff = similar::TextDiff::from_lines(old_text.as_ref(), new_text.as_ref());
            let mut unified = diff.unified_diff().header(path, path).to_string();
            if *rendered + unified.len() > MAXIMUM_PREVIEW_BYTES {
                unified.truncate(MAXIMUM_PREVIEW_BYTES.saturating_sub(*rendered));
                *rendered = MAXIMUM_PREVIEW_BYTES;
                return unified;
            }
            *rendered += unified.len();
            unified
        }
        (None, Some(CandidateEntryKind::Regular { bytes, .. })) => {
            format!("Added file ({bytes} bytes)")
        }
        (Some(CandidateEntryKind::Regular { bytes, .. }), None) => {
            format!("Removed file ({bytes} bytes)")
        }
        (_, Some(CandidateEntryKind::Symlink { target, .. })) => {
            format!("Symbolic link → {target}")
        }
        (Some(CandidateEntryKind::Symlink { target, .. }), None) => {
            format!("Removed symbolic link → {target}")
        }
        (_, Some(CandidateEntryKind::Gitlink { commit })) => {
            format!("Gitlink {}", commit.0)
        }
        _ => change_label(change).to_owned(),
    }
}

fn change_kind(before: &CandidateEntryKind, after: &CandidateEntryKind) -> CandidateChange {
    match (before, after) {
        (
            CandidateEntryKind::Regular {
                executable: left, ..
            },
            CandidateEntryKind::Regular {
                executable: right,
                blob,
                ..
            },
        ) => {
            let CandidateEntryKind::Regular { blob: old_blob, .. } = before else {
                return CandidateChange::Modified;
            };
            if old_blob != blob {
                CandidateChange::Modified
            } else if left != right {
                CandidateChange::ModeChanged
            } else {
                CandidateChange::Modified
            }
        }
        (CandidateEntryKind::Symlink { .. }, CandidateEntryKind::Symlink { .. }) => {
            CandidateChange::LinkChanged
        }
        (CandidateEntryKind::Gitlink { .. }, CandidateEntryKind::Gitlink { .. }) => {
            CandidateChange::GitlinkChanged
        }
        _ => CandidateChange::Modified,
    }
}

fn change_label(change: CandidateChange) -> &'static str {
    match change {
        CandidateChange::Added => "Added",
        CandidateChange::Removed => "Removed",
        CandidateChange::Modified => "Modified",
        CandidateChange::ModeChanged => "Mode changed",
        CandidateChange::LinkChanged => "Link changed",
        CandidateChange::GitlinkChanged => "Gitlink changed",
    }
}

fn is_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

fn discover_host(
    root: &Path,
    store: &WorkflowArtefactRepository,
) -> Result<CandidateRevisionArtefact, CaptureError> {
    let git_dir = root.join(".git");
    let meta = std::fs::symlink_metadata(&git_dir).map_err(|_| CaptureError::SourceNotGit)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(CaptureError::SourceUnsupported);
    }
    let toplevel = git_output(root, &["rev-parse", "--show-toplevel"])?;
    let toplevel = PathBuf::from(
        String::from_utf8(toplevel)
            .map_err(|_| CaptureError::SourceUnsupported)?
            .trim(),
    );
    if std::fs::canonicalize(root).ok().as_ref() != Some(&toplevel) && root != toplevel {
        return Err(CaptureError::SourceRoot);
    }
    let head = git_output(root, &["rev-parse", "HEAD"]).ok();
    let head = head.and_then(|bytes| {
        let text = String::from_utf8(bytes).ok()?;
        let id = text.trim();
        if id.is_empty() || id == "HEAD" {
            None
        } else {
            Some(GitObjectId(id.to_owned()))
        }
    });
    let format = git_output(root, &["rev-parse", "--show-object-format"])
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| value.trim().to_owned());
    let object_format = match format.as_deref() {
        Some("sha256") => GitObjectFormat::Sha256,
        _ => GitObjectFormat::Sha1,
    };
    let unmerged = git_output(root, &["ls-files", "-u", "-z"])?;
    if !unmerged.is_empty() {
        return Err(CaptureError::SourceUnsupported);
    }
    let staged = git_output(root, &["ls-files", "-z", "--stage"])?;
    let others = git_output(root, &["ls-files", "-z", "--others", "--exclude-standard"])?;
    let mut entries = Vec::new();
    let mut total = 0u64;
    parse_staged(root, store, &staged, &mut entries, &mut total)?;
    parse_untracked(root, store, &others, &mut entries, &mut total)?;
    entries.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    if entries.len() > MAXIMUM_ENTRIES {
        return Err(CaptureError::SourceTooLarge);
    }
    let hash = hash_entries(&entries);
    Ok(CandidateRevisionArtefact {
        format_version: CANDIDATE_SCHEMA,
        candidate_hash: hash,
        repository: RepositoryAnchor {
            object_format,
            head,
        },
        entries,
    })
}

fn parse_staged(
    root: &Path,
    store: &WorkflowArtefactRepository,
    bytes: &[u8],
    entries: &mut Vec<CandidateEntry>,
    total: &mut u64,
) -> Result<(), CaptureError> {
    for record in split_nul(bytes) {
        if record.is_empty() {
            continue;
        }
        let Some((meta, path)) = split_once_space_tab(record) else {
            return Err(CaptureError::SourceUnsupported);
        };
        let path = parse_path(path)?;
        let meta = std::str::from_utf8(meta).map_err(|_| CaptureError::SourceUnsupported)?;
        let mut parts = meta.split(' ');
        let mode = parts.next().ok_or(CaptureError::SourceUnsupported)?;
        let object = parts.next().ok_or(CaptureError::SourceUnsupported)?;
        let stage = parts.next().ok_or(CaptureError::SourceUnsupported)?;
        if stage != "0" {
            return Err(CaptureError::SourceUnsupported);
        }
        match mode {
            "100644" | "100755" => {
                let executable = mode == "100755";
                let (blob, bytes) = read_regular(root, store, &path)?;
                push_regular(entries, total, path, executable, bytes, blob)?;
            }
            "120000" => {
                let target = read_link(root, &path)?;
                let blob = store.publish(target.as_bytes()).map_err(map_store)?;
                entries.push(CandidateEntry {
                    path,
                    kind: CandidateEntryKind::Symlink { target, blob },
                });
            }
            "160000" => {
                entries.push(CandidateEntry {
                    path,
                    kind: CandidateEntryKind::Gitlink {
                        commit: GitObjectId(object.to_owned()),
                    },
                });
            }
            _ => return Err(CaptureError::SourceUnsupported),
        }
    }
    Ok(())
}

fn parse_untracked(
    root: &Path,
    store: &WorkflowArtefactRepository,
    bytes: &[u8],
    entries: &mut Vec<CandidateEntry>,
    total: &mut u64,
) -> Result<(), CaptureError> {
    for record in split_nul(bytes) {
        if record.is_empty() {
            continue;
        }
        let path = parse_path(record)?;
        if entries.iter().any(|entry| entry.path == path) {
            continue;
        }
        let full = root.join(&path);
        let meta = std::fs::symlink_metadata(&full).map_err(|_| CaptureError::SourceRead)?;
        if meta.file_type().is_symlink() {
            let target = read_link(root, &path)?;
            let blob = store.publish(target.as_bytes()).map_err(map_store)?;
            entries.push(CandidateEntry {
                path,
                kind: CandidateEntryKind::Symlink { target, blob },
            });
        } else if meta.is_file() {
            let executable = is_executable(&meta);
            let (blob, bytes) = read_regular(root, store, &path)?;
            push_regular(entries, total, path, executable, bytes, blob)?;
        } else if meta.is_dir() {
            continue;
        } else {
            return Err(CaptureError::SourceUnsupported);
        }
    }
    Ok(())
}

fn push_regular(
    entries: &mut Vec<CandidateEntry>,
    total: &mut u64,
    path: String,
    executable: bool,
    bytes: u64,
    blob: ObjectHash,
) -> Result<(), CaptureError> {
    *total = total.saturating_add(bytes);
    if *total > MAXIMUM_TOTAL_BYTES || bytes > MAXIMUM_FILE_BYTES {
        return Err(CaptureError::SourceTooLarge);
    }
    entries.push(CandidateEntry {
        path,
        kind: CandidateEntryKind::Regular {
            executable,
            bytes,
            blob,
        },
    });
    Ok(())
}

fn read_regular(
    root: &Path,
    store: &WorkflowArtefactRepository,
    path: &str,
) -> Result<(ObjectHash, u64), CaptureError> {
    let full = root.join(path);
    let before = std::fs::symlink_metadata(&full).map_err(|_| CaptureError::SourceRead)?;
    if before.file_type().is_symlink() {
        return Err(CaptureError::SourceUnsupported);
    }
    let bytes = std::fs::read(&full).map_err(|_| CaptureError::SourceRead)?;
    let after = std::fs::symlink_metadata(&full).map_err(|_| CaptureError::SourceRead)?;
    if before.len() != after.len() {
        return Err(CaptureError::SourceChanged);
    }
    if bytes.len() as u64 > MAXIMUM_FILE_BYTES {
        return Err(CaptureError::SourceTooLarge);
    }
    let blob = store.publish(&bytes).map_err(map_store)?;
    Ok((blob, bytes.len() as u64))
}

fn read_link(root: &Path, path: &str) -> Result<String, CaptureError> {
    let target = std::fs::read_link(root.join(path)).map_err(|_| CaptureError::SourceRead)?;
    let text = target
        .to_str()
        .ok_or(CaptureError::SourceUnsupported)?
        .to_owned();
    if text.as_bytes().contains(&0) {
        return Err(CaptureError::SourceUnsupported);
    }
    Ok(text)
}

fn parse_path(bytes: &[u8]) -> Result<String, CaptureError> {
    if bytes.len() > MAXIMUM_PATH_BYTES || bytes.contains(&0) {
        return Err(CaptureError::SourceTooLarge);
    }
    let path = std::str::from_utf8(bytes).map_err(|_| CaptureError::SourceUnsupported)?;
    if path.is_empty() || path.starts_with('/') || path.contains('\0') {
        return Err(CaptureError::SourceUnsupported);
    }
    Ok(path.to_owned())
}

fn split_nul(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split(|byte| *byte == 0).collect()
}

fn split_once_space_tab(record: &[u8]) -> Option<(&[u8], &[u8])> {
    let index = record.iter().position(|byte| *byte == b'\t')?;
    Some((&record[..index], &record[index + 1..]))
}

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, CaptureError> {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=",
            "--no-optional-locks",
        ])
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|_| CaptureError::SourceUnsupported)?;
    if !output.status.success() {
        return Err(CaptureError::SourceNotGit);
    }
    Ok(output.stdout)
}

fn is_executable(meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        false
    }
}

fn map_store(error: ArtefactStoreError) -> CaptureError {
    match error {
        ArtefactStoreError::Integrity => CaptureError::ArtefactIntegrity,
        ArtefactStoreError::Persist | ArtefactStoreError::Missing => CaptureError::ArtefactWrite,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_init(dir: &Path) {
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(dir)
                .status()
                .expect("git")
                .success()
        );
        let _ = std::process::Command::new("git")
            .args(["config", "user.email", "dev@example.com"])
            .current_dir(dir)
            .status();
        let _ = std::process::Command::new("git")
            .args(["config", "user.name", "Dev"])
            .current_dir(dir)
            .status();
    }

    #[test]
    fn path_kind_mode_and_blob_changes_create_different_hashes() {
        let left = vec![CandidateEntry {
            path: "a.txt".to_owned(),
            kind: CandidateEntryKind::Regular {
                executable: false,
                bytes: 1,
                blob: ObjectHash::of(b"a"),
            },
        }];
        let right = vec![CandidateEntry {
            path: "a.txt".to_owned(),
            kind: CandidateEntryKind::Regular {
                executable: true,
                bytes: 1,
                blob: ObjectHash::of(b"a"),
            },
        }];
        assert_ne!(hash_entries(&left), hash_entries(&right));
        let renamed = vec![CandidateEntry {
            path: "b.txt".to_owned(),
            kind: left[0].kind.clone(),
        }];
        assert_ne!(hash_entries(&left), hash_entries(&renamed));
    }

    #[test]
    fn capture_includes_tracked_and_permitted_untracked_files() {
        let dir = tempfile::tempdir().expect("dir");
        git_init(dir.path());
        std::fs::write(dir.path().join("tracked.txt"), b"one").expect("write");
        assert!(
            std::process::Command::new("git")
                .args(["add", "tracked.txt"])
                .current_dir(dir.path())
                .status()
                .expect("add")
                .success()
        );
        std::fs::write(dir.path().join("loose.txt"), b"two").expect("loose");
        std::fs::write(dir.path().join(".gitignore"), b"ignored.txt\n").expect("ignore");
        std::fs::write(dir.path().join("ignored.txt"), b"nope").expect("ignored");
        let store = WorkflowArtefactRepository::in_memory();
        let candidate = CandidateCapture::capture_host(dir.path(), &store).expect("capture");
        let paths: Vec<_> = candidate
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();
        assert!(paths.contains(&"tracked.txt"));
        assert!(paths.contains(&"loose.txt"));
        assert!(paths.contains(&".gitignore"));
        assert!(!paths.contains(&"ignored.txt"));
        let again = hash_entries(&candidate.entries);
        assert_eq!(again, candidate.candidate_hash);
    }

    #[test]
    fn comparison_reports_additions_and_mode_changes() {
        let blob = ObjectHash::of(b"x");
        let before = vec![CandidateEntry {
            path: "keep.txt".to_owned(),
            kind: CandidateEntryKind::Regular {
                executable: false,
                bytes: 1,
                blob,
            },
        }];
        let after = vec![
            CandidateEntry {
                path: "keep.txt".to_owned(),
                kind: CandidateEntryKind::Regular {
                    executable: true,
                    bytes: 1,
                    blob,
                },
            },
            CandidateEntry {
                path: "new.txt".to_owned(),
                kind: CandidateEntryKind::Regular {
                    executable: false,
                    bytes: 1,
                    blob,
                },
            },
        ];
        let changes = compare_candidates(&before, &after);
        assert!(
            changes
                .iter()
                .any(|item| item.1 == CandidateChange::ModeChanged)
        );
        assert!(changes.iter().any(|item| item.1 == CandidateChange::Added));
    }
}
