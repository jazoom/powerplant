use super::*;

impl super::PlanAttempt {
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

use std::fs;

#[test]
fn device_prompts_reject_unsafe_provider_values() {
    assert_eq!(
        super::sanitise_user_code("  ABCD-1234  ").as_deref(),
        Some("ABCD-1234")
    );
    for value in ["", "code with spaces", "code\nnext", "\nABCD-1234"] {
        assert!(super::sanitise_user_code(value).is_none());
    }
    assert!(super::sanitise_user_code(&"A".repeat(65)).is_none());

    assert_eq!(
        super::sanitise_chatgpt_uri("  https://auth.openai.com/codex/device  ").as_deref(),
        Some("https://auth.openai.com/codex/device")
    );
    assert_eq!(
        super::sanitise_chatgpt_uri("https://auth.openai.com:443/codex/device").as_deref(),
        Some("https://auth.openai.com:443/codex/device")
    );
    for value in [
        "http://auth.openai.com/codex/device",
        "https://auth.openai.com.evil.com/codex/device",
        "https://evil.com/auth.openai.com",
        "https://auth.openai.com.attacker/codex/device",
        "https://auth.openai.com./codex/device",
        "https://user@auth.openai.com/codex/device",
        "https://@auth.openai.com/codex/device",
        "https://:@auth.openai.com/codex/device",
        "https://user:pass@auth.openai.com/codex/device",
        "https://auth.openai.com:8443/codex/device",
        "https://auth.openai.com/codex/device#frag",
        "https://auth.openai.com/codex/device#",
        "https://",
        "https://?device=1",
        "https:///codex/device",
        "https://auth.openai.com/codex/device\nnext",
        "\nhttps://auth.openai.com/codex/device",
        "javascript:alert(1)",
    ] {
        assert!(
            super::sanitise_chatgpt_uri(value).is_none(),
            "accepted {value}"
        );
    }
    assert!(
        super::sanitise_chatgpt_uri(&format!("https://auth.openai.com/{}", "a".repeat(2_048)))
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn chatgpt_staged_files_use_owner_read_write_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("dir");
    let path = super::stage_chatgpt_file(dir.path()).expect("stage");
    let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(fs::read(&path).expect("bytes"), b"{}");
}

#[tokio::test]
async fn dropping_an_attempt_aborts_the_task_and_removes_its_staged_file() {
    struct NotifyOnDrop(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for NotifyOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    let dir = tempfile::tempdir().expect("dir");
    let staged = crate::storage::create_unique_private(dir.path(), b"{}").expect("stage");
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _notify = NotifyOnDrop(Some(dropped_tx));
        let _ = started_tx.send(());
        std::future::pending::<Result<(), crate::providers::ProviderError>>().await
    });
    let attempt = super::PlanAttempt::from_parts(staged.clone(), task);
    started_rx.await.expect("task start");
    drop(attempt);

    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("task abort timeout")
        .expect("task drop notification");
    assert!(!staged.exists());
}

#[tokio::test]
async fn cancelling_a_wait_aborts_the_task_and_only_removes_its_staged_file() {
    struct NotifyOnDrop(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for NotifyOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    let dir = tempfile::tempdir().expect("dir");
    let old = crate::storage::create_unique_private(dir.path(), b"old").expect("old");
    let new = crate::storage::create_unique_private(dir.path(), b"new").expect("new");
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _notify = NotifyOnDrop(Some(dropped_tx));
        let _ = started_tx.send(());
        std::future::pending::<Result<(), crate::providers::ProviderError>>().await
    });
    let mut attempt = super::PlanAttempt::from_parts(old.clone(), task);
    let owner = tokio::spawn(async move { attempt.wait().await });
    started_rx.await.expect("task start");

    owner.abort();
    assert!(owner.await.expect_err("owner abort").is_cancelled());
    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("task abort timeout")
        .expect("task drop notification");
    assert!(!old.exists());
    assert_eq!(fs::read(&new).expect("new bytes"), b"new");
}
