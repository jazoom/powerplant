use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use futures_util::Stream;

use super::{ChatTurn, ProviderConnection, ProviderError, TokenStream};

#[derive(Clone)]
enum Script {
    Chunks(Vec<Result<String, ProviderError>>),
    Hang {
        started: Option<Arc<AtomicBool>>,
        dropped: Option<Arc<AtomicBool>>,
    },
}

#[derive(Clone)]
pub(crate) struct ScriptedBackend {
    pub(crate) verify_result: Result<(), ProviderError>,
    script: Result<Script, ProviderError>,
}

impl ScriptedBackend {
    pub(crate) fn accept() -> Self {
        Self {
            verify_result: Ok(()),
            script: Ok(Script::Chunks(
                chunk_reply("Hello from Circus.")
                    .into_iter()
                    .map(Ok)
                    .collect(),
            )),
        }
    }

    pub(crate) fn chunks<I>(chunks: I) -> Self
    where
        I: IntoIterator<Item = Result<String, ProviderError>>,
    {
        Self {
            verify_result: Ok(()),
            script: Ok(Script::Chunks(chunks.into_iter().collect())),
        }
    }

    pub(crate) fn hang() -> Self {
        Self {
            verify_result: Ok(()),
            script: Ok(Script::Hang {
                started: None,
                dropped: None,
            }),
        }
    }

    pub(crate) fn hang_watched(started: Arc<AtomicBool>, dropped: Arc<AtomicBool>) -> Self {
        Self {
            verify_result: Ok(()),
            script: Ok(Script::Hang {
                started: Some(started),
                dropped: Some(dropped),
            }),
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
        match self.script.clone() {
            Ok(Script::Chunks(items)) => Ok(Box::pin(futures_util::stream::iter(items))),
            Ok(Script::Hang { started, dropped }) => Ok(Box::pin(HangStream { started, dropped })),
            Err(error) => Err(error),
        }
    }
}

struct HangStream {
    started: Option<Arc<AtomicBool>>,
    dropped: Option<Arc<AtomicBool>>,
}

impl Stream for HangStream {
    type Item = Result<String, ProviderError>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(started) = &self.started {
            started.store(true, Ordering::SeqCst);
        }
        Poll::Pending
    }
}

impl Drop for HangStream {
    fn drop(&mut self) {
        if let Some(dropped) = &self.dropped {
            dropped.store(true, Ordering::SeqCst);
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
