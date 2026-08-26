use super::ProviderVault;
use crate::providers::{AuthMethod, ProviderConnection, ProviderKind};

const SECRET: &str = "sk-vault-secret-do-not-echo";

fn connection(kind: ProviderKind, model: &str) -> ProviderConnection {
    ProviderConnection::with_key(kind, SECRET, model)
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
fn favourites_round_trip_respect_the_cap_and_survive_a_new_key() {
    let (vault, _dir, path) = file_vault();
    vault
        .put(connection(ProviderKind::Xai, "grok-4.6"))
        .unwrap();
    assert_eq!(
        vault.toggle_favourite(ProviderKind::Xai, "grok-4.6").ok(),
        Some(true)
    );
    assert_eq!(
        vault.toggle_favourite(ProviderKind::Xai, "grok-4.6").ok(),
        Some(false)
    );
    assert_eq!(
        vault.toggle_favourite(ProviderKind::Xai, "grok-4.6").ok(),
        Some(true)
    );

    let reloaded = ProviderVault::open(path);
    let favourites = &reloaded.desk_providers()[0].favourites;
    assert_eq!(favourites, &vec!["grok-4.6".to_owned()]);

    reloaded
        .put(connection(ProviderKind::Xai, "ignored-default"))
        .unwrap();
    let favourites = &reloaded.desk_providers()[0].favourites;
    assert_eq!(favourites, &vec!["grok-4.6".to_owned()]);

    for index in 1..crate::providers::MAXIMUM_FAVOURITES {
        reloaded
            .toggle_favourite(ProviderKind::Xai, &format!("model-{index}"))
            .unwrap();
    }
    assert!(matches!(
        reloaded.toggle_favourite(ProviderKind::Xai, "one-more"),
        Err(super::FavouriteError::Full)
    ));
    assert_eq!(
        reloaded.selected_connection().map(|item| item.model),
        Some("grok-4.6".to_owned())
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
fn plan_auth_round_trips_without_a_key_and_forget_deletes_the_plan_file() {
    let (vault, _dir, path) = file_vault();
    let plan_path = path.parent().unwrap().join("xai-auth.json");
    std::fs::write(
        &plan_path,
        br#"{"access_token":"xai-plan-access-do-not-echo"}"#,
    )
    .unwrap();
    vault
        .put(ProviderConnection::with_plan(
            ProviderKind::Xai,
            "grok-4.6",
            Some(plan_path.clone()),
        ))
        .unwrap();

    let reloaded = ProviderVault::open(path.clone());
    let stored = reloaded.selected_connection().expect("plan");
    assert_eq!(stored.auth, AuthMethod::Plan);
    assert!(stored.api_key.expose().is_empty());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("xai-plan-access"));

    reloaded.forget(ProviderKind::Xai).unwrap();
    assert!(!plan_path.exists());
}

#[test]
fn a_retired_chatgpt_plan_model_is_replaced_on_read() {
    let vault = ProviderVault::in_memory();
    vault
        .put(ProviderConnection::with_plan(
            ProviderKind::OpenaiCodex,
            "gpt-5.1-codex",
            None,
        ))
        .unwrap();
    assert_eq!(
        vault.selected_connection().map(|item| item.model),
        Some("gpt-5.6-sol".to_owned())
    );
    assert_eq!(
        vault
            .desk_providers()
            .into_iter()
            .find(|item| item.kind == ProviderKind::OpenaiCodex)
            .map(|item| item.model),
        Some("gpt-5.6-sol".to_owned())
    );
}

#[test]
fn invalid_files_load_as_empty() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    std::fs::write(&path, "{not-json").unwrap();
    let vault = ProviderVault::open(path);
    assert!(!vault.has_providers());
}
