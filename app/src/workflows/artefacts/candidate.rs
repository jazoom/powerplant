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
const GIT_ADMIN_DOMAIN: &[u8] = b"powerplant.git-admin.v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidateRevisionArtefact {
    pub(crate) format_version: u32,
    pub(crate) candidate_hash: CandidateHash,
    pub(crate) repository: RepositoryAnchor,
    pub(crate) git_admin: GitAdministrativeFingerprint,
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
pub(crate) struct GitObjectId(pub(crate) String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GitAdministrativeFingerprint(String);

impl GitAdministrativeFingerprint {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        ObjectHash::parse(value).map(|hash| Self(hash.as_str()))
    }
}

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
    repository: RepositoryAnchorFile,
    git_admin: String,
    entries: Vec<CandidateEntryFile>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
struct RepositoryAnchorFile {
    object_format: String,
    head: Option<String>,
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
            repository: RepositoryAnchorFile {
                object_format: match self.repository.object_format {
                    GitObjectFormat::Sha1 => "sha1".to_owned(),
                    GitObjectFormat::Sha256 => "sha256".to_owned(),
                },
                head: self.repository.head.as_ref().map(|id| id.0.clone()),
            },
            git_admin: self.git_admin.as_str().to_owned(),
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

    pub(crate) fn from_manifest_bytes(bytes: &[u8]) -> Option<Self> {
        let file: CandidateManifestFile = serde_json::from_slice(bytes).ok()?;
        if file.format_version != CANDIDATE_SCHEMA {
            return None;
        }
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
        let object_format = match file.repository.object_format.as_str() {
            "sha1" => GitObjectFormat::Sha1,
            "sha256" => GitObjectFormat::Sha256,
            _ => return None,
        };
        let artefact = Self {
            format_version: file.format_version,
            candidate_hash: CandidateHash::parse(&file.candidate_hash)?,
            repository: RepositoryAnchor {
                object_format,
                head: file.repository.head.map(GitObjectId),
            },
            git_admin: GitAdministrativeFingerprint::parse(&file.git_admin)?,
            entries,
        };
        if hash_entries(&artefact.entries) != artefact.candidate_hash {
            return None;
        }
        Some(artefact)
    }
}

pub(crate) struct CandidateCapture;

impl CandidateCapture {
    pub(crate) fn capture_host(
        root: &Path,
        store: &WorkflowArtefactRepository,
    ) -> Result<CandidateRevisionArtefact, CaptureError> {
        let git_dir = root.join(".git");
        capture_twice(root, &git_dir, None, store)
    }

    pub(crate) fn capture_worktree(
        worktree: &Path,
        git_dir: &Path,
        expected_git: &GitAdministrativeFingerprint,
        store: &WorkflowArtefactRepository,
    ) -> Result<CandidateRevisionArtefact, CaptureError> {
        capture_twice(worktree, git_dir, Some(expected_git), store)
    }
}

