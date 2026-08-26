use std::time::Duration;

use futures_util::StreamExt;
use rig_core::{
    client::CompletionClient,
    completion::{CompletionError, CompletionModel, Message},
    providers::{deepseek, openai, openrouter, xai},
    streaming::StreamedAssistantContent,
};

use super::{
    ChatTurn, DEEPSEEK_BASE_URL, MAXIMUM_LISTED_MODELS, OPENROUTER_BASE_URL, ProviderConnection,
    ProviderError, ProviderKind, Role, SYNTHETIC_BASE_URL, TokenStream, classify_failure_status,
    classify_verify_status, model_is_bounded, with_json_detail, with_provider_detail,
};

const XAI_BASE_URL: &str = "https://api.x.ai/v1";
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

const MAXIMUM_MODEL_LIST_BYTES: usize = 1_048_576;
const MAXIMUM_PROVIDER_ERROR_BYTES: usize = 4_096;

const PREAMBLE: &str = "You are Power Plant, a local coding agent. Help the user write, explain and review code. Be direct.";

pub(super) const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

fn base_url(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Xai => XAI_BASE_URL,
        ProviderKind::OpenaiCodex => OPENAI_BASE_URL,
        ProviderKind::Synthetic => SYNTHETIC_BASE_URL,
        ProviderKind::Openrouter => OPENROUTER_BASE_URL,
        ProviderKind::Deepseek => DEEPSEEK_BASE_URL,
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
// Read only status, Retry-After, and a bounded error.message. Never log the body.
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
    let status = response.status().as_u16();
    let retry_after = retry_after_value(response.headers());
    match classify_verify_status(status, retry_after) {
        Ok(()) => Ok(()),
        Err(error) => {
            let body = bounded_body(response, MAXIMUM_PROVIDER_ERROR_BYTES).await;
            Err(with_provider_detail(error, body.as_deref()))
        }
    }
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
            let classified = classify_failure_status(status, retry_after_value(response.headers()));
            let body = bounded_body(response, MAXIMUM_PROVIDER_ERROR_BYTES).await;
            return Err(with_provider_detail(classified, body.as_deref()));
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
        ProviderKind::Openrouter => {
            let client = openrouter::Client::new(connection.api_key.expose())
                .map_err(|_| ProviderError::Unreachable)?;
            let model = client.completion_model(&connection.model);
            stream_with(model, history).await
        }
        ProviderKind::Deepseek => {
            let client = deepseek::Client::new(connection.api_key.expose())
                .map_err(|_| ProviderError::Unreachable)?;
            let model = client.completion_model(&connection.model);
            stream_with(model, history).await
        }
    }
}

fn classify_completion(error: CompletionError) -> ProviderError {
    let classified = match error
        .provider_response_status()
        .map(|status| status.as_u16())
    {
        Some(code) => classify_failure_status(code, retry_after_from_completion(&error)),
        None => ProviderError::Unreachable,
    };
    let json = error.provider_response_json().ok().flatten();
    with_json_detail(classified, json.as_ref())
}

fn retry_after_value(headers: &reqwest::header::HeaderMap) -> Option<&str> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
}

fn retry_after_from_completion(error: &CompletionError) -> Option<&str> {
    error
        .provider_response_headers()?
        .get("retry-after")?
        .to_str()
        .ok()
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
