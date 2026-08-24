use super::{ChatTurn, ProviderConnection, ProviderError, TokenStream};

#[derive(Clone)]
pub(crate) struct ScriptedBackend {
    pub(crate) verify_result: Result<(), ProviderError>,
    pub(crate) complete_result: Result<String, ProviderError>,
}

impl ScriptedBackend {
    pub(crate) fn accept() -> Self {
        Self {
            verify_result: Ok(()),
            complete_result: Ok("Hello from Circus.".to_owned()),
        }
    }

    pub(crate) fn verify(&self, _connection: &ProviderConnection) -> Result<(), ProviderError> {
        self.verify_result
    }

    pub(crate) fn stream(
        &self,
        _connection: &ProviderConnection,
        _history: &[ChatTurn],
    ) -> Result<TokenStream, ProviderError> {
        match self.complete_result.clone() {
            Ok(text) => Ok(Box::pin(futures_util::stream::iter(
                chunk_reply(&text).into_iter().map(Ok),
            ))),
            Err(error) => Err(error),
        }
    }
}

fn chunk_reply(text: &str) -> Vec<String> {
    if text.chars().count() <= 12 {
        return vec![text.to_owned()];
    }
    let mid = text.chars().count() / 2;
    let mut chars = text.chars();
    let first: String = chars.by_ref().take(mid).collect();
    let second: String = chars.collect();
    vec![first, second]
}
