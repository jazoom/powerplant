use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::agents::{
    AccessMode, AgentDraft, AgentError, DirectoryGrant, MAXIMUM_GRANTS, MAXIMUM_INSTRUCTION_BYTES,
    MAXIMUM_NAME_BYTES, MAXIMUM_PATH_BYTES, ToolId,
};

#[derive(Deserialize)]
pub(super) struct AgentForm {
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) instructions: String,
    #[serde(default)]
    pub(super) primary: String,
    #[serde(flatten)]
    extra: HashMap<String, String>,
}

impl AgentForm {
    pub(super) fn draft(&self) -> Result<AgentDraft, AgentError> {
        if self.name.len() > MAXIMUM_NAME_BYTES {
            return Err(AgentError::Name);
        }
        if self.instructions.len() > MAXIMUM_INSTRUCTION_BYTES {
            return Err(AgentError::Instructions);
        }
        Ok(AgentDraft {
            name: self.name.clone(),
            instructions: self.instructions.clone(),
            tools: self.tools(),
            directories: self.directories()?,
            primary_directory: self.primary.clone(),
        })
    }

    fn tools(&self) -> Vec<ToolId> {
        ToolId::ALL
            .into_iter()
            .filter(|tool| {
                self.extra
                    .get(&format!("tool_{}", tool.as_str()))
                    .is_some_and(|value| value == "on" || value == "true" || value == "1")
            })
            .collect()
    }

    fn directories(&self) -> Result<Vec<DirectoryGrant>, AgentError> {
        let mut grants = Vec::new();
        for index in 0..MAXIMUM_GRANTS {
            let alias = self
                .extra
                .get(&format!("alias_{index}"))
                .map(String::as_str)
                .unwrap_or("")
                .trim();
            let path = self
                .extra
                .get(&format!("path_{index}"))
                .map(String::as_str)
                .unwrap_or("")
                .trim();
            let access = self
                .extra
                .get(&format!("access_{index}"))
                .map(String::as_str)
                .unwrap_or("read-write")
                .trim();
            if alias.is_empty() && path.is_empty() {
                continue;
            }
            if path.len() > MAXIMUM_PATH_BYTES || path.chars().any(char::is_control) {
                return Err(AgentError::Path);
            }
            let Some(access) = AccessMode::parse(access) else {
                return Err(AgentError::Path);
            };
            grants.push(DirectoryGrant {
                alias: alias.to_owned(),
                host_path: PathBuf::from(path),
                access,
            });
        }
        Ok(grants)
    }
}

#[derive(Deserialize)]
pub(super) struct OrphanForm {
    #[serde(default)]
    pub(super) name: String,
}

#[cfg(test)]
mod tests;
