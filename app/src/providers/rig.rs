use std::time::Duration;

use futures_util::StreamExt;
use rig_core::{
    client::CompletionClient,
    completion::{CompletionError, CompletionModel, Message},
    providers::{openai, xai},
    streaming::StreamedAssistantContent,
};

use super::{
    ChatTurn, MAXIMUM_LISTED_MODELS, ProviderConnection, ProviderError, ProviderKind, Role,
    SYNTHETIC_BASE_URL, TokenStream, classify_failure_status, classify_verify_status,
    model_is_bounded,
};

const XAI_BASE_URL: &str = "https://api.x.ai/v1";
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

const MAXIMUM_MODEL_LIST_BYTES: usize = 1_048_576;

const PREAMBLE: &str = "You are Circus, a local coding agent. Help the user write, explain and review code. Be direct.";

pub(super) const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

fn base_url(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Xai => XAI_BASE_URL,
        ProviderKind::OpenaiCodex => OPENAI_BASE_URL,
        ProviderKind::Synthetic => SYNTHETIC_BASE_URL,
    }
}

pub(super) async fn verify(connection: &ProviderConnection) -> Result<(), ProviderError> {
    verify_at(
        base_url(connection.kind),
        connection.api_key.expose(),
        VERIFY_TIMEOUT,
    )
    .await
}

// An empty `{}` probe still authenticates and does not spend tokens.
// Read only the status and Retry-After. Never read the provider body.
pub(super) async fn verify_at(
    base_url: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<(), ProviderError> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let request = reqwest::Client::new()
        .post(url)
        .bearer_auth(api_key)
        .header("content-type", "application/json")
        .body("{}")
        .send();
    let response = match tokio::time::timeout(timeout, request).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) | Err(_) => return Err(ProviderError::Unreachable),
    };
    classify_verify_status(
        response.status().as_u16(),
        retry_after_value(response.headers()),
    )
}

// Read only ids from the model list. Never log the provider body.
pub(super) async fn models(connection: &ProviderConnection) -> Result<Vec<String>, ProviderError> {
    models_at(
        base_url(connection.kind),
        connection.api_key.expose(),
        VERIFY_TIMEOUT,
    )
    .await
}

pub(super) async fn models_at(
    base_url: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<Vec<String>, ProviderError> {
    let operation = async {
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let response = reqwest::Client::new()
            .get(url)
            .bearer_auth(api_key)
            .send()
            .await
            .map_err(|_| ProviderError::Unreachable)?;
        let status = response.status().as_u16();
        if !(200..=299).contains(&status) {
            return Err(classify_failure_status(
                status,
                retry_after_value(response.headers()),
            ));
        }
        let Some(body) = bounded_body(response, MAXIMUM_MODEL_LIST_BYTES).await else {
            return Err(ProviderError::Unreachable);
        };
        Ok(parse_model_list(&body))
    };
    tokio::time::timeout(timeout, operation)
        .await
        .unwrap_or(Err(ProviderError::Unreachable))
}

async fn bounded_body(mut response: reqwest::Response, limit: usize) -> Option<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return None;
    }
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len().saturating_add(chunk.len()) > limit {
                    return None;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Some(body),
            Err(_) => return None,
        }
    }
}

pub(super) fn parse_model_list(body: &[u8]) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct ModelListing {
        #[serde(default)]
        data: Vec<ModelEntry>,
    }

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        #[serde(default)]
        id: String,
    }

    let Ok(listing) = serde_json::from_slice::<ModelListing>(body) else {
        return Vec::new();
    };
    let mut models: Vec<String> = Vec::new();
    for entry in listing.data {
        let id = entry.id.trim();
        if id.is_empty() || !model_is_bounded(id) || models.iter().any(|listed| listed == id) {
            continue;
        }
        if models.len() >= MAXIMUM_LISTED_MODELS {
            break;
        }
        models.push(id.to_owned());
    }
    models.sort();
    models
}

pub(super) async fn stream(
    connection: &ProviderConnection,
    history: &[ChatTurn],
) -> Result<TokenStream, ProviderError> {
    match connection.kind {
        ProviderKind::Xai => {
            let client = xai::Client::new(connection.api_key.expose())
                .map_err(|_| ProviderError::Unreachable)?;
            let model = client.completion_model(&connection.model);
            stream_with(model, history).await
        }
        ProviderKind::OpenaiCodex => {
            let client = openai::Client::new(connection.api_key.expose())
                .map_err(|_| ProviderError::Unreachable)?;
            let model = client.completion_model(&connection.model);
            stream_with(model, history).await
        }
        ProviderKind::Synthetic => {
            let client = openai::Client::builder()
                .api_key(connection.api_key.expose())
                .base_url(SYNTHETIC_BASE_URL)
                .build()
                .map_err(|_| ProviderError::Unreachable)?
                .completions_api();
            let model = client.completion_model(&connection.model);
            stream_with(model, history).await
        }
    }
}

fn classify_completion(error: CompletionError) -> ProviderError {
    match error
        .provider_response_status()
        .map(|status| status.as_u16())
    {
        Some(code) => classify_failure_status(code, retry_after_value_from_error(&error)),
        None => ProviderError::Unreachable,
    }
}

fn retry_after_value(headers: &reqwest::header::HeaderMap) -> Option<&str> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
}

fn retry_after_value_from_error(error: &CompletionError) -> Option<&str> {
    retry_after_value(error.provider_response_headers()?)
}

async fn stream_with<M>(model: M, history: &[ChatTurn]) -> Result<TokenStream, ProviderError>
where
    M: CompletionModel + Clone,
{
    let Some(last) = history.last() else {
        return Err(ProviderError::EmptyReply);
    };
    if last.role != Role::User {
        return Err(ProviderError::EmptyReply);
    }
    let prior = history[..history.len() - 1]
        .iter()
        .map(|turn| match turn.role {
            Role::User => Message::user(turn.text.clone()),
            Role::Assistant => Message::assistant(turn.text.clone()),
        })
        .collect::<Vec<_>>();
    let response = model
        .completion_request(last.text.clone())
        .preamble(PREAMBLE.to_owned())
        .messages(prior)
        .stream()
        .await
        .map_err(classify_completion)?;
    Ok(Box::pin(response.filter_map(|item| {
        std::future::ready(match item {
            Ok(StreamedAssistantContent::Text(text)) => Some(Ok(text.text)),
            Ok(_) => None,
            Err(error) => Some(Err(classify_completion(error))),
        })
    })))
}
