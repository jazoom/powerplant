use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use url::Url;

use super::{ProviderError, ProviderKind, xai_plan};

#[cfg(test)]
mod tests;

const CHATGPT_AUTH_HOST: &str = "auth.openai.com";

pub(crate) struct DevicePrompt {
    pub(crate) verification_uri: String,
    pub(crate) user_code: String,
}

// A unique staged path keeps an aborted authorisation from writing into a later attempt.
pub(crate) struct PlanAttempt {
    pub(crate) prompt: DevicePrompt,
    staged: Option<PathBuf>,
    task: Option<JoinHandle<Result<(), ProviderError>>>,
}

impl PlanAttempt {
    pub(crate) fn staged_path(&self) -> Option<&Path> {
        self.staged.as_deref()
    }

    pub(crate) async fn wait(&mut self) -> Result<(), ProviderError> {
        let result = match self.task.as_mut() {
            Some(task) => task.await,
            None => return Err(ProviderError::Unreachable),
        };
        self.task = None;
        match result {
            Ok(result) => result,
            Err(_) => Err(ProviderError::Unreachable),
        }
    }

    pub(crate) fn mark_installed(&mut self) {
        self.staged = None;
    }

    pub(crate) fn discard(&mut self) -> Result<(), crate::storage::PersistError> {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        if let Some(path) = self.staged.as_ref() {
            crate::storage::remove_private(path)?;
            self.staged = None;
        }
        Ok(())
    }

    #[cfg(test)]
    fn from_parts(staged: PathBuf, task: JoinHandle<Result<(), ProviderError>>) -> Self {
        Self {
            prompt: DevicePrompt {
                verification_uri: "https://auth.openai.com/codex/device".to_owned(),
                user_code: "TEST-CODE".to_owned(),
            },
            staged: Some(staged),
            task: Some(task),
        }
    }
}

impl Drop for PlanAttempt {
    fn drop(&mut self) {
        let _ = self.discard();
    }
}

pub(crate) async fn start(kind: ProviderKind, dir: PathBuf) -> Result<PlanAttempt, ProviderError> {
    crate::storage::ensure_private_dir(&dir).map_err(|_| ProviderError::Unreachable)?;
    match kind {
        ProviderKind::OpenaiCodex => start_chatgpt(dir).await,
        ProviderKind::Xai => start_xai(dir).await,
        ProviderKind::Synthetic | ProviderKind::Openrouter | ProviderKind::Deepseek => {
            Err(ProviderError::Refused)
        }
    }
}

// Rig writes tokens with std::fs::write, which keeps the mode of a pre-created 0600 file.
fn stage_chatgpt_file(dir: &Path) -> Result<PathBuf, ProviderError> {
    crate::storage::create_unique_private(dir, b"{}").map_err(|_| ProviderError::Unreachable)
}

async fn start_chatgpt(dir: PathBuf) -> Result<PlanAttempt, ProviderError> {
    let staged = stage_chatgpt_file(&dir)?;
    let (prompt_tx, prompt_rx) = oneshot::channel();
    let prompt_tx = std::sync::Mutex::new(Some(prompt_tx));
    let client = match rig_core::providers::chatgpt::Client::builder()
        .oauth()
        .auth_file(&staged)
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
    {
        Ok(client) => client,
        Err(_) => {
            crate::storage::remove_private(&staged).map_err(|_| ProviderError::Unreachable)?;
            return Err(ProviderError::Unreachable);
        }
    };
    let task = tokio::spawn(async move {
        client
            .authorize()
            .await
            .map_err(|_| ProviderError::Unreachable)
    });
    let mut attempt = PlanAttempt {
        prompt: DevicePrompt {
            verification_uri: String::new(),
            user_code: String::new(),
        },
        staged: Some(staged),
        task: Some(task),
    };
    let prompt = match tokio::time::timeout(Duration::from_secs(20), prompt_rx).await {
        Ok(Ok(prompt)) => prompt,
        Ok(Err(_)) | Err(_) => {
            attempt.discard().map_err(|_| ProviderError::Unreachable)?;
            return Err(ProviderError::Unreachable);
        }
    };
    let Some(verification_uri) = sanitise_chatgpt_uri(&prompt.verification_uri) else {
        attempt.discard().map_err(|_| ProviderError::Unreachable)?;
        return Err(ProviderError::Unreachable);
    };
    let Some(user_code) = sanitise_user_code(&prompt.user_code) else {
        attempt.discard().map_err(|_| ProviderError::Unreachable)?;
        return Err(ProviderError::Unreachable);
    };
    attempt.prompt = DevicePrompt {
        verification_uri,
        user_code,
    };
    Ok(attempt)
}

async fn start_xai(dir: PathBuf) -> Result<PlanAttempt, ProviderError> {
    let device = xai_plan::request_device_code().await?;
    let staged =
        crate::storage::create_unique_private(&dir, b"").map_err(|_| ProviderError::Unreachable)?;
    let path = staged.clone();
    let prompt = DevicePrompt {
        verification_uri: device.verification_uri.clone(),
        user_code: device.user_code.clone(),
    };
    let task = tokio::spawn(async move { xai_plan::complete_device_code(device, &path).await });
    Ok(PlanAttempt {
        prompt,
        staged: Some(staged),
        task: Some(task),
    })
}

fn sanitise_user_code(value: &str) -> Option<String> {
    if value.chars().any(|character| character.is_control()) {
        return None;
    }
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

fn sanitise_chatgpt_uri(value: &str) -> Option<String> {
    if value.chars().any(|character| character.is_control()) {
        return None;
    }
    let value = value.trim();
    if value.is_empty() || value.len() > 2_048 {
        return None;
    }
    let url = Url::parse(value).ok()?;
    let authority = value.split_once("://")?.1.split(['/', '?', '#']).next()?;
    if url.scheme() != "https"
        || url.host_str() != Some(CHATGPT_AUTH_HOST)
        || authority.contains('@')
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port().is_some()
    {
        return None;
    }
    Some(value.to_owned())
}
