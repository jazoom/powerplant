use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::projects::{ProjectError, submitted_host_path, submitted_name};

pub(super) const REVISION_MESSAGE: &str = "Reload the project and try again.";

#[derive(Deserialize)]
pub(super) struct ProjectForm {
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) path: String,
    #[serde(default)]
    pub(super) revision: String,
}

impl ProjectForm {
    pub(super) fn submitted_name(&self) -> Result<String, ProjectError> {
        submitted_name(&self.name)
    }

    pub(super) fn submitted_path(&self) -> Result<PathBuf, ProjectError> {
        submitted_host_path(Path::new(&self.path))
    }

    pub(super) fn revision(&self) -> Result<u32, &'static str> {
        parse_revision(&self.revision)
    }
}

fn parse_revision(raw: &str) -> Result<u32, &'static str> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "0" || (raw.len() > 1 && raw.starts_with('0')) {
        return Err(REVISION_MESSAGE);
    }
    if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(REVISION_MESSAGE);
    }
    raw.parse().map_err(|_| REVISION_MESSAGE)
}

#[cfg(test)]
mod tests;
