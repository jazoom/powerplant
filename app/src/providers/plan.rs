use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::oneshot;

use super::{ProviderError, ProviderKind, xai_plan};

#[cfg(test)]
mod tests;

pub(crate) struct DevicePrompt {
    pub(crate) verification_uri: String,
    pub(crate) user_code: String,
}

pub(crate) struct StartedPlan {
    pub(crate) prompt: DevicePrompt,
    pub(crate) done: oneshot::Receiver<Result<(), ProviderError>>,
}

pub(crate) async fn start(
    kind: ProviderKind,
    plan_file: PathBuf,
) -> Result<StartedPlan, ProviderError> {
    if let Some(parent) = plan_file.parent() {
        std::fs::create_dir_all(parent).map_err(|_| ProviderError::Unreachable)?;
    }
    match kind {
        ProviderKind::OpenaiCodex => start_chatgpt(plan_file).await,
        ProviderKind::Xai => start_xai(plan_file).await,
        ProviderKind::Synthetic | ProviderKind::Openrouter | ProviderKind::Deepseek => {
            Err(ProviderError::Refused)
        }
    }
}

async fn start_chatgpt(plan_file: PathBuf) -> Result<StartedPlan, ProviderError> {
    let _ = std::fs::remove_file(&plan_file);
    let (prompt_tx, prompt_rx) = oneshot::channel();
    let prompt_tx = std::sync::Mutex::new(Some(prompt_tx));
    let (done_tx, done_rx) = oneshot::channel();
    let client = rig_core::providers::chatgpt::Client::builder()
        .oauth()
        .auth_file(&plan_file)
        .on_device_code(move |prompt| {
            if let Ok(mut slot) = prompt_tx.lock()
                && let Some(tx) = slot.take()
            {
                let _ = tx.send(DevicePrompt {
                    verification_uri: prompt.verification_uri,
                    user_code: prompt.user_code,
                });
            }
        })
        .build()
        .map_err(|_| ProviderError::Unreachable)?;
    tokio::spawn(async move {
        let result = client
            .authorize()
            .await
            .map_err(|_| ProviderError::Unreachable);
        let _ = done_tx.send(result);
    });
    let prompt = tokio::time::timeout(Duration::from_secs(20), prompt_rx)
        .await
        .map_err(|_| ProviderError::Unreachable)?
        .map_err(|_| ProviderError::Unreachable)?;
    let verification_uri =
        sanitise_https_uri(&prompt.verification_uri).ok_or(ProviderError::Unreachable)?;
    let user_code = sanitise_user_code(&prompt.user_code).ok_or(ProviderError::Unreachable)?;
    Ok(StartedPlan {
        prompt: DevicePrompt {
            verification_uri,
            user_code,
        },
        done: done_rx,
    })
}

async fn start_xai(plan_file: PathBuf) -> Result<StartedPlan, ProviderError> {
    let _ = std::fs::remove_file(&plan_file);
    let device = xai_plan::request_device_code().await?;
    let (done_tx, done_rx) = oneshot::channel();
    let prompt = DevicePrompt {
        verification_uri: device.verification_uri.clone(),
        user_code: device.user_code.clone(),
    };
    tokio::spawn(async move {
        let result = xai_plan::complete_device_code(device, &plan_file).await;
        let _ = done_tx.send(result);
    });
    Ok(StartedPlan {
        prompt,
        done: done_rx,
    })
}

fn sanitise_user_code(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return None;
    }
    Some(value.to_owned())
}

fn sanitise_https_uri(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 2_048
        || value.chars().any(|character| character.is_control())
    {
        return None;
    }
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(value.to_owned())
}
