use crate::agents::{AgentStore, AuthorityError, EffectiveAuthority};
use crate::projects::{ProjectId, ProjectRecord, ProjectStore};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationGrant {
    pub(crate) project_id: ProjectId,
    pub(crate) project_revision: u32,
    pub(crate) authority_revision: u32,
    pub(crate) access: crate::agents::AccessMode,
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
}

impl ConversationAccessError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::MissingProject => "That project is not in the catalogue.",
            Self::MissingGrant => "Grant read-only access before you inspect this project.",
            Self::Stale => "The project or preset changed. Reload the conversation.",
            Self::Unavailable => "The selected project is unavailable.",
            Self::Path => "A granted directory is no longer at the saved path.",
            Self::Preset => "The applied preset no longer permits this project.",
        }
    }
}

impl std::fmt::Display for ConversationAccessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for ConversationAccessError {}

pub(crate) fn resolve_authority(
    record: &crate::conversations::ConversationRecord,
    projects: &ProjectStore,
    agents: &AgentStore,
) -> Result<Option<ConversationAuthority>, ConversationAccessError> {
    let Some(project_id) = record.execution_target else {
        return Ok(None);
    };
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
    let authority = resolve_grant(
        grant,
        &project,
        record.id,
        grant.authority_revision,
        preset.as_ref(),
    )?;
    Ok(Some(authority))
}

fn resolve_grant(
    grant: &ConversationGrant,
    project: &ProjectRecord,
    conversation_id: crate::conversations::ConversationId,
    conversation_revision: u32,
    preset: Option<&crate::agents::AgentRecord>,
) -> Result<ConversationAuthority, ConversationAccessError> {
    let effective = EffectiveAuthority::from_conversation(
        conversation_id,
        conversation_revision,
        project,
        grant.project_revision,
        grant.access,
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
    }
}

#[cfg(test)]
mod tests;
