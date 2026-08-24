use serde::Deserialize;

use crate::providers::ProviderKind;

pub(super) const MAXIMUM_API_KEY_BYTES: usize = 4_096;
pub(super) const MAXIMUM_MODEL_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectField {
    Provider,
    ApiKey,
    Model,
}

impl ConnectField {
    pub(super) const fn id(self) -> &'static str {
        match self {
            Self::Provider => "connect-provider",
            Self::ApiKey => "connect-key",
            Self::Model => "connect-model",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FieldError {
    pub(super) field: ConnectField,
    pub(super) message: &'static str,
}

impl FieldError {
    pub(super) fn target_id(self) -> &'static str {
        self.field.id()
    }

    pub(super) fn is_provider(self) -> bool {
        self.field == ConnectField::Provider
    }

    pub(super) fn is_api_key(self) -> bool {
        self.field == ConnectField::ApiKey
    }

    pub(super) fn is_model(self) -> bool {
        self.field == ConnectField::Model
    }
}

/// API key has no `Debug` so submitted credentials cannot be logged.
#[derive(Deserialize)]
pub(super) struct ConnectForm {
    #[serde(default)]
    pub(super) provider: String,
    #[serde(default)]
    pub(super) api_key: String,
    #[serde(default)]
    pub(super) model: String,
}

impl ConnectForm {
    pub(super) fn validate(&self) -> Result<ProviderKind, FieldError> {
        let Some(kind) = self.provider_kind() else {
            return Err(FieldError {
                field: ConnectField::Provider,
                message: "Choose a provider.",
            });
        };
        if !self.api_key_is_bounded() {
            return Err(FieldError {
                field: ConnectField::ApiKey,
                message: "Enter an API key.",
            });
        }
        if !self.model_is_bounded() {
            return Err(FieldError {
                field: ConnectField::Model,
                message: "That model name is too long.",
            });
        }
        Ok(kind)
    }

    pub(super) fn provider_kind(&self) -> Option<ProviderKind> {
        ProviderKind::parse(self.provider.trim())
    }

    pub(super) fn api_key_is_bounded(&self) -> bool {
        let key = self.api_key.trim();
        !key.is_empty()
            && key.len() <= MAXIMUM_API_KEY_BYTES
            && !key.chars().any(|character| character.is_control())
    }

    pub(super) fn model_is_bounded(&self) -> bool {
        self.model.trim().len() <= MAXIMUM_MODEL_BYTES
            && !self.model.chars().any(|character| character.is_control())
    }

    pub(super) fn resolved_model(&self, kind: ProviderKind) -> String {
        let model = self.model.trim();
        if model.is_empty() {
            kind.default_model().to_owned()
        } else {
            model.to_owned()
        }
    }
}

#[cfg(test)]
mod tests;
