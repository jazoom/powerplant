use serde::Deserialize;

use crate::providers::{ProviderKind, ThinkingEffort, model_is_bounded, resolve_model};
use crate::sessions::JobId;
use crate::workflows::WorkflowSelection;

pub(crate) const MAXIMUM_MESSAGE_BYTES: usize = 32_768;
pub(crate) const MAXIMUM_CURSOR: u64 = 1_000_000;

#[derive(Deserialize)]
pub(crate) struct ChatForm {
    #[serde(default)]
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) workflow: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeskMode {
    Quick,
    Configured,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeskModeError {
    Absent,
    Malformed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkflowTokenError {
    Absent,
    Malformed,
}

impl ChatForm {
    pub(crate) fn is_bounded(&self) -> bool {
        let message = self.message.trim();
        !message.is_empty() && message.len() <= MAXIMUM_MESSAGE_BYTES
    }

    pub(crate) fn mode(&self) -> Result<DeskMode, DeskModeError> {
        match self.mode.trim() {
            "" => Err(DeskModeError::Absent),
            "quick" => Ok(DeskMode::Quick),
            "configured" => Ok(DeskMode::Configured),
            _ => Err(DeskModeError::Malformed),
        }
    }

    pub(crate) fn workflow_selection(
        &self,
        query_workflow: &str,
    ) -> Result<WorkflowSelection, WorkflowTokenError> {
        let submitted = self.workflow.trim();
        let query = query_workflow.trim();
        if !submitted.is_empty() && !query.is_empty() && submitted != query {
            return Err(WorkflowTokenError::Malformed);
        }
        let token = if submitted.is_empty() {
            query
        } else {
            submitted
        };
        if token.is_empty() {
            return Err(WorkflowTokenError::Absent);
        }
        WorkflowSelection::parse(token).ok_or(WorkflowTokenError::Malformed)
    }
}

#[derive(Deserialize)]
pub(crate) struct ModelForm {
    #[serde(default)]
    pub(crate) provider: String,
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) favourite: Option<String>,
    #[serde(default)]
    pub(crate) thinking: String,
    #[serde(default)]
    pub(crate) provider_model_synced: bool,
    #[serde(default)]
    pub(crate) project: String,
    #[serde(default)]
    pub(crate) agent: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModelError {
    Provider,
    Model,
    Thinking,
}

impl ModelForm {
    pub(crate) fn wants_favourite_toggle(&self) -> bool {
        self.favourite.is_some()
    }

    pub(crate) fn validate(
        &self,
        stored: impl Fn(ProviderKind) -> bool,
    ) -> Result<(ProviderKind, String, Option<ThinkingEffort>), ModelError> {
        let kind = self.stored_provider(stored)?;
        if !model_is_bounded(&self.model) {
            return Err(ModelError::Model);
        }
        let thinking = match self.thinking.trim() {
            "" | "default" => None,
            value => Some(ThinkingEffort::new(value.to_owned()).ok_or(ModelError::Thinking)?),
        };
        Ok((kind, resolve_model(kind, &self.model), thinking))
    }

    pub(crate) fn validate_favourite(
        &self,
        stored: impl Fn(ProviderKind) -> bool,
    ) -> Result<(ProviderKind, String), ModelError> {
        let kind = self.stored_provider(stored)?;
        let model = self.favourite.as_deref().unwrap_or_default().trim();
        if model.is_empty() || !model_is_bounded(model) {
            return Err(ModelError::Model);
        }
        Ok((kind, model.to_owned()))
    }

    fn stored_provider(
        &self,
        stored: impl Fn(ProviderKind) -> bool,
    ) -> Result<ProviderKind, ModelError> {
        let Some(kind) = ProviderKind::parse(self.provider.trim()) else {
            return Err(ModelError::Provider);
        };
        if !stored(kind) {
            return Err(ModelError::Provider);
        }
        Ok(kind)
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ObserveQuery {
    #[serde(default)]
    pub(crate) job: String,
    #[serde(default)]
    pub(crate) cursor: String,
    #[serde(default)]
    pub(crate) workflow: String,
    #[serde(default)]
    pub(crate) sandbox: String,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CursorError {
    Malformed,
    Excessive,
}

impl ObserveQuery {
    pub(crate) fn job_id(&self) -> Option<JobId> {
        JobId::parse(self.job.trim())
    }

    pub(crate) fn cursor(&self) -> Result<u64, CursorError> {
        parse_cursor(&self.cursor)
    }
}

pub(crate) fn parse_cursor(raw: &str) -> Result<u64, CursorError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(0);
    }
    if raw.len() > 7 {
        return Err(CursorError::Excessive);
    }
    if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CursorError::Malformed);
    }
    let value: u64 = raw.parse().map_err(|_| CursorError::Malformed)?;
    if value > MAXIMUM_CURSOR {
        return Err(CursorError::Excessive);
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
