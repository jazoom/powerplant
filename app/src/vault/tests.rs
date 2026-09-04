use super::*;

impl super::ProviderVault {
    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            inner: Mutex::new(VaultState::default()),
            fail_after_next_persist: Mutex::new(false),
            fail_next_marker_remove: Mutex::new(false),
        }
    }
    pub(crate) fn put(&self, connection: ProviderConnection) -> Result<(), VaultError> {
        self.insert_api_key(connection)
    }
    pub(crate) fn fail_after_next_persist(&self) {
        *self
            .fail_after_next_persist
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    }
    pub(crate) fn fail_next_marker_remove(&self) {
        *self
            .fail_next_marker_remove
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    }
}

use super::{ProviderVault, VaultError};
use crate::providers::{AuthMethod, MAXIMUM_FAVOURITES, ProviderConnection, ProviderKind};
use std::path::{Path, PathBuf};

const SECRET: &str = "sk-vault-secret-do-not-echo";

fn connection(kind: ProviderKind, model: &str) -> ProviderConnection {
    ProviderConnection::with_key(kind, SECRET, model)
}

fn file_vault() -> (ProviderVault, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    (ProviderVault::open(path.clone()).expect("vault"), dir, path)
}

fn marker_for(plan_path: &Path) -> PathBuf {
    let mut name = plan_path.file_name().unwrap().to_os_string();
    name.push(".deleting");
    plan_path.with_file_name(name)
}

fn write_plan_metadata(path: &Path) {
    std::fs::write(
        path,
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"plan","model":"grok-4.6"}]}"#,
    )
    .unwrap();
}

fn write_api_metadata(path: &Path) {
    std::fs::write(
        path,
        br#"{"version":1,"selected":"synthetic","providers":[{"kind":"synthetic","auth":"api_key","api_key":"sk-one","model":"hf:custom"}]}"#,
    )
    .unwrap();
}

const PLAN_BYTES: &[u8] = b"plan-credential";

