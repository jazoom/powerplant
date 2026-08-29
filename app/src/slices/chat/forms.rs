use serde::Deserialize;

use crate::providers::{ProviderKind, model_is_bounded, resolve_model};
use crate::sessions::JobId;
use crate::workflows::WorkflowSelection;

pub(super) const MAXIMUM_MESSAGE_BYTES: usize = 32_768;
pub(super) const MAXIMUM_CURSOR: u64 = 1_000_000;

#[derive(Deserialize)]
pub(super) struct ChatForm {
    #[serde(default)]
    pub(super) message: String,
    #[serde(default)]
    pub(super) workflow: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkflowTokenError {
    Absent,
    Malformed,
}

impl ChatForm {
    pub(super) fn is_bounded(&self) -> bool {
        let message = self.message.trim();
        !message.is_empty() && message.len() <= MAXIMUM_MESSAGE_BYTES
    }

    pub(super) fn workflow_selection(&self) -> Result<WorkflowSelection, WorkflowTokenError> {
        let token = self.workflow.trim();
        if token.is_empty() {
            return Err(WorkflowTokenError::Absent);
        }
        WorkflowSelection::parse(token).ok_or(WorkflowTokenError::Malformed)
    }
}

#[derive(Deserialize)]
pub(super) struct ModelForm {
    #[serde(default)]
    pub(super) provider: String,
    #[serde(default)]
    pub(super) model: String,
    #[serde(default)]
    pub(super) favourite: Option<String>,
    #[serde(default)]
    pub(super) provider_model_synced: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModelError {
    Provider,
    Model,
}

impl ModelForm {
    pub(super) fn wants_favourite_toggle(&self) -> bool {
        self.favourite.is_some()
    }

    pub(super) fn validate(
        &self,
        stored: impl Fn(ProviderKind) -> bool,
    ) -> Result<(ProviderKind, String), ModelError> {
        let kind = self.stored_provider(stored)?;
        if !model_is_bounded(&self.model) {
            return Err(ModelError::Model);
        }
        Ok((kind, resolve_model(kind, &self.model)))
    }

    pub(super) fn validate_favourite(
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
pub(super) struct ObserveQuery {
    #[serde(default)]
    pub(super) job: String,
    #[serde(default)]
    pub(super) cursor: String,
    #[serde(default)]
    pub(super) workflow: String,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum CursorError {
    Malformed,
    Excessive,
}

impl ObserveQuery {
    pub(super) fn job_id(&self) -> Option<JobId> {
        JobId::parse(self.job.trim())
    }

    pub(super) fn cursor(&self) -> Result<u64, CursorError> {
        parse_cursor(&self.cursor)
    }
}

pub(super) fn parse_cursor(raw: &str) -> Result<u64, CursorError> {
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
