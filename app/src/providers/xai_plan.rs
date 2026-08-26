use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use url::Url;

use crate::vault::{VaultError, write_private};

use super::{
    AuthMethod, MAXIMUM_API_KEY_BYTES, ProviderError, api_key_is_bounded,
    classify_failure_status_for, with_provider_detail,
};

// Pinned from xai-org/grok-build@77cd7eb675ba911c225c3aaeeece3a20cbccc426.
const XAI_OAUTH_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_OAUTH_DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const XAI_OAUTH_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const XAI_ACCOUNTS_HOST: &str = "accounts.x.ai";
const XAI_AUTH_HOST: &str = "auth.x.ai";
pub(super) const XAI_PLAN_BASE_URL: &str = "https://cli-chat-proxy.grok.com/v1";
// The proxy answers 426 when these headers are absent.
const XAI_PLAN_CLIENT_VERSION: &str = "0.2.116";
const XAI_PLAN_CLIENT_IDENTIFIER: &str = "grok-shell";
const XAI_PLAN_USER_AGENT: &str = "xai-grok-cli";
const XAI_OAUTH_SCOPES: &str = "openid profile email offline_access grok-cli:access api:access conversations:read conversations:write workspaces:read workspaces:write";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const SLOW_DOWN_INCREMENT: Duration = Duration::from_secs(5);
const MIN_DEVICE_EXPIRY: Duration = Duration::from_secs(10 * 60);
const TOKEN_EXPIRY_SKEW_SECS: u64 = 60;
const MAXIMUM_USER_CODE_BYTES: usize = 64;
const MAXIMUM_VERIFICATION_URI_BYTES: usize = 2_048;
const MAXIMUM_OAUTH_ERROR_BYTES: usize = 4_096;

pub(super) fn proxy_headers() -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-grok-client-version",
        reqwest::header::HeaderValue::from_static(XAI_PLAN_CLIENT_VERSION),
    );
    headers.insert(
        "x-grok-client-identifier",
        reqwest::header::HeaderValue::from_static(XAI_PLAN_CLIENT_IDENTIFIER),
    );
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static(XAI_PLAN_USER_AGENT),
    );
    headers
}

static REFRESH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn refresh_lock() -> &'static Mutex<()> {
    REFRESH_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Clone)]
pub(super) struct DeviceCode {
    pub(super) verification_uri: String,
    pub(super) user_code: String,
    device_code: String,
    interval: Duration,
    expires_in: Duration,
}

#[derive(Deserialize, Serialize)]
struct PlanFile {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: Option<u64>,
}

impl std::fmt::Debug for PlanFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlanFile")
            .field("access_token", &"SecretString(<redacted>)")
            .field(
                "refresh_token",
                &self
                    .refresh_token
                    .as_ref()
                    .map(|_| "SecretString(<redacted>)"),
            )
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: i64,
    interval: Option<i64>,
}

#[derive(Deserialize)]
struct TokenOk {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Deserialize)]
struct TokenErr {
    #[serde(default)]
    error: String,
}

pub(super) async fn request_device_code() -> Result<DeviceCode, ProviderError> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", XAI_OAUTH_CLIENT_ID)
        .append_pair("scope", XAI_OAUTH_SCOPES)
        .append_pair("referrer", "grok-build")
        .finish();
    let response = reqwest::Client::new()
        .post(XAI_OAUTH_DEVICE_CODE_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|_| ProviderError::Unreachable)?;
    let status = response.status().as_u16();
    if !(200..=299).contains(&status) {
        let classified = classify_failure_status_for(status, None, AuthMethod::Plan);
        let body = bounded_text(response, MAXIMUM_OAUTH_ERROR_BYTES).await;
        return Err(with_provider_detail(classified, body.as_deref()));
    }
    let payload: DeviceCodeResponse = response
        .json()
        .await
        .map_err(|_| ProviderError::Unreachable)?;
    let user_code = sanitise_user_code(&payload.user_code).ok_or(ProviderError::Unreachable)?;
    let verification_uri = sanitise_verification_uri(
        payload
            .verification_uri_complete
            .as_deref()
            .filter(|uri| sanitise_verification_uri(uri).is_some())
            .unwrap_or(&payload.verification_uri),
    )
    .ok_or(ProviderError::Unreachable)?;
    let device_code = payload.device_code.trim();
    if !api_key_is_bounded(device_code) {
        return Err(ProviderError::Unreachable);
    }
    Ok(DeviceCode {
        verification_uri,
        user_code,
        device_code: device_code.to_owned(),
        interval: duration_secs(payload.interval).unwrap_or(DEFAULT_POLL_INTERVAL),
        expires_in: duration_secs(Some(payload.expires_in)).unwrap_or(MIN_DEVICE_EXPIRY),
    })
}

