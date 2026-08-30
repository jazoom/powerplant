use crate::agents::{AccessMode, AgentRecord, ToolId, guest_path_for};
use crate::providers::ProviderConnection;
use crate::sandbox::GuestAccess;
use crate::workflows::commands::{CommandSourceEffect, SystemCommandId};
#[cfg(test)]
use crate::workflows::definition::PRIMARY_SOURCE_ALIAS;
use crate::workflows::definition::{StepAction, StepDefinition};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttemptCapabilities {
    pub(crate) tools: Vec<ToolId>,
    pub(crate) directories: Vec<CapabilityDirectory>,
    pub(crate) source_location: PrimarySourceLocation,
    pub(crate) git_admin: AccessMode,
    pub(crate) network: NetworkCapability,
    pub(crate) secret: SecretPresence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrimarySourceLocation {
    AttemptWorkspace,
    UserProject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CapabilityDirectory {
    pub(crate) alias: String,
    pub(crate) guest_path: String,
    pub(crate) access: AccessMode,
    pub(crate) role: DirectoryRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DirectoryRole {
    PrimarySource,
    SecondaryContext,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NetworkCapability {
    None,
    ProviderHost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SecretPresence {
    None,
    ProviderPlaceholder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CapabilityError {
    Authority,
}

impl AttemptCapabilities {
    pub(crate) fn derive(
        step: &StepDefinition,
        agent: &AgentRecord,
        _connection: &ProviderConnection,
    ) -> Result<Self, CapabilityError> {
        match &step.action {
            StepAction::Agent(action) => {
                let ceiling_dirs: Vec<(&str, AccessMode)> = agent
                    .directories
                    .iter()
                    .map(|grant| (grant.alias.as_str(), grant.access))
                    .collect();
                let primary_access = action.candidate_authority.access();
                let primary_ceiling = agent
                    .directories
                    .iter()
                    .find(|grant| grant.alias == agent.primary_directory)
                    .map(|grant| grant.access);
                if primary_ceiling
                    .is_none_or(|access| primary_access.is_writable() && !access.is_writable())
                    || !action
                        .authority
                        .allowed_by(&agent.tools, ceiling_dirs.iter().copied())
                {
                    return Err(CapabilityError::Authority);
                }
                let mut directories = vec![CapabilityDirectory {
                    alias: agent.primary_directory.clone(),
                    guest_path: guest_path_for(&agent.primary_directory, &agent.primary_directory),
                    access: primary_access,
                    role: DirectoryRole::PrimarySource,
                }];
                for directory in &action.authority.directories {
                    let Some(grant) = agent
                        .directories
                        .iter()
                        .find(|grant| grant.alias == directory.alias)
                    else {
                        return Err(CapabilityError::Authority);
                    };
                    directories.push(CapabilityDirectory {
                        alias: directory.alias.clone(),
                        guest_path: guest_path_for(&directory.alias, &agent.primary_directory),
                        access: min_access(AccessMode::ReadOnly, grant.access),
                        role: DirectoryRole::SecondaryContext,
                    });
                }
                Ok(Self {
                    tools: action.authority.tools.clone(),
                    directories,
                    source_location: PrimarySourceLocation::AttemptWorkspace,
                    git_admin: AccessMode::ReadOnly,
                    network: NetworkCapability::ProviderHost,
                    secret: SecretPresence::ProviderPlaceholder,
                })
            }
            StepAction::SystemCommand(action) => Ok(commit_or_read_only(action.command, agent)),
            StepAction::HumanGate(_) => Err(CapabilityError::Authority),
        }
    }

    pub(crate) fn guest_access(&self, connection: &ProviderConnection) -> GuestAccess {
        if self.network != NetworkCapability::ProviderHost {
            return GuestAccess::default();
        }
        let mut access = GuestAccess::from_connection(connection);
        if self.secret != SecretPresence::ProviderPlaceholder {
            access.secret = None;
        }
        access
    }

    pub(crate) fn primary(&self) -> Option<&CapabilityDirectory> {
        self.directories
            .iter()
            .find(|directory| directory.role == DirectoryRole::PrimarySource)
    }

    pub(crate) fn tools_label(&self) -> String {
        if self.tools.is_empty() {
            return "none".to_owned();
        }
        self.tools
            .iter()
            .map(|tool| tool.label())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub(crate) fn primary_access_label(&self) -> &'static str {
        match self.primary().map(|directory| directory.access) {
            Some(AccessMode::ReadWrite) => "read-write",
            _ => "read-only",
        }
    }

    pub(crate) fn network_label(&self) -> &'static str {
        match self.network {
            NetworkCapability::None => "none",
            NetworkCapability::ProviderHost => "provider",
        }
    }
}

impl PrimarySourceLocation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AttemptWorkspace => "attempt-workspace",
            Self::UserProject => "user-project",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "attempt-workspace" => Some(Self::AttemptWorkspace),
            "user-project" => Some(Self::UserProject),
            _ => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::AttemptWorkspace => "Attempt workspace",
            Self::UserProject => "User project",
        }
    }
}

impl DirectoryRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PrimarySource => "primary-source",
            Self::SecondaryContext => "secondary-context",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "primary-source" => Some(Self::PrimarySource),
            "secondary-context" => Some(Self::SecondaryContext),
            _ => None,
        }
    }
}

