use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::agents::{AccessMode, AgentId};
use crate::projects::{ProjectError, submitted_host_path, submitted_name};

pub(super) const REVISION_MESSAGE: &str = "Reload the project and try again.";
pub(super) const AGENT_REVISION_MESSAGE: &str = "Reload the agent and try again.";
pub(super) const AGENT_MESSAGE: &str = "That agent does not exist.";
pub(super) const ACCESS_MESSAGE: &str = "Choose read-only or read-write access.";
pub(super) const CHOOSER_BUSY: &str = "Another project folder chooser is open.";

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NewProjectQuery {
    #[serde(default)]
    entry: Option<NewProjectEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum NewProjectEntry {
    Manual,
}

impl NewProjectQuery {
    pub(super) fn is_manual(&self) -> bool {
        matches!(self.entry, Some(NewProjectEntry::Manual))
    }
}

#[derive(Deserialize)]
pub(super) struct ProjectForm {
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) path: String,
    #[serde(default)]
    pub(super) entry: String,
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

    pub(super) fn is_manual(&self) -> bool {
        self.entry == "manual"
    }

    pub(super) fn revision(&self) -> Result<u32, &'static str> {
        parse_revision(&self.revision).map_err(|()| REVISION_MESSAGE)
    }
}

#[derive(Deserialize)]
pub(super) struct GrantForm {
    #[serde(default)]
    pub(super) agent_id: String,
    #[serde(default)]
    pub(super) revision: String,
    #[serde(default)]
    pub(super) alias: String,
    #[serde(default)]
    pub(super) access: String,
}

impl GrantForm {
    pub(super) fn agent_id(&self) -> Result<AgentId, &'static str> {
        AgentId::parse(self.agent_id.trim()).ok_or(AGENT_MESSAGE)
    }

    pub(super) fn revision(&self) -> Result<u32, &'static str> {
        parse_revision(&self.revision).map_err(|()| AGENT_REVISION_MESSAGE)
    }

    pub(super) fn access(&self) -> Result<AccessMode, &'static str> {
        AccessMode::parse(&self.access).ok_or(ACCESS_MESSAGE)
    }

    pub(super) fn alias(&self) -> String {
        self.alias.clone()
    }
}

fn parse_revision(raw: &str) -> Result<u32, ()> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "0" || (raw.len() > 1 && raw.starts_with('0')) {
        return Err(());
    }
    if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    raw.parse().map_err(|_| ())
}

#[cfg(test)]
mod tests;