fn capture_twice(
    worktree: &Path,
    git_dir: &Path,
    expected_git: Option<&GitAdministrativeFingerprint>,
    store: &WorkflowArtefactRepository,
) -> Result<CandidateRevisionArtefact, CaptureError> {
    let first = discover(worktree, git_dir, expected_git, store)?;
    let second = discover(worktree, git_dir, Some(&first.git_admin), store)?;
    if first != second {
        return Err(CaptureError::SourceChanged);
    }
    Ok(first)
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

pub(crate) fn inspect_supported_worktree(worktree: &Path) -> Result<(), CaptureError> {
    inspect_worktree(worktree, &worktree.join(".git"), None).map(|_| ())
}

fn inspect_worktree(
    worktree: &Path,
    git_dir: &Path,
    expected_git: Option<&GitAdministrativeFingerprint>,
) -> Result<(GitAdministrativeFingerprint, super::confine::WorkspaceDir), CaptureError> {
    let git_meta = std::fs::symlink_metadata(git_dir).map_err(|_| CaptureError::SourceNotGit)?;
    if git_meta.file_type().is_symlink() || !git_meta.is_dir() {
        return Err(CaptureError::SourceUnsupported);
    }
    let fingerprint = git_fingerprint(git_dir)?;
    if let Some(expected) = expected_git
        && expected != &fingerprint
    {
        return Err(CaptureError::SourceChanged);
    }
    let workspace = super::confine::WorkspaceDir::open(worktree)?;
    let unmerged = git_output(git_dir, worktree, &["ls-files", "-u", "-z"])?;
    if !unmerged.is_empty() {
        return Err(CaptureError::SourceUnsupported);
    }
    Ok((fingerprint, workspace))
}

fn discover(
    worktree: &Path,
    git_dir: &Path,
    expected_git: Option<&GitAdministrativeFingerprint>,
    store: &WorkflowArtefactRepository,
) -> Result<CandidateRevisionArtefact, CaptureError> {
    let (fingerprint, workspace) = inspect_worktree(worktree, git_dir, expected_git)?;
    let object_format = match git_text(git_dir, worktree, &["rev-parse", "--show-object-format"])
        .ok()
        .as_deref()
    {
        Some("sha256") => GitObjectFormat::Sha256,
        _ => GitObjectFormat::Sha1,
    };
    let head = git_text(git_dir, worktree, &["rev-parse", "HEAD"])
        .ok()
        .and_then(|text| {
            let id = text.trim();
            if id.is_empty() || id == "HEAD" {
                None
            } else {
                Some(GitObjectId(id.to_owned()))
            }
        });
    let staged = git_output(git_dir, worktree, &["ls-files", "-z", "--stage"])?;
    let others = git_output(
        git_dir,
        worktree,
        &["ls-files", "-z", "--others", "--exclude-standard"],
    )?;
    let mut entries = Vec::new();
    let mut total = 0u64;
    parse_staged(&workspace, store, &staged, &mut entries, &mut total)?;
    parse_untracked(&workspace, store, &others, &mut entries, &mut total)?;
    entries.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    if entries.len() > MAXIMUM_ENTRIES {
        return Err(CaptureError::SourceTooLarge);
    }
    let after = git_fingerprint(git_dir)?;
    if after != fingerprint {
        return Err(CaptureError::SourceChanged);
    }
    Ok(CandidateRevisionArtefact {
        format_version: CANDIDATE_SCHEMA,
        candidate_hash: hash_entries(&entries),
        repository: RepositoryAnchor {
            object_format,
            head,
        },
        git_admin: fingerprint,
        entries,
    })
}

fn parse_staged(
    workspace: &super::confine::WorkspaceDir,
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
        if path_under_symlink(workspace, &path) {
            continue;
        }
        let meta = std::str::from_utf8(meta).map_err(|_| CaptureError::SourceUnsupported)?;
        let mut parts = meta.split(' ');
        let mode = parts.next().ok_or(CaptureError::SourceUnsupported)?;
        let object = parts.next().ok_or(CaptureError::SourceUnsupported)?;
        let stage = parts.next().ok_or(CaptureError::SourceUnsupported)?;
        if stage != "0" {
            return Err(CaptureError::SourceUnsupported);
        }
        if let Some(entry) = capture_tracked(workspace, store, &path, mode, object, total)? {
            entries.push(entry);
        }
    }
    Ok(())
}