impl NetworkCapability {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ProviderHost => "provider-host",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "provider-host" => Some(Self::ProviderHost),
            _ => None,
        }
    }
}

impl SecretPresence {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ProviderPlaceholder => "provider-placeholder",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "provider-placeholder" => Some(Self::ProviderPlaceholder),
            _ => None,
        }
    }
}

fn commit_or_read_only(command: SystemCommandId, agent: &AgentRecord) -> AttemptCapabilities {
    if command.contract().source_effect == CommandSourceEffect::Commit {
        AttemptCapabilities {
            tools: Vec::new(),
            directories: vec![CapabilityDirectory {
                alias: agent.primary_directory.clone(),
                guest_path: guest_path_for(&agent.primary_directory, &agent.primary_directory),
                access: AccessMode::ReadWrite,
                role: DirectoryRole::PrimarySource,
            }],
            source_location: PrimarySourceLocation::UserProject,
            git_admin: AccessMode::ReadWrite,
            network: NetworkCapability::None,
            secret: SecretPresence::None,
        }
    } else {
        AttemptCapabilities {
            tools: Vec::new(),
            directories: vec![primary_read_only(agent)],
            source_location: PrimarySourceLocation::AttemptWorkspace,
            git_admin: AccessMode::ReadOnly,
            network: NetworkCapability::None,
            secret: SecretPresence::None,
        }
    }
}

fn primary_read_only(agent: &AgentRecord) -> CapabilityDirectory {
    CapabilityDirectory {
        alias: agent.primary_directory.clone(),
        guest_path: guest_path_for(&agent.primary_directory, &agent.primary_directory),
        access: AccessMode::ReadOnly,
        role: DirectoryRole::PrimarySource,
    }
}

fn min_access(left: AccessMode, right: AccessMode) -> AccessMode {
    if left.is_writable() && right.is_writable() {
        AccessMode::ReadWrite
    } else {
        AccessMode::ReadOnly
    }
}

impl std::fmt::Display for CapabilityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl CapabilityError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Authority => "The pinned step authority exceeds the current agent ceiling.",
        }
    }
}

#[cfg(test)]
use crate::agents::GUEST_PROJECT;

#[cfg(test)]
pub(crate) fn test_agent_capabilities() -> AttemptCapabilities {
    AttemptCapabilities {
        tools: vec![ToolId::List],
        directories: vec![CapabilityDirectory {
            alias: PRIMARY_SOURCE_ALIAS.to_owned(),
            guest_path: GUEST_PROJECT.to_owned(),
            access: AccessMode::ReadWrite,
            role: DirectoryRole::PrimarySource,
        }],
        source_location: PrimarySourceLocation::AttemptWorkspace,
        git_admin: AccessMode::ReadOnly,
        network: NetworkCapability::ProviderHost,
        secret: SecretPresence::ProviderPlaceholder,
    }
}

#[cfg(test)]
pub(crate) fn test_command_capabilities() -> AttemptCapabilities {
    AttemptCapabilities {
        tools: Vec::new(),
        directories: vec![CapabilityDirectory {
            alias: PRIMARY_SOURCE_ALIAS.to_owned(),
            guest_path: GUEST_PROJECT.to_owned(),
            access: AccessMode::ReadOnly,
            role: DirectoryRole::PrimarySource,
        }],
        source_location: PrimarySourceLocation::AttemptWorkspace,
        git_admin: AccessMode::ReadOnly,
        network: NetworkCapability::None,
        secret: SecretPresence::None,
    }
}

#[cfg(test)]
mod tests;
