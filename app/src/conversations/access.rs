use crate::agents::{
    AccessMode, AgentStore, AuthorityError, EffectiveAuthority, NetworkAccess, PolicyGrant,
    guest_path_for,
};
use crate::projects::{ProjectId, ProjectRecord, ProjectStore};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationGrant {
    pub(crate) project_id: ProjectId,
    pub(crate) project_revision: u32,
    pub(crate) authority_revision: u32,
    pub(crate) access: AccessMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationAuthority {
    pub(crate) effective: EffectiveAuthority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConversationAccessError {
    MissingProject,
    MissingGrant,
    Stale,
    Unavailable,
    Path,
    Preset,
    Alias,
    DuplicatePath,
    AmbiguousTarget,
}

impl ConversationAccessError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::MissingProject => "That project is not in the catalogue.",
            Self::MissingGrant => "Grant project access before you inspect or change this project.",
            Self::Stale => "The project or preset changed. Reload the conversation.",
            Self::Unavailable => "The selected project is unavailable.",
            Self::Path => "A granted directory is no longer at the saved path.",
            Self::Preset => "The applied preset no longer permits this project.",
            Self::Alias => "The project context has a conflicting guest alias.",
            Self::DuplicatePath => "Two project grants use the same canonical directory.",
            Self::AmbiguousTarget => "Choose only one writable project for this conversation.",
        }
    }
}

impl std::fmt::Display for ConversationAccessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for ConversationAccessError {}

pub(crate) fn secondary_alias(project: ProjectId) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let bytes = crate::hex::decode::<16>(&project.as_hex()).expect("project identifier");
    let mut value = u128::from_be_bytes(bytes);
    let mut encoded = [b'a'; 26];
    for character in encoded.iter_mut().rev() {
        *character = ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    format!(
        "p{}",
        String::from_utf8(encoded.to_vec()).expect("base32 alias")
    )
}

pub(crate) fn intersect_network(
    selected: &NetworkAccess,
    ceiling: Option<&NetworkAccess>,
) -> NetworkAccess {
    let Some(ceiling) = ceiling else {
        return selected.clone();
    };
    match (selected, ceiling) {
        (NetworkAccess::None, _) | (_, NetworkAccess::None) => NetworkAccess::None,
        (NetworkAccess::Public, NetworkAccess::Public) => NetworkAccess::Public,
        (NetworkAccess::Public, NetworkAccess::Restricted(domains))
        | (NetworkAccess::Restricted(domains), NetworkAccess::Public) => {
            NetworkAccess::Restricted(domains.clone())
        }
        (NetworkAccess::Restricted(selected), NetworkAccess::Restricted(ceiling)) => {
            let mut domains = Vec::new();
            for left in selected {
                for right in ceiling {
                    if let Some(narrower) = narrower_domain(left, right)
                        && !domains.contains(&narrower)
                    {
                        domains.push(narrower);
                    }
                }
            }
            if domains.is_empty() {
                NetworkAccess::None
            } else {
                NetworkAccess::Restricted(domains)
            }
        }
    }
}

fn narrower_domain(left: &str, right: &str) -> Option<String> {
    if left == right || left.ends_with(&format!(".{right}")) {
        Some(left.to_owned())
    } else if right.ends_with(&format!(".{left}")) {
        Some(right.to_owned())
    } else {
        None
    }
}

pub(crate) fn resolve_authority(
    record: &crate::conversations::ConversationRecord,
    projects: &ProjectStore,
    agents: &AgentStore,
) -> Result<Option<ConversationAuthority>, ConversationAccessError> {
    let Some(project_id) = record.execution_target else {
        return Ok(None);
    };
    if record
        .grants
        .iter()
        .filter(|grant| grant.access.is_writable())
        .count()
        > 1
    {
        return Err(ConversationAccessError::AmbiguousTarget);
    }
    let grant = record
        .grants
        .iter()
        .find(|grant| grant.project_id == project_id)
        .ok_or(ConversationAccessError::MissingGrant)?;
    let project = projects
        .get(&project_id)
        .ok_or(ConversationAccessError::MissingProject)?;
    let preset = record
        .model
        .as_ref()
        .and_then(|model| model.preset.as_ref())
        .map(|preset| {
            let agent = agents
                .get(&preset.id)
                .ok_or(ConversationAccessError::Preset)?;
            if agent.revision != preset.revision {
                return Err(ConversationAccessError::Stale);
            }
            Ok(agent)
        })
        .transpose()?;
    let mut paths = vec![project.host_path.clone()];
    let mut aliases = vec!["project".to_owned()];
    let mut secondary = Vec::new();
    for other in &record.grants {
        if other.project_id == project_id {
            continue;
        }
        let secondary_project = projects
            .get(&other.project_id)
            .ok_or(ConversationAccessError::MissingProject)?;
        if secondary_project.revision != other.project_revision {
            return Err(ConversationAccessError::Stale);
        }
        if !secondary_project.host_path_is_available() {
            return Err(ConversationAccessError::Unavailable);
        }
        if paths.contains(&secondary_project.host_path) {
            return Err(ConversationAccessError::DuplicatePath);
        }
        let alias = secondary_alias(secondary_project.id);
        if !aliases.iter().all(|seen| seen != &alias) {
            return Err(ConversationAccessError::Alias);
        }
        paths.push(secondary_project.host_path.clone());
        aliases.push(alias.clone());
        secondary.push(PolicyGrant {
            alias: alias.clone(),
            guest_path: guest_path_for(&alias, "project"),
            host_path: secondary_project.host_path.clone(),
            access: AccessMode::ReadOnly,
        });
    }
    let network = intersect_network(&record.network, preset.as_ref().map(|agent| &agent.network));
    let authority = resolve_grant_with_context(
        grant,
        &project,
        record.id,
        grant.authority_revision,
        network,
        secondary,
        preset.as_ref(),
    )?;
    Ok(Some(authority))
}

fn resolve_grant_with_context(
    grant: &ConversationGrant,
    project: &ProjectRecord,
    conversation_id: crate::conversations::ConversationId,
    conversation_revision: u32,
    network: NetworkAccess,
    secondary: Vec<PolicyGrant>,
    preset: Option<&crate::agents::AgentRecord>,
) -> Result<ConversationAuthority, ConversationAccessError> {
    let effective = EffectiveAuthority::from_conversation_with_context(
        conversation_id,
        conversation_revision,
        project,
        grant.project_revision,
        grant.access,
        network,
        secondary,
        preset,
    )
    .map_err(map_authority_error)?;
    Ok(ConversationAuthority { effective })
}

fn map_authority_error(error: AuthorityError) -> ConversationAccessError {
    match error {
        AuthorityError::MissingGrant => ConversationAccessError::MissingGrant,
        AuthorityError::Stale => ConversationAccessError::Stale,
        AuthorityError::Unavailable => ConversationAccessError::Unavailable,
        AuthorityError::Path => ConversationAccessError::Path,
        AuthorityError::Alias => ConversationAccessError::Alias,
        AuthorityError::DuplicatePath => ConversationAccessError::DuplicatePath,
        AuthorityError::SecondaryWrite => ConversationAccessError::AmbiguousTarget,
    }
}

#[cfg(test)]
mod tests;