fn capture_tracked(
    workspace: &super::confine::WorkspaceDir,
    store: &WorkflowArtefactRepository,
    path: &str,
    index_mode: &str,
    index_object: &str,
    total: &mut u64,
) -> Result<Option<CandidateEntry>, CaptureError> {
    use super::confine::WorkspaceKind;
    if !workspace.exists(path) {
        return Ok(None);
    }
    match workspace.kind(path)? {
        WorkspaceKind::File { executable } => {
            if index_mode == "160000" {
                return Err(CaptureError::SourceUnsupported);
            }
            let (bytes, opened_executable, size) = workspace.read_file(path)?;
            if opened_executable != executable {
                return Err(CaptureError::SourceChanged);
            }
            let blob = store.publish(&bytes).map_err(map_store)?;
            *total = total.saturating_add(size);
            if *total > MAXIMUM_TOTAL_BYTES || size > MAXIMUM_FILE_BYTES {
                return Err(CaptureError::SourceTooLarge);
            }
            Ok(Some(CandidateEntry {
                path: path.to_owned(),
                kind: CandidateEntryKind::Regular {
                    executable,
                    bytes: size,
                    blob,
                },
            }))
        }
        WorkspaceKind::Symlink => {
            if index_mode == "160000" {
                return Err(CaptureError::SourceUnsupported);
            }
            let target = workspace.read_link(path)?;
            let blob = store.publish(target.as_bytes()).map_err(map_store)?;
            Ok(Some(CandidateEntry {
                path: path.to_owned(),
                kind: CandidateEntryKind::Symlink { target, blob },
            }))
        }
        WorkspaceKind::Directory => {
            if index_mode != "160000" {
                return Ok(None);
            }
            if !workspace.dir_is_empty(path)? {
                return Err(CaptureError::SourceUnsupported);
            }
            Ok(Some(CandidateEntry {
                path: path.to_owned(),
                kind: CandidateEntryKind::Gitlink {
                    commit: GitObjectId(index_object.to_owned()),
                },
            }))
        }
        WorkspaceKind::Other => Err(CaptureError::SourceUnsupported),
    }
}

fn parse_untracked(
    workspace: &super::confine::WorkspaceDir,
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
        if path_under_symlink(workspace, &path) {
            continue;
        }
        if !workspace.exists(&path) {
            continue;
        }
        match workspace.kind(&path)? {
            super::confine::WorkspaceKind::Symlink => {
                let target = workspace.read_link(&path)?;
                let blob = store.publish(target.as_bytes()).map_err(map_store)?;
                entries.push(CandidateEntry {
                    path,
                    kind: CandidateEntryKind::Symlink { target, blob },
                });
            }
            super::confine::WorkspaceKind::File { executable } => {
                let (bytes, opened_executable, size) = match workspace.read_file(&path) {
                    Ok(value) => value,
                    Err(CaptureError::SourceRead)
                        if workspace.kind(&path).ok()
                            == Some(super::confine::WorkspaceKind::Symlink) =>
                    {
                        let target = workspace.read_link(&path)?;
                        let blob = store.publish(target.as_bytes()).map_err(map_store)?;
                        entries.push(CandidateEntry {
                            path,
                            kind: CandidateEntryKind::Symlink { target, blob },
                        });
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                if opened_executable != executable {
                    return Err(CaptureError::SourceChanged);
                }
                let blob = store.publish(&bytes).map_err(map_store)?;
                *total = total.saturating_add(size);
                if *total > MAXIMUM_TOTAL_BYTES || size > MAXIMUM_FILE_BYTES {
                    return Err(CaptureError::SourceTooLarge);
                }
                entries.push(CandidateEntry {
                    path,
                    kind: CandidateEntryKind::Regular {
                        executable,
                        bytes: size,
                        blob,
                    },
                });
            }
            super::confine::WorkspaceKind::Directory => continue,
            super::confine::WorkspaceKind::Other => return Err(CaptureError::SourceUnsupported),
        }
    }
    Ok(())
}

fn path_under_symlink(workspace: &super::confine::WorkspaceDir, path: &str) -> bool {
    let mut current = PathBuf::new();
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 {
        return false;
    }
    for part in &parts[..parts.len() - 1] {
        current.push(part);
        let relative = current.to_string_lossy();
        if workspace.exists(relative.as_ref())
            && workspace.kind(relative.as_ref()).ok()
                == Some(super::confine::WorkspaceKind::Symlink)
        {
            return true;
        }
    }
    false
}

pub(crate) fn git_fingerprint(
    git_dir: &Path,
) -> Result<GitAdministrativeFingerprint, CaptureError> {
    let index = read_optional_git_file(git_dir, "index")?.unwrap_or_default();
    let head = read_git_file(git_dir, "HEAD")?;
    let config = read_git_file(git_dir, "config")?;
    reject_config_includes(&config)?;
    if git_dir.join("config.worktree").exists() {
        return Err(CaptureError::SourceUnsupported);
    }
    let exclude = read_optional_git_file(git_dir, "info/exclude")?;
    let resolved = std::process::Command::new("git")
        .args(["--git-dir"])
        .arg(git_dir)
        .args([
            "--no-optional-locks",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "alias.rev-parse=",
            "rev-parse",
            "HEAD",
        ])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
        .unwrap_or_default();
    let format = std::process::Command::new("git")
        .args(["--git-dir"])
        .arg(git_dir)
        .args([
            "--no-optional-locks",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "alias.rev-parse=",
            "rev-parse",
            "--show-object-format",
        ])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
        .unwrap_or_default();
    let mut encoded = Vec::from(GIT_ADMIN_DOMAIN);
    encoded.push(0);
    push_len_bytes(&mut encoded, &index);
    push_len_bytes(&mut encoded, &head);
    push_len_bytes(&mut encoded, &resolved);
    push_len_bytes(&mut encoded, &format);
    push_len_bytes(&mut encoded, &config);
    match exclude {
        Some(bytes) => {
            encoded.push(1);
            push_len_bytes(&mut encoded, &bytes);
        }
        None => encoded.push(0),
    }
    Ok(GitAdministrativeFingerprint(
        ObjectHash::of(&encoded).as_str(),
    ))
}

fn push_len_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn read_git_file(git_dir: &Path, relative: &str) -> Result<Vec<u8>, CaptureError> {
    read_optional_git_file(git_dir, relative)?.ok_or(CaptureError::SourceRead)
}

fn read_optional_git_file(git_dir: &Path, relative: &str) -> Result<Option<Vec<u8>>, CaptureError> {
    let path = git_dir.join(relative);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(CaptureError::SourceRead),
    };
    if meta.file_type().is_symlink() {
        return Err(CaptureError::SourceUnsupported);
    }
    Ok(Some(
        std::fs::read(&path).map_err(|_| CaptureError::SourceRead)?,
    ))
}

