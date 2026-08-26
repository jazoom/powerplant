use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};

use futures_util::Stream;
use rig_core::completion::{Message, ToolDefinition};

use super::{ChatTurn, ModelEvent, ModelStream, ProviderConnection, ProviderError};

#[derive(Clone)]
enum Script {
    Chunks(Vec<Result<String, ProviderError>>),
    Rounds(Vec<Vec<Result<ModelEvent, ProviderError>>>),
    Hang {
        started: Option<Arc<AtomicBool>>,
        dropped: Option<Arc<AtomicBool>>,
    },
}

#[derive(Clone)]
pub(crate) struct ScriptedBackend {
    pub(crate) verify_result: Result<(), ProviderError>,
    models_result: Result<Vec<String>, ProviderError>,
    script: Result<Script, ProviderError>,
    round: Arc<AtomicUsize>,
    last_preamble: Arc<Mutex<Option<String>>>,
    last_tools: Arc<Mutex<Vec<String>>>,
}

impl ScriptedBackend {
    pub(crate) fn accept() -> Self {
        Self {
            verify_result: Ok(()),
            models_result: Ok(Vec::new()),
            script: Ok(Script::Chunks(
                chunk_reply("Hello from Power Plant.")
                    .into_iter()
                    .map(Ok)
                    .collect(),
            )),
            round: Arc::new(AtomicUsize::new(0)),
            last_preamble: Arc::new(Mutex::new(None)),
            last_tools: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn with_models(mut self, models: Vec<String>) -> Self {
        self.models_result = Ok(models);
        self
    }

    pub(crate) fn chunks<I>(chunks: I) -> Self
    where
        I: IntoIterator<Item = Result<String, ProviderError>>,
    {
        Self {
            verify_result: Ok(()),
            models_result: Ok(Vec::new()),
            script: Ok(Script::Chunks(chunks.into_iter().collect())),
            round: Arc::new(AtomicUsize::new(0)),
            last_preamble: Arc::new(Mutex::new(None)),
            last_tools: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn tool_then(name: &str, arguments: serde_json::Value, reply: &str) -> Self {
        Self {
            verify_result: Ok(()),
            models_result: Ok(Vec::new()),
            script: Ok(Script::Rounds(vec![
                vec![Ok(ModelEvent::ToolCall {
                    id: "call-1".to_owned(),
                    name: name.to_owned(),
                    arguments,
                })],
                chunk_reply(reply)
                    .into_iter()
                    .map(|text| Ok(ModelEvent::Text(text)))
                    .collect(),
            ])),
            round: Arc::new(AtomicUsize::new(0)),
            last_preamble: Arc::new(Mutex::new(None)),
            last_tools: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn hang() -> Self {
        Self {
            verify_result: Ok(()),
            models_result: Ok(Vec::new()),
            script: Ok(Script::Hang {
                started: None,
                dropped: None,
            }),
            round: Arc::new(AtomicUsize::new(0)),
            last_preamble: Arc::new(Mutex::new(None)),
            last_tools: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn hang_watched(started: Arc<AtomicBool>, dropped: Arc<AtomicBool>) -> Self {
        Self {
            verify_result: Ok(()),
            models_result: Ok(Vec::new()),
            script: Ok(Script::Hang {
                started: Some(started),
                dropped: Some(dropped),
            }),
            round: Arc::new(AtomicUsize::new(0)),
            last_preamble: Arc::new(Mutex::new(None)),
            last_tools: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn last_preamble(&self) -> Option<String> {
        self.last_preamble
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn last_tools(&self) -> Vec<String> {
        self.last_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn verify(&self, _connection: &ProviderConnection) -> Result<(), ProviderError> {
        self.verify_result.clone()
    }

    pub(crate) fn models(
        &self,
        _connection: &ProviderConnection,
    ) -> Result<Vec<String>, ProviderError> {
        self.models_result.clone()
    }

    pub(crate) fn stream_turn(
        &self,
        _connection: &ProviderConnection,
        _history: &[ChatTurn],
        _extra: &[Message],
        tools: &[ToolDefinition],
        preamble: &str,
    ) -> Result<ModelStream, ProviderError> {
        *self
            .last_preamble
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(preamble.to_owned());
        *self
            .last_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            tools.iter().map(|tool| tool.name.clone()).collect();
        match self.script.clone() {
            Ok(Script::Chunks(items)) => Ok(Box::pin(futures_util::stream::iter(
                items.into_iter().map(|item| item.map(ModelEvent::Text)),
            ))),
            Ok(Script::Rounds(rounds)) => {
                let index = self.round.fetch_add(1, Ordering::SeqCst);
                let items = rounds.into_iter().nth(index).unwrap_or_default();
                Ok(Box::pin(futures_util::stream::iter(items)))
            }
            Ok(Script::Hang { started, dropped }) => {
                Ok(Box::pin(ModelHangStream { started, dropped }))
            }
            Err(error) => Err(error),
        }
    }
}

struct ModelHangStream {
    started: Option<Arc<AtomicBool>>,
    dropped: Option<Arc<AtomicBool>>,
}

impl Stream for ModelHangStream {
    type Item = Result<ModelEvent, ProviderError>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(started) = &self.started {
            started.store(true, Ordering::SeqCst);
        }
        Poll::Pending
    }
}

impl Drop for ModelHangStream {
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
