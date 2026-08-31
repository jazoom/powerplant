use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::id::ProjectId;

pub(crate) const MAXIMUM_PROJECTS: usize = 64;
pub(crate) const MAXIMUM_NAME_BYTES: usize = 80;
pub(crate) const MAXIMUM_PATH_BYTES: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectRecord {
    pub(crate) id: ProjectId,
    pub(crate) revision: u32,
    pub(crate) name: String,
    pub(crate) host_path: PathBuf,
    pub(crate) created_at_ms: u64,
}

impl ProjectRecord {
    pub(crate) fn host_path_is_available(&self) -> bool {
        let Ok(canonical) = std::fs::canonicalize(&self.host_path) else {
            return false;
        };
        canonical == self.host_path
            && std::fs::metadata(&canonical).is_ok_and(|metadata| metadata.is_dir())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectError {
    Random,
    Persist,
    Corrupt,
    Full,
    Name,
    Path,
    NotADirectory,
    Worktree,
    DuplicatePath,
    Missing,
    Conflict,
}

impl ProjectError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Random => "Power Plant could not create a project identifier. Try again.",
            Self::Persist => "Power Plant could not store the project. Try again.",
            Self::Corrupt => "The project catalogue is unreadable.",
            Self::Full => "The project catalogue is full.",
            Self::Name => "Enter a name of 1 to 80 bytes without control characters.",
            Self::Path => {
                "Enter an absolute directory path of at most 4,096 bytes without control characters."
            }
            Self::NotADirectory => "That path is not an accessible directory.",
            Self::Worktree => "The project is not a supported Git worktree.",
            Self::DuplicatePath => "A project already uses that directory.",
            Self::Missing => "That project is not in the catalogue.",
            Self::Conflict => "Reload the project and try again.",
        }
    }
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for ProjectError {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(super) struct ProjectFile {
    pub(super) id: String,
    pub(super) revision: u32,
    pub(super) name: String,
    pub(super) host_path: String,
    pub(super) created_at_ms: u64,
}

impl ProjectRecord {
    pub(super) fn create(
        id: ProjectId,
        name: String,
        host_path: PathBuf,
        created_at_ms: u64,
    ) -> Result<Self, ProjectError> {
        Ok(Self {
            id,
            revision: 1,
            name: normalise_name(&name)?,
            host_path: stored_host_path(&host_path)?,
            created_at_ms,
        })
    }

    pub(super) fn from_file(file: ProjectFile) -> Result<Self, ProjectError> {
        let id = ProjectId::parse(&file.id).ok_or(ProjectError::Corrupt)?;
        if file.revision == 0 {
            return Err(ProjectError::Corrupt);
        }
        let name = normalise_name(&file.name).map_err(|_| ProjectError::Corrupt)?;
        let host_path =
            stored_host_path(Path::new(&file.host_path)).map_err(|_| ProjectError::Corrupt)?;
        Ok(Self {
            id,
            revision: file.revision,
            name,
            host_path,
            created_at_ms: file.created_at_ms,
        })
    }

    pub(super) fn to_file(&self) -> ProjectFile {
        ProjectFile {
            id: self.id.as_hex(),
            revision: self.revision,
            name: self.name.clone(),
            host_path: self.host_path.to_string_lossy().into_owned(),
            created_at_ms: self.created_at_ms,
        }
    }
}

pub(crate) fn submitted_name(raw: &str) -> Result<String, ProjectError> {
    normalise_name(raw)
}

pub(crate) fn submitted_host_path(path: &Path) -> Result<PathBuf, ProjectError> {
    stored_host_path(path)?;
    let metadata = std::fs::metadata(path).map_err(|_| ProjectError::NotADirectory)?;
    if !metadata.is_dir() {
        return Err(ProjectError::NotADirectory);
    }
    let canonical = std::fs::canonicalize(path).map_err(|_| ProjectError::NotADirectory)?;
    stored_host_path(&canonical)?;
    crate::workflows::artefacts::inspect_supported_worktree(&canonical)
        .map_err(|_| ProjectError::Worktree)?;
    Ok(canonical)
}

pub(super) fn normalise_name(raw: &str) -> Result<String, ProjectError> {
    if raw.chars().any(char::is_control) {
        return Err(ProjectError::Name);
    }
    let name = raw.trim();
    if name.is_empty() || name.len() > MAXIMUM_NAME_BYTES {
        return Err(ProjectError::Name);
    }
    Ok(name.to_owned())
}

pub(super) fn stored_host_path(path: &Path) -> Result<PathBuf, ProjectError> {
    let Some(raw) = path.to_str() else {
        return Err(ProjectError::Path);
    };
    if raw.len() > MAXIMUM_PATH_BYTES || raw.chars().any(char::is_control) || !path.is_absolute() {
        return Err(ProjectError::Path);
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests;
