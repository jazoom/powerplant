use serde::Deserialize;

use crate::providers::{ProviderKind, ThinkingEffort, model_is_bounded, resolve_model};

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

#[cfg(test)]
mod tests;