pub(super) async fn complete_device_code(
    device: DeviceCode,
    path: &Path,
) -> Result<(), ProviderError> {
    let mut interval = device.interval.max(Duration::from_secs(1));
    let deadline = tokio::time::Instant::now() + device.expires_in.max(MIN_DEVICE_EXPIRY);
    loop {
        tokio::time::sleep(interval).await;
        if tokio::time::Instant::now() > deadline {
            return Err(ProviderError::Unreachable);
        }
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", DEVICE_GRANT_TYPE)
            .append_pair("device_code", &device.device_code)
            .append_pair("client_id", XAI_OAUTH_CLIENT_ID)
            .finish();
        let response = reqwest::Client::new()
            .post(XAI_OAUTH_TOKEN_URL)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|_| ProviderError::Unreachable)?;
        let status = response.status().as_u16();
        if (200..=299).contains(&status) {
            let tokens: TokenOk = response
                .json()
                .await
                .map_err(|_| ProviderError::Unreachable)?;
            write_tokens(path, &tokens)?;
            return Ok(());
        }
        let payload = bounded_text(response, MAXIMUM_OAUTH_ERROR_BYTES).await;
        let error = payload
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<TokenErr>(bytes).ok())
            .map(|item| item.error)
            .unwrap_or_default();
        match error.as_str() {
            "authorization_pending" => {}
            "slow_down" => interval += SLOW_DOWN_INCREMENT,
            "access_denied" | "expired_token" => return Err(ProviderError::Unreachable),
            "invalid_grant" => return Err(ProviderError::Reauthenticate),
            _ => {
                return Err(classify_failure_status_for(status, None, AuthMethod::Plan));
            }
        }
    }
}

pub(super) async fn access_token(path: &Path) -> Result<String, ProviderError> {
    let _guard = refresh_lock().lock().await;
    let mut record = read_plan_file(path)?;
    if !token_expired(record.expires_at) && api_key_is_bounded(&record.access_token) {
        return Ok(record.access_token);
    }
    let Some(refresh_token) = record.refresh_token.clone() else {
        return Err(ProviderError::Reauthenticate);
    };
    if !api_key_is_bounded(&refresh_token) {
        return Err(ProviderError::Reauthenticate);
    }
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "refresh_token")
        .append_pair("refresh_token", &refresh_token)
        .append_pair("client_id", XAI_OAUTH_CLIENT_ID)
        .finish();
    let response = reqwest::Client::new()
        .post(XAI_OAUTH_TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|_| ProviderError::Unreachable)?;
    let status = response.status().as_u16();
    if !(200..=299).contains(&status) {
        let payload = bounded_text(response, MAXIMUM_OAUTH_ERROR_BYTES).await;
        let error = payload
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<TokenErr>(bytes).ok())
            .map(|item| item.error)
            .unwrap_or_default();
        if error == "invalid_grant" || matches!(status, 401 | 403) {
            return Err(ProviderError::Reauthenticate);
        }
        return Err(classify_failure_status_for(status, None, AuthMethod::Plan));
    }
    let tokens: TokenOk = response
        .json()
        .await
        .map_err(|_| ProviderError::Unreachable)?;
    write_tokens(path, &tokens)?;
    record = read_plan_file(path)?;
    Ok(record.access_token)
}

fn write_tokens(path: &Path, tokens: &TokenOk) -> Result<(), ProviderError> {
    if !api_key_is_bounded(&tokens.access_token) {
        return Err(ProviderError::Unreachable);
    }
    if tokens
        .refresh_token
        .as_deref()
        .is_some_and(|token| !api_key_is_bounded(token))
    {
        return Err(ProviderError::Unreachable);
    }
    let file = PlanFile {
        access_token: tokens.access_token.trim().to_owned(),
        refresh_token: tokens
            .refresh_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_owned),
        expires_at: tokens.expires_in.and_then(expiry_from_lifetime),
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|_| ProviderError::Unreachable)?;
    write_private(path, &bytes).map_err(|_: VaultError| ProviderError::Unreachable)
}

fn read_plan_file(path: &Path) -> Result<PlanFile, ProviderError> {
    let bytes = std::fs::read(path).map_err(|_| ProviderError::Reauthenticate)?;
    let file: PlanFile =
        serde_json::from_slice(&bytes).map_err(|_| ProviderError::Reauthenticate)?;
    if file.access_token.len() > MAXIMUM_API_KEY_BYTES
        || file
            .refresh_token
            .as_ref()
            .is_some_and(|token| token.len() > MAXIMUM_API_KEY_BYTES)
    {
        return Err(ProviderError::Reauthenticate);
    }
    Ok(file)
}

fn token_expired(expires_at: Option<u64>) -> bool {
    let Some(expires_at) = expires_at else {
        return true;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now + TOKEN_EXPIRY_SKEW_SECS >= expires_at
}

fn expiry_from_lifetime(seconds: i64) -> Option<u64> {
    let seconds = u64::try_from(seconds).ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    now.checked_add(seconds)
}

fn duration_secs(value: Option<i64>) -> Option<Duration> {
    let seconds = u64::try_from(value?).ok()?;
    (seconds > 0).then_some(Duration::from_secs(seconds))
}

fn sanitise_user_code(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAXIMUM_USER_CODE_BYTES
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return None;
    }
    Some(value.to_owned())
}

fn sanitise_verification_uri(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAXIMUM_VERIFICATION_URI_BYTES
        || value.chars().any(|character| character.is_control())
    {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some_and(|query| query.len() > 512)
    {
        return None;
    }
    let host = url.host_str()?;
    if host != XAI_ACCOUNTS_HOST && host != XAI_AUTH_HOST {
        return None;
    }
    Some(value.to_owned())
}

async fn bounded_text(mut response: reqwest::Response, limit: usize) -> Option<Vec<u8>> {
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

#[cfg(test)]
mod tests;
