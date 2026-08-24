mod rig;

#[cfg(test)]
pub(crate) mod scripted;

use std::fmt;
use std::pin::Pin;

use futures_util::Stream;

pub(crate) const SYNTHETIC_BASE_URL: &str = "https://api.synthetic.new/openai/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone)]
pub(crate) struct SecretString(String);

impl SecretString {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
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
    EmptyReply,
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
            Self::EmptyReply => "The model returned an empty reply. Try again.",
        }
    }
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

    pub(crate) async fn complete(
        &self,
        connection: &ProviderConnection,
        history: &[ChatTurn],
    ) -> Result<String, ProviderError> {
        use futures_util::StreamExt;
        let mut stream = self.stream(connection, history).await?;
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            text.push_str(&chunk?);
        }
        if text.trim().is_empty() {
            Err(ProviderError::EmptyReply)
        } else {
            Ok(text)
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
