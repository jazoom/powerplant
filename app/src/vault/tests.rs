use super::ProviderVault;
use crate::providers::{ProviderConnection, ProviderKind, SecretString};

const SECRET: &str = "sk-vault-secret-do-not-echo";

fn connection(kind: ProviderKind, model: &str) -> ProviderConnection {
    ProviderConnection {
        kind,
        api_key: SecretString::new(SECRET.to_owned()),
        model: model.to_owned(),
    }
}

fn file_vault() -> (ProviderVault, tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    (ProviderVault::open(path.clone()), dir, path)
}

#[test]
fn put_keeps_other_providers_and_survives_reload() {
    let (vault, _dir, path) = file_vault();
    vault
        .put(connection(ProviderKind::Xai, "grok-4.6"))
        .unwrap();
    vault
        .put(connection(ProviderKind::Synthetic, "hf:custom"))
        .unwrap();

    assert!(vault.contains(ProviderKind::Xai));
    assert!(vault.contains(ProviderKind::Synthetic));
    assert_eq!(
        vault.selected_connection().map(|item| item.kind),
        Some(ProviderKind::Synthetic)
    );

    let reloaded = ProviderVault::open(path);
    let desk = reloaded.desk_providers();
    assert_eq!(desk.len(), 2);
    assert_eq!(desk[0].kind, ProviderKind::Xai);
    assert_eq!(desk[1].kind, ProviderKind::Synthetic);
    assert_eq!(desk[1].model, "hf:custom");
    assert_eq!(
        reloaded
            .selected_connection()
            .map(|item| item.api_key.expose().to_owned())
            .as_deref(),
        Some(SECRET)
    );
}

#[test]
fn replacing_a_key_keeps_the_saved_model() {
    let vault = ProviderVault::in_memory();
    vault
        .put(connection(ProviderKind::Xai, "grok-4.6"))
        .unwrap();
    vault
        .select(ProviderKind::Xai, "grok-custom".to_owned())
        .unwrap();
    vault
        .put(connection(ProviderKind::Xai, "ignored-default"))
        .unwrap();
    assert_eq!(
        vault.selected_connection().map(|item| item.model),
        Some("grok-custom".to_owned())
    );
}

#[test]
fn forget_removes_one_provider_and_deletes_an_empty_file() {
    let (vault, _dir, path) = file_vault();
    vault
        .put(connection(ProviderKind::Xai, "grok-4.6"))
        .unwrap();
    vault
        .put(connection(ProviderKind::OpenaiCodex, "gpt-5.1-codex"))
        .unwrap();
    vault.forget(ProviderKind::Xai).unwrap();

    assert!(!vault.contains(ProviderKind::Xai));
    assert!(path.exists());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("xai"));
    assert!(text.contains("openai-codex"));

    vault.forget(ProviderKind::OpenaiCodex).unwrap();
    assert!(!vault.has_providers());
    assert!(!path.exists());
}

#[test]
fn persist_restricts_unix_permissions() {
    let (vault, _dir, path) = file_vault();
    vault
        .put(connection(ProviderKind::Xai, "grok-4.6"))
        .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn persist_error_debug_does_not_include_a_key() {
    assert_eq!(format!("{:?}", super::VaultError), "VaultError");
}

#[test]
fn invalid_files_load_as_empty() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    std::fs::write(&path, "{not-json").unwrap();
    let vault = ProviderVault::open(path);
    assert!(!vault.has_providers());
}
