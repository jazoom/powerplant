use serde::Deserialize;

use crate::providers::{ProviderKind, api_key_is_bounded};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectField {
    Provider,
    ApiKey,
}

impl ConnectField {
    pub(super) const fn id(self) -> &'static str {
        match self {
            Self::Provider => "connect-provider",
            Self::ApiKey => "connect-key",
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
}

/// API key has no `Debug` so submitted credentials cannot be logged.
#[derive(Deserialize)]
pub(super) struct ConnectForm {
    #[serde(default)]
    pub(super) provider: String,
    #[serde(default)]
    pub(super) api_key: String,
}

impl ConnectForm {
    pub(super) fn validate(&self) -> Result<ProviderKind, FieldError> {
        let Some(kind) = self.provider_kind() else {
            return Err(FieldError {
                field: ConnectField::Provider,
                message: "Choose a provider.",
            });
        };
        if !api_key_is_bounded(&self.api_key) {
            return Err(FieldError {
                field: ConnectField::ApiKey,
                message: "Enter an API key.",
            });
        }
        Ok(kind)
    }

    pub(super) fn provider_kind(&self) -> Option<ProviderKind> {
        ProviderKind::parse(self.provider.trim())
    }
}

#[derive(Deserialize)]
pub(super) struct ForgetForm {
    #[serde(default)]
    pub(super) provider: String,
}

impl ForgetForm {
    pub(super) fn provider_kind(&self) -> Option<ProviderKind> {
        ProviderKind::parse(self.provider.trim())
    }
}

#[cfg(test)]
mod tests;