fn reject_config_includes(bytes: &[u8]) -> Result<(), CaptureError> {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    if text.contains("[include]") || text.contains("[includeif") || text.contains("worktreeconfig")
    {
        return Err(CaptureError::SourceUnsupported);
    }
    Ok(())
}

fn git_text(git_dir: &Path, worktree: &Path, args: &[&str]) -> Result<String, CaptureError> {
    let bytes = git_output(git_dir, worktree, args)?;
    String::from_utf8(bytes)
        .map(|text| text.trim().to_owned())
        .map_err(|_| CaptureError::SourceUnsupported)
}

fn git_output(git_dir: &Path, worktree: &Path, args: &[&str]) -> Result<Vec<u8>, CaptureError> {
    let output = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(worktree)
        .args([
            "--no-optional-locks",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=",
            "-c",
            "core.excludesFile=/dev/null",
            "-c",
            "alias.rev-parse=",
            "-c",
            "alias.ls-files=",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|_| CaptureError::SourceUnsupported)?;
    if !output.status.success() {
        return Err(CaptureError::SourceNotGit);
    }
    Ok(output.stdout)
}

fn parse_path(bytes: &[u8]) -> Result<String, CaptureError> {
    if bytes.len() > MAXIMUM_PATH_BYTES || bytes.contains(&0) {
        return Err(CaptureError::SourceTooLarge);
    }
    let path = std::str::from_utf8(bytes).map_err(|_| CaptureError::SourceUnsupported)?;
    if path.is_empty() || path.starts_with('/') || path.contains('\0') {
        return Err(CaptureError::SourceUnsupported);
    }
    if path == ".git" || path.starts_with(".git/") || path.split('/').any(|part| part == "..") {
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

fn map_store(error: ArtefactStoreError) -> CaptureError {
    match error {
        ArtefactStoreError::Integrity => CaptureError::ArtefactIntegrity,
        ArtefactStoreError::Persist | ArtefactStoreError::Missing => CaptureError::ArtefactWrite,
    }
}

#[cfg(test)]
mod tests;
