mod rig;

#[cfg(test)]
pub(crate) mod scripted;

use std::fmt;
use std::pin::Pin;

use futures_util::Stream;

pub(crate) const SYNTHETIC_BASE_URL: &str = "https://api.synthetic.new/openai/v1";

pub(crate) const MAXIMUM_API_KEY_BYTES: usize = 4_096;
pub(crate) const MAXIMUM_MODEL_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ProviderKind {
    Xai,
    OpenaiCodex,
    Synthetic,
}

impl ProviderKind {
    pub(crate) const ALL: [Self; 3] = [Self::Xai, Self::OpenaiCodex, Self::Synthetic];

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "xai" => Some(Self::Xai),
            "openai-codex" => Some(Self::OpenaiCodex),
            "synthetic" => Some(Self::Synthetic),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Xai => "xai",
            Self::OpenaiCodex => "openai-codex",
            Self::Synthetic => "synthetic",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Xai => "xAI (Grok)",
            Self::OpenaiCodex => "OpenAI Codex",
            Self::Synthetic => "Synthetic",
        }
    }

    pub(crate) fn default_model(self) -> &'static str {
        match self {
            Self::Xai => "grok-4.6",
            Self::OpenaiCodex => "gpt-5.1-codex",
            Self::Synthetic => "hf:moonshotai/Kimi-K3",
        }
    }
}

pub(crate) fn api_key_is_bounded(key: &str) -> bool {
    let key = key.trim();
    !key.is_empty()
        && key.len() <= MAXIMUM_API_KEY_BYTES
        && !key.chars().any(|character| character.is_control())
}

pub(crate) fn model_is_bounded(model: &str) -> bool {
    model.trim().len() <= MAXIMUM_MODEL_BYTES
        && !model.chars().any(|character| character.is_control())
}

pub(crate) fn resolve_model(kind: ProviderKind, model: &str) -> String {
    let model = model.trim();
    if model.is_empty() {
        kind.default_model().to_owned()
    } else {
        model.to_owned()
    }
}

#[derive(Clone)]
pub(crate) struct SecretString(String);

impl SecretString {
    pub(crate) fn new(value: String) -> Self {
        Self(value.trim().to_owned())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString(<redacted>)")
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ProviderConnection {
    pub(crate) kind: ProviderKind,
    pub(crate) api_key: SecretString,
    pub(crate) model: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChatTurn {
    pub(crate) role: Role,
    pub(crate) text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderError {
    Rejected,
    Unreachable,
    RateLimited {
        retry_after: Option<hypergraft::RetryAfter>,
    },
    EmptyReply,
    ReplyTooLong,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for ProviderError {}

impl ProviderError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Rejected => "That key was rejected. Check the provider and try again.",
            Self::Unreachable => "The provider could not be reached. Try again.",
            Self::RateLimited { .. } => {
                "The provider rate-limited this request. Try again shortly."
            }
            Self::EmptyReply => "The model returned an empty reply. Try again.",
            Self::ReplyTooLong => {
                "Circus truncated the model reply because it was too long. Try again."
            }
        }
    }

    pub(crate) fn patch_status(self) -> hypergraft::PatchStatus {
        match self {
            Self::Rejected => hypergraft::PatchStatus::Unauthorized,
            Self::RateLimited { retry_after } => hypergraft::PatchStatus::TooManyRequests(
                retry_after.unwrap_or_else(default_retry_after),
            ),
            Self::Unreachable | Self::EmptyReply | Self::ReplyTooLong => {
                hypergraft::PatchStatus::UnprocessableEntity
            }
        }
    }
}

fn default_retry_after() -> hypergraft::RetryAfter {
    hypergraft::RetryAfter::seconds(1).expect("one second is a valid retry interval")
}

// 400 and 422 mean the empty probe was authenticated. Never treat them as a rejected key.
pub(crate) fn classify_verify_status(
    status: u16,
    retry_after: Option<&str>,
) -> Result<(), ProviderError> {
    match status {
        200..=299 | 400 | 422 => Ok(()),
        other => Err(classify_failure_status(other, retry_after)),
    }
}

pub(crate) fn classify_failure_status(status: u16, retry_after: Option<&str>) -> ProviderError {
    match status {
        401 | 403 => ProviderError::Rejected,
        429 => ProviderError::RateLimited {
            retry_after: parse_retry_after(retry_after),
        },
        _ => ProviderError::Unreachable,
    }
}

fn parse_retry_after(value: Option<&str>) -> Option<hypergraft::RetryAfter> {
    let seconds = value?.trim().parse().ok()?;
    hypergraft::RetryAfter::seconds(seconds)
}

pub(crate) type TokenStream = Pin<Box<dyn Stream<Item = Result<String, ProviderError>> + Send>>;

pub(crate) enum ChatBackend {
    Rig,
    #[cfg(test)]
    Scripted(scripted::ScriptedBackend),
}

impl ChatBackend {
    pub(crate) async fn verify(
        &self,
        connection: &ProviderConnection,
    ) -> Result<(), ProviderError> {
        match self {
            Self::Rig => rig::verify(connection).await,
            #[cfg(test)]
            Self::Scripted(backend) => backend.verify(connection),
        }
    }

    pub(crate) async fn stream(
        &self,
        connection: &ProviderConnection,
        history: &[ChatTurn],
    ) -> Result<TokenStream, ProviderError> {
        match self {
            Self::Rig => rig::stream(connection, history).await,
            #[cfg(test)]
            Self::Scripted(backend) => backend.stream(connection, history),
        }
    }
}

#[cfg(test)]
mod tests;
