use super::{ProviderVault, VaultError};
use crate::providers::{AuthMethod, MAXIMUM_FAVOURITES, ProviderConnection, ProviderKind};

const SECRET: &str = "sk-vault-secret-do-not-echo";

fn connection(kind: ProviderKind, model: &str) -> ProviderConnection {
    ProviderConnection::with_key(kind, SECRET, model)
}

fn file_vault() -> (ProviderVault, tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    (ProviderVault::open(path.clone()).expect("vault"), dir, path)
}

fn assert_open_leaves_bytes(path: &std::path::Path, bytes: &[u8]) {
    assert_eq!(
        ProviderVault::open(path.to_path_buf()).err(),
        Some(VaultError::Corrupt)
    );
    assert_eq!(std::fs::read(path).unwrap(), bytes);
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

    let reloaded = ProviderVault::open(path).expect("reload");
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

    let reloaded = ProviderVault::open(path).expect("reload");
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

    let reloaded = ProviderVault::open(path.clone()).expect("reload");
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
fn absent_file_opens_as_empty() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    let vault = ProviderVault::open(path).expect("absent");
    assert!(!vault.has_providers());
    assert!(vault.selected_connection().is_none());
}

#[test]
fn malformed_json_is_corrupt_and_leaves_the_file_unchanged() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    let bytes = b"{not-json";
    std::fs::write(&path, bytes).unwrap();
    assert_open_leaves_bytes(&path, bytes);
}

#[test]
fn unreadable_path_is_corrupt() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    std::fs::create_dir(&path).unwrap();
    assert_eq!(ProviderVault::open(path).err(), Some(VaultError::Corrupt));
}

#[test]
fn duplicate_providers_are_corrupt_and_leave_the_file_unchanged() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    let bytes = br#"{
        "version": 1,
        "selected": "xai",
        "providers": [
            {"kind": "xai", "auth": "api_key", "api_key": "sk-one", "model": "grok-4.6"},
            {"kind": "xai", "auth": "api_key", "api_key": "sk-two", "model": "grok-4.6"}
        ]
    }"#;
    std::fs::write(&path, bytes).unwrap();
    assert_open_leaves_bytes(&path, bytes);
}

#[test]
fn invalid_selections_are_corrupt_and_leave_the_file_unchanged() {
    let cases: &[&[u8]] = &[
        br#"{"version":1,"selected":null,"providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6"}]}"#,
        br#"{"version":1,"providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6"}]}"#,
        br#"{"version":1,"selected":"synthetic","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6"}]}"#,
        br#"{"version":1,"selected":"unknown","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6"}]}"#,
        br#"{"version":1,"selected":"xai","providers":[]}"#,
    ];
    for bytes in cases {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("providers.json");
        std::fs::write(&path, bytes).unwrap();
        assert_open_leaves_bytes(&path, bytes);
    }
}

#[test]
fn invalid_provider_records_are_corrupt_and_leave_the_file_unchanged() {
    let too_many_favourites = format!(
        r#"{{"version":1,"selected":"xai","providers":[{{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6","favourites":[{}]}}]}}"#,
        (0..=MAXIMUM_FAVOURITES)
            .map(|index| format!("\"model-{index}\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    let excessive_model = "a".repeat(crate::providers::MAXIMUM_MODEL_BYTES + 1);
    let excessive_key = "a".repeat(crate::providers::MAXIMUM_API_KEY_BYTES + 1);
    let mut cases = vec![
        br#"{"version":2,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6"}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"unknown","auth":"api_key","api_key":"sk-one","model":"grok-4.6"}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","api_key":"sk-one","model":"grok-4.6"}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"token","api_key":"sk-one","model":"grok-4.6"}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"","model":"grok-4.6"}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":" sk-one ","model":"grok-4.6"}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"bad\nkey","model":"grok-4.6"}]}"#.to_vec(),
        format!(
            r#"{{"version":1,"selected":"xai","providers":[{{"kind":"xai","auth":"api_key","api_key":"{excessive_key}","model":"grok-4.6"}}]}}"#
        )
        .into_bytes(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":""}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":" grok-4.6 "}]}"#.to_vec(),
        format!(
            r#"{{"version":1,"selected":"xai","providers":[{{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"{excessive_model}"}}]}}"#
        )
        .into_bytes(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"bad\nmodel"}]}"#.to_vec(),
        br#"{"version":1,"selected":"synthetic","providers":[{"kind":"synthetic","auth":"plan","model":"hf:custom"}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"plan","api_key":"sk-one","model":"grok-4.6"}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6","favourites":["grok-4.6","grok-4.6"]}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6","favourites":[""]}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6","favourites":[" grok-4.6 "]}]}"#.to_vec(),
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6","favourites":["bad\nmodel"]}]}"#.to_vec(),
        format!(
            r#"{{"version":1,"selected":"xai","providers":[{{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6","favourites":["{excessive_model}"]}}]}}"#
        )
        .into_bytes(),
    ];
    cases.push(too_many_favourites.into_bytes());
    for bytes in cases {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("providers.json");
        std::fs::write(&path, &bytes).unwrap();
        assert_open_leaves_bytes(&path, &bytes);
    }
}
