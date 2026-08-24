use serde::Deserialize;

use crate::providers::ProviderKind;

pub(super) const MAXIMUM_API_KEY_BYTES: usize = 4_096;
pub(super) const MAXIMUM_MODEL_BYTES: usize = 256;

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
