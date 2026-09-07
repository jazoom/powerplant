use std::path::PathBuf;

use super::record::{
    AccessMode, AgentError, AgentRecord, GUEST_PROJECT, MAXIMUM_ALIAS_BYTES, NetworkAccess,
    canonical_directory, guest_path_for,
};
use super::tool_id::ToolId;
use crate::execution::GUEST_WORKSPACE;
use crate::projects::{ProjectId, ProjectRecord};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyGrant {
    pub(crate) alias: String,
    pub(crate) guest_path: String,
    pub(crate) host_path: PathBuf,
    pub(crate) access: AccessMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirectoryPolicy {
    grants: Vec<PolicyGrant>,
    primary_alias: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AuthorityOrigin {
    SavedAgent {
        agent_id: super::AgentId,
    },
    Conversation {
        conversation_id: crate::conversations::ConversationId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EffectiveAuthority {
    pub(crate) origin: AuthorityOrigin,
    pub(crate) revision: u32,
    pub(crate) project_id: ProjectId,
    pub(crate) project_revision: u32,
    pub(crate) grant_alias: String,
    pub(crate) grant_access: AccessMode,
    pub(crate) tools: Vec<ToolId>,
    pub(crate) network: NetworkAccess,
    pub(crate) policy: DirectoryPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthorityError {
    MissingGrant,
    Stale,
    Unavailable,
    Path,
    Alias,
    DuplicatePath,
    SecondaryWrite,
}

impl EffectiveAuthority {
    pub(crate) fn from_saved_agent(
        agent: &AgentRecord,
        project: &ProjectRecord,
        grant_alias: &str,
    ) -> Result<Self, AuthorityError> {
        let Some(grant) = agent
            .directories
            .iter()
            .find(|grant| grant.alias == grant_alias && grant.host_path == project.host_path)
        else {
            return Err(AuthorityError::MissingGrant);
        };
        if !project.host_path_is_available() {
            return Err(AuthorityError::Unavailable);
        }
        let policy = DirectoryPolicy::from_record_with_primary(agent, grant_alias);
        policy.confirm_hosts().map_err(|_| AuthorityError::Path)?;
        Ok(Self {
            origin: AuthorityOrigin::SavedAgent { agent_id: agent.id },
            revision: agent.revision,
            project_id: project.id,
            project_revision: project.revision,
            grant_alias: grant.alias.clone(),
            grant_access: grant.access,
            tools: agent.tools.clone(),
            network: agent.network.clone(),
            policy,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_conversation_with_context(
        conversation_id: crate::conversations::ConversationId,
        conversation_revision: u32,
        project: &ProjectRecord,
        project_revision: u32,
        access: AccessMode,
        network: NetworkAccess,
        secondary: Vec<PolicyGrant>,
        preset: Option<&AgentRecord>,
    ) -> Result<Self, AuthorityError> {
        if project.revision != project_revision {
            return Err(AuthorityError::Stale);
        }
        if !project.host_path_is_available() {
            return Err(AuthorityError::Unavailable);
        }
        let mut tools = if access.is_writable() {
            vec![ToolId::List, ToolId::Read, ToolId::Run, ToolId::Write]
        } else {
            vec![ToolId::List, ToolId::Read, ToolId::Run]
        };
        let mut grant_access = access;
        if let Some(preset) = preset {
            tools.retain(|tool| preset.tools.contains(tool));
            if let Some(access) = preset_directory_access(preset, &project.host_path)? {
                grant_access = min_access(grant_access, access);
            }
        }
        let mut grants = vec![PolicyGrant {
            alias: "project".to_owned(),
            guest_path: GUEST_PROJECT.to_owned(),
            host_path: project.host_path.clone(),
            access: grant_access,
        }];
        for mut directory in secondary {
            if !valid_secondary_alias(&directory.alias)
                || directory.alias == "project"
                || directory.guest_path != guest_path_for(&directory.alias, "project")
            {
                return Err(AuthorityError::Alias);
            }
            if directory.access.is_writable() {
                return Err(AuthorityError::SecondaryWrite);
            }
            if grants.iter().any(|grant| grant.alias == directory.alias) {
                return Err(AuthorityError::Alias);
            }
            if grants
                .iter()
                .any(|grant| grant.host_path == directory.host_path)
            {
                return Err(AuthorityError::DuplicatePath);
            }
            if let Some(preset) = preset
                && let Some(access) = preset_directory_access(preset, &directory.host_path)?
            {
                directory.access = min_access(directory.access, access);
            }
            directory.access = AccessMode::ReadOnly;
            grants.push(directory);
        }
        let policy = DirectoryPolicy::from_grants(grants, "project".to_owned());
        policy.confirm_hosts().map_err(|_| AuthorityError::Path)?;
        Ok(Self {
            origin: AuthorityOrigin::Conversation { conversation_id },
            revision: conversation_revision,
            project_id: project.id,
            project_revision,
            grant_alias: "project".to_owned(),
            grant_access,
            tools,
            network,
            policy,
        })
    }

    pub(crate) fn revalidate_project(&self, project: &ProjectRecord) -> Result<(), AuthorityError> {
        if self.project_id != project.id
            || self.project_revision != project.revision
            || !project.host_path_is_available()
            || self
                .policy
                .grants()
                .iter()
                .find(|grant| grant.alias == self.grant_alias)
                .is_none_or(|grant| grant.host_path != project.host_path)
        {
            return Err(AuthorityError::Stale);
        }
        self.policy
            .confirm_hosts()
            .map_err(|_| AuthorityError::Path)
    }

    pub(crate) fn directories(&self) -> impl Iterator<Item = (&str, AccessMode)> {
        self.policy
            .grants()
            .iter()
            .map(|grant| (grant.alias.as_str(), grant.access))
    }
}

fn min_access(left: AccessMode, right: AccessMode) -> AccessMode {
    if left.is_writable() && right.is_writable() {
        AccessMode::ReadWrite
    } else {
        AccessMode::ReadOnly
    }
}

fn valid_secondary_alias(alias: &str) -> bool {
    if alias.is_empty() || alias.len() > MAXIMUM_ALIAS_BYTES {
        return false;
    }
    let mut characters = alias.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        && characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn preset_directory_access(
    preset: &AgentRecord,
    host_path: &std::path::Path,
) -> Result<Option<AccessMode>, AuthorityError> {
    if preset.directories.is_empty() {
        Ok(None)
    } else {
        preset
            .directories
            .iter()
            .find(|directory| directory.host_path == host_path)
            .map(|directory| Some(directory.access))
            .ok_or(AuthorityError::MissingGrant)
    }
}

impl DirectoryPolicy {
    pub(crate) fn private_workspace() -> Self {
        Self {
            grants: Vec::new(),
            primary_alias: String::new(),
        }
    }

    pub(crate) fn is_private_workspace(&self) -> bool {
        self.grants.is_empty()
    }

    // The selected project grant is /project even when another grant is the saved primary.
    pub(crate) fn from_record_with_primary(record: &AgentRecord, primary_alias: &str) -> Self {
        let grants = record
            .directories
            .iter()
            .map(|grant| PolicyGrant {
                alias: grant.alias.clone(),
                guest_path: guest_path_for(&grant.alias, primary_alias),
                host_path: grant.host_path.clone(),
                access: grant.access,
            })
            .collect();
        Self {
            grants,
            primary_alias: primary_alias.to_owned(),
        }
    }

    pub(crate) fn from_grants(grants: Vec<PolicyGrant>, primary_alias: String) -> Self {
        Self {
            grants,
            primary_alias,
        }
    }

    pub(crate) fn grants(&self) -> &[PolicyGrant] {
        &self.grants
    }

    pub(crate) fn primary_alias(&self) -> &str {
        &self.primary_alias
    }

    pub(crate) fn primary_guest(&self) -> &str {
        self.grants
            .iter()
            .find(|grant| grant.alias == self.primary_alias)
            .map(|grant| grant.guest_path.as_str())
            .unwrap_or(if self.grants.is_empty() {
                GUEST_WORKSPACE
            } else {
                GUEST_PROJECT
            })
    }

    pub(crate) fn primary_access(&self) -> AccessMode {
        self.grants
            .iter()
            .find(|grant| grant.alias == self.primary_alias)
            .map(|grant| grant.access)
            .unwrap_or(if self.grants.is_empty() {
                AccessMode::ReadWrite
            } else {
                AccessMode::ReadOnly
            })
    }

    pub(crate) fn resolve(&self, raw: &str) -> Result<(String, AccessMode), &'static str> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok((self.primary_guest().to_owned(), self.primary_access()));
        }
        if raw.chars().any(char::is_control) {
            return Err("That path is not valid.");
        }
        let joined = if raw.starts_with('/') {
            raw.to_owned()
        } else {
            format!("{}/{raw}", self.primary_guest())
        };
        let normalised = normalise_absolute(&joined)?;
        if self.grants.is_empty() {
            return (normalised == GUEST_WORKSPACE
                || normalised.starts_with(&format!("{GUEST_WORKSPACE}/")))
            .then_some((normalised, AccessMode::ReadWrite))
            .ok_or("Stay inside the private workspace.");
        }
        self.grant_for(&normalised)
            .map(|grant| (normalised, grant.access))
            .ok_or("Stay inside a granted directory.")
    }

    pub(crate) fn guest_roots(&self) -> Vec<String> {
        if self.grants.is_empty() {
            return vec![GUEST_WORKSPACE.to_owned()];
        }
        self.grants
            .iter()
            .map(|grant| grant.guest_path.clone())
            .collect()
    }

    pub(crate) fn writable_roots(&self) -> Vec<String> {
        if self.grants.is_empty() {
            return vec![GUEST_WORKSPACE.to_owned()];
        }
        self.grants
            .iter()
            .filter(|grant| grant.access.is_writable())
            .map(|grant| grant.guest_path.clone())
            .collect()
    }

    pub(crate) fn confirm_hosts(&self) -> Result<(), AgentError> {
        for grant in &self.grants {
            let resolved = canonical_directory(&grant.host_path)?;
            if resolved != grant.host_path {
                return Err(AgentError::Path);
            }
        }
        Ok(())
    }

    fn grant_for(&self, path: &str) -> Option<&PolicyGrant> {
        self.grants.iter().find(|grant| {
            path == grant.guest_path || path.starts_with(&format!("{}/", grant.guest_path))
        })
    }
}

fn normalise_absolute(path: &str) -> Result<String, &'static str> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if parts.is_empty() {
                return Err("Stay inside a granted directory.");
            }
            parts.pop();
            continue;
        }
        parts.push(part);
    }
    Ok(format!("/{}", parts.join("/")))
}

#[cfg(test)]
mod tests;