fn assert_open_leaves_bytes(path: &Path, bytes: &[u8]) {
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
        .select_settings(
            ProviderKind::Xai,
            "grok-custom".to_owned(),
            crate::providers::ThinkingLevel::Default,
        )
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
fn thinking_level_round_trips_and_survives_a_new_key() {
    let (vault, _dir, path) = file_vault();
    vault
        .put(connection(ProviderKind::Xai, "grok-4.6"))
        .unwrap();
    vault
        .select_settings(
            ProviderKind::Xai,
            "grok-4.6".to_owned(),
            crate::providers::ThinkingLevel::High,
        )
        .unwrap();

    let reloaded = ProviderVault::open(path).expect("reload");
    reloaded
        .put(connection(ProviderKind::Xai, "ignored-default"))
        .unwrap();
    assert_eq!(
        reloaded.selected_connection().map(|item| item.thinking),
        Some(crate::providers::ThinkingLevel::High)
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
    let staged = path.parent().unwrap().join("staged-xai.json");
    let plan_path = path.parent().unwrap().join("xai-auth.json");
    std::fs::write(
        &staged,
        br#"{"access_token":"xai-plan-access-do-not-echo"}"#,
    )
    .unwrap();
    vault.install_plan(ProviderKind::Xai, &staged).unwrap();
    assert!(plan_path.exists());
    assert!(!staged.exists());

    let reloaded = ProviderVault::open(path.clone()).expect("reload");
    let stored = reloaded.selected_connection().expect("plan");
    assert_eq!(stored.auth, AuthMethod::Plan);
    assert!(stored.api_key.expose().is_empty());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("xai-plan-access"));

    reloaded.forget(ProviderKind::Xai).unwrap();
    assert!(!plan_path.exists());
    assert!(!marker_for(&plan_path).exists());
}

#[test]
fn api_key_insertion_removes_the_prior_plan_file() {
    let (vault, _dir, path) = file_vault();
    let staged = path.parent().unwrap().join("staged-xai.json");
    let plan_path = path.parent().unwrap().join("xai-auth.json");
    std::fs::write(&staged, br#"{"access_token":"plan-token"}"#).unwrap();
    vault.install_plan(ProviderKind::Xai, &staged).unwrap();

    vault
        .insert_api_key(connection(ProviderKind::Xai, "grok-4.6"))
        .unwrap();

    assert!(!plan_path.exists());
    let stored = vault.selected_connection().expect("api key");
    assert_eq!(stored.auth, AuthMethod::ApiKey);
    assert_eq!(stored.api_key.expose(), SECRET);
}

#[test]
fn a_retired_chatgpt_plan_model_is_replaced_on_read() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    std::fs::write(
        &path,
        br#"{"version":1,"selected":"openai-codex","providers":[{"kind":"openai-codex","auth":"plan","model":"gpt-5.1-codex"}]}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("chatgpt-auth.json"), b"{}").unwrap();
    let vault = ProviderVault::open(path).expect("vault");
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
fn plan_installation_restores_metadata_and_files_after_persist_failure() {
    let (vault, _dir, path) = file_vault();
    vault
        .put(connection(ProviderKind::Xai, "grok-4.6"))
        .unwrap();
    let previous = std::fs::read(&path).unwrap();
    let staged = path.parent().unwrap().join("staged-xai.json");
    let plan_path = path.parent().unwrap().join("xai-auth.json");
    std::fs::write(
        &staged,
        br#"{"access_token":"xai-plan-access-do-not-echo"}"#,
    )
    .unwrap();

    vault.fail_after_next_persist();
    assert_eq!(
        vault.install_plan(ProviderKind::Xai, &staged).err(),
        Some(VaultError::Persist)
    );

    assert_eq!(std::fs::read(&path).unwrap(), previous);
    assert_eq!(
        std::fs::read(&staged).unwrap(),
        br#"{"access_token":"xai-plan-access-do-not-echo"}"#
    );
    assert!(!plan_path.exists());
    let stored = vault.selected_connection().expect("api key");
    assert_eq!(stored.auth, AuthMethod::ApiKey);
    assert_eq!(stored.api_key.expose(), SECRET);
}

#[test]
fn failed_plan_replacement_restores_the_prior_plan_file() {
    let (vault, _dir, path) = file_vault();
    let first = path.parent().unwrap().join("first-xai.json");
    let second = path.parent().unwrap().join("second-xai.json");
    let plan_path = path.parent().unwrap().join("xai-auth.json");
    std::fs::write(&first, br#"{"access_token":"first-token"}"#).unwrap();
    vault.install_plan(ProviderKind::Xai, &first).unwrap();
    let previous_metadata = std::fs::read(&path).unwrap();
    let previous_plan = std::fs::read(&plan_path).unwrap();
    std::fs::write(&second, br#"{"access_token":"second-token"}"#).unwrap();

    vault.fail_after_next_persist();
    assert_eq!(
        vault.install_plan(ProviderKind::Xai, &second).err(),
        Some(VaultError::Persist)
    );

    assert_eq!(std::fs::read(&path).unwrap(), previous_metadata);
    assert_eq!(std::fs::read(&plan_path).unwrap(), previous_plan);
    assert_eq!(
        std::fs::read(&second).unwrap(),
        br#"{"access_token":"second-token"}"#
    );
    assert_eq!(
        vault.selected_connection().map(|stored| stored.auth),
        Some(AuthMethod::Plan)
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
        br#"{"version":1,"selected":"xai","providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","model":"grok-4.6","thinking":"extreme"}]}"#.to_vec(),
        br#"{"version":1,"selected":"deepseek","providers":[{"kind":"deepseek","auth":"api_key","api_key":"sk-one","model":"deepseek-v4-flash","thinking":"low"}]}"#.to_vec(),
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

#[derive(Clone, Copy)]
enum PlanFiles {
    None,
    Final,
    Marker,
    Both,
}

#[test]
fn open_reconciles_every_final_file_and_marker_combination() {
    struct Case {
        named: bool,
        files: PlanFiles,
        open: Result<(), VaultError>,
        final_after: bool,
        marker_after: bool,
    }
    let cases = [
        Case {
            named: true,
            files: PlanFiles::Final,
            open: Ok(()),
            final_after: true,
            marker_after: false,
        },
        Case {
            named: true,
            files: PlanFiles::Marker,
            open: Ok(()),
            final_after: true,
            marker_after: false,
        },
        Case {
            named: true,
            files: PlanFiles::None,
            open: Err(VaultError::Corrupt),
            final_after: false,
            marker_after: false,
        },
        Case {
            named: true,
            files: PlanFiles::Both,
            open: Err(VaultError::Corrupt),
            final_after: true,
            marker_after: true,
        },
        Case {
            named: false,
            files: PlanFiles::Final,
            open: Ok(()),
            final_after: false,
            marker_after: false,
        },
        Case {
            named: false,
            files: PlanFiles::Marker,
            open: Ok(()),
            final_after: false,
            marker_after: false,
        },
        Case {
            named: false,
            files: PlanFiles::None,
            open: Ok(()),
            final_after: false,
            marker_after: false,
        },
        Case {
            named: false,
            files: PlanFiles::Both,
            open: Err(VaultError::Corrupt),
            final_after: true,
            marker_after: true,
        },
    ];
    for case in cases {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("providers.json");
        if case.named {
            write_plan_metadata(&path);
        } else {
            write_api_metadata(&path);
        }
        let metadata = std::fs::read(&path).unwrap();
        let plan_path = dir.path().join("xai-auth.json");
        let marker_path = marker_for(&plan_path);
        match case.files {
            PlanFiles::None => {}
            PlanFiles::Final => std::fs::write(&plan_path, PLAN_BYTES).unwrap(),
            PlanFiles::Marker => std::fs::write(&marker_path, PLAN_BYTES).unwrap(),
            PlanFiles::Both => {
                std::fs::write(&plan_path, PLAN_BYTES).unwrap();
                std::fs::write(&marker_path, PLAN_BYTES).unwrap();
            }
        }
        let opened = ProviderVault::open(path.clone());
        match case.open {
            Ok(()) => {
                let vault = opened.expect("open");
                assert_eq!(vault.contains(ProviderKind::Xai), case.named);
                if case.named {
                    assert_eq!(
                        vault.selected_connection().map(|item| item.auth),
                        Some(AuthMethod::Plan)
                    );
                    assert_eq!(std::fs::read(&plan_path).unwrap(), PLAN_BYTES);
                }
            }
            Err(error) => {
                assert_eq!(opened.err(), Some(error));
                assert_eq!(std::fs::read(&path).unwrap(), metadata);
            }
        }
        assert_eq!(plan_path.exists(), case.final_after);
        assert_eq!(marker_path.exists(), case.marker_after);
    }
}

#[test]
fn open_removes_abandoned_staged_files() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    write_api_metadata(&path);
    let staged = dir.path().join(".deadbeef.staging");
    std::fs::write(&staged, b"abandoned").unwrap();
    ProviderVault::open(path).expect("open");
    assert!(!staged.exists());
}

#[test]
fn open_fails_when_staged_recovery_cannot_complete() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    write_api_metadata(&path);
    let metadata = std::fs::read(&path).unwrap();
    let staged = dir.path().join(".deadbeef.staging");
    std::fs::create_dir(&staged).unwrap();
    assert_eq!(
        ProviderVault::open(path.clone()).err(),
        Some(VaultError::Corrupt)
    );
    assert_eq!(std::fs::read(&path).unwrap(), metadata);
    assert!(staged.exists());
}

#[test]
fn forget_of_an_absent_api_key_provider_is_idempotent() {
    let (vault, _dir, path) = file_vault();
    vault.forget(ProviderKind::Xai).unwrap();
    assert!(!path.exists());
    vault
        .put(connection(ProviderKind::Synthetic, "hf:custom"))
        .unwrap();
    vault.forget(ProviderKind::Xai).unwrap();
    assert!(vault.contains(ProviderKind::Synthetic));
    vault.forget(ProviderKind::Synthetic).unwrap();
    vault.forget(ProviderKind::Synthetic).unwrap();
    assert!(!vault.has_providers());
    assert!(!path.exists());
}

#[test]
fn forget_rejects_a_missing_plan_file_without_changing_metadata() {
    let (vault, _dir, path) = file_vault();
    let staged = path.parent().unwrap().join("staged-xai.json");
    let plan_path = path.parent().unwrap().join("xai-auth.json");
    std::fs::write(&staged, PLAN_BYTES).unwrap();
    vault.install_plan(ProviderKind::Xai, &staged).unwrap();
    let previous = std::fs::read(&path).unwrap();
    std::fs::remove_file(&plan_path).unwrap();

    assert_eq!(
        vault.forget(ProviderKind::Xai).err(),
        Some(VaultError::Persist)
    );

    assert_eq!(std::fs::read(&path).unwrap(), previous);
    assert!(vault.contains(ProviderKind::Xai));
}

#[test]
fn forget_restores_plan_files_after_persist_failure() {
    let (vault, _dir, path) = file_vault();
    let staged = path.parent().unwrap().join("staged-xai.json");
    let plan_path = path.parent().unwrap().join("xai-auth.json");
    std::fs::write(&staged, PLAN_BYTES).unwrap();
    vault.install_plan(ProviderKind::Xai, &staged).unwrap();
    let previous = std::fs::read(&path).unwrap();
    let previous_plan = std::fs::read(&plan_path).unwrap();

    vault.fail_after_next_persist();
    assert_eq!(
        vault.forget(ProviderKind::Xai).err(),
        Some(VaultError::Persist)
    );

    assert_eq!(std::fs::read(&path).unwrap(), previous);
    assert_eq!(std::fs::read(&plan_path).unwrap(), previous_plan);
    assert!(!marker_for(&plan_path).exists());
    assert!(vault.contains(ProviderKind::Xai));
    assert_eq!(
        vault.selected_connection().map(|item| item.auth),
        Some(AuthMethod::Plan)
    );
}

#[test]
fn forget_restores_plan_files_after_marker_removal_failure() {
    let (vault, _dir, path) = file_vault();
    let staged = path.parent().unwrap().join("staged-xai.json");
    let plan_path = path.parent().unwrap().join("xai-auth.json");
    std::fs::write(&staged, PLAN_BYTES).unwrap();
    vault.install_plan(ProviderKind::Xai, &staged).unwrap();
    let previous = std::fs::read(&path).unwrap();
    let previous_plan = std::fs::read(&plan_path).unwrap();

    vault.fail_next_marker_remove();
    assert_eq!(
        vault.forget(ProviderKind::Xai).err(),
        Some(VaultError::Persist)
    );

    assert_eq!(std::fs::read(&path).unwrap(), previous);
    assert_eq!(std::fs::read(&plan_path).unwrap(), previous_plan);
    assert!(!marker_for(&plan_path).exists());
    assert!(vault.contains(ProviderKind::Xai));
}

#[test]
fn forget_restores_api_key_metadata_after_persist_failure() {
    let (vault, _dir, path) = file_vault();
    vault
        .put(connection(ProviderKind::Xai, "grok-4.6"))
        .unwrap();
    let previous = std::fs::read(&path).unwrap();
    vault.fail_after_next_persist();
    assert_eq!(
        vault.forget(ProviderKind::Xai).err(),
        Some(VaultError::Persist)
    );
    assert_eq!(std::fs::read(&path).unwrap(), previous);
    assert!(vault.contains(ProviderKind::Xai));
}

#[cfg(unix)]
#[test]
fn open_restricts_retained_plan_files_to_owner_read_write() {
    use std::os::unix::fs::PermissionsExt;

    for restore_marker in [false, true] {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("providers.json");
        write_plan_metadata(&path);
        let plan_path = dir.path().join("xai-auth.json");
        let credential_path = if restore_marker {
            marker_for(&plan_path)
        } else {
            plan_path.clone()
        };
        std::fs::write(&credential_path, PLAN_BYTES).unwrap();
        let mut permissions = std::fs::metadata(&credential_path).unwrap().permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(&credential_path, permissions).unwrap();

        ProviderVault::open(path).expect("open");
        let mode = std::fs::metadata(&plan_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
