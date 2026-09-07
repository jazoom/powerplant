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
use crate::providers::{AuthMethod, ProviderConnection, ProviderKind};
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
        br#"{"version":1,"providers":[{"kind":"xai","auth":"plan","api_key":""}]}"#,
    )
    .unwrap();
}

fn write_api_metadata(path: &Path) {
    std::fs::write(
        path,
        br#"{"version":1,"providers":[{"kind":"synthetic","auth":"api_key","api_key":"sk-one"}]}"#,
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
    let file: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(file.get("selected").is_none());
    for provider in file["providers"].as_array().unwrap() {
        for field in ["model", "thinking", "favourites"] {
            assert!(provider.get(field).is_none());
        }
    }
    let reloaded = ProviderVault::open(path).expect("reload");
    assert_eq!(
        reloaded.providers(),
        vec![
            (ProviderKind::Xai, AuthMethod::ApiKey),
            (ProviderKind::Synthetic, AuthMethod::ApiKey)
        ]
    );
    assert_eq!(
        reloaded
            .first_connection()
            .map(|item| item.api_key.expose().to_owned())
            .as_deref(),
        Some(SECRET)
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
    let stored = reloaded.first_connection().expect("plan");
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
    let stored = vault.first_connection().expect("api key");
    assert_eq!(stored.auth, AuthMethod::ApiKey);
    assert_eq!(stored.api_key.expose(), SECRET);
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
    let stored = vault.first_connection().expect("api key");
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
        vault.first_connection().map(|stored| stored.auth),
        Some(AuthMethod::Plan)
    );
}

#[test]
fn absent_file_opens_as_empty() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    let vault = ProviderVault::open(path).expect("absent");
    assert!(!vault.has_providers());
    assert!(vault.first_connection().is_none());
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
fn missing_current_provider_fields_are_corrupt() {
    for provider in [
        r#"{"kind":"xai","auth":"api_key"}"#,
        r#"{"kind":"xai","api_key":"sk-one"}"#,
        r#"{"auth":"api_key","api_key":"sk-one"}"#,
    ] {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("providers.json");
        let bytes = format!(r#"{{"version":1,"providers":[{provider}]}}"#);
        std::fs::write(&path, &bytes).unwrap();
        assert_open_leaves_bytes(&path, bytes.as_bytes());
    }
}

#[test]
fn unknown_vault_fields_are_corrupt() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("providers.json");
    let bytes = br#"{"version":1,"providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one","removed-field":true}]}"#;
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
        "providers": [
            {"kind": "xai", "auth": "api_key", "api_key": "sk-one"},
            {"kind": "xai", "auth": "api_key", "api_key": "sk-two"}
        ]
    }"#;
    std::fs::write(&path, bytes).unwrap();
    assert_open_leaves_bytes(&path, bytes);
}

#[test]
fn invalid_provider_records_are_corrupt_and_leave_the_file_unchanged() {
    let excessive_key = "a".repeat(crate::providers::MAXIMUM_API_KEY_BYTES + 1);
    let cases = vec![
        br#"{"version":2,"providers":[{"kind":"xai","auth":"api_key","api_key":"sk-one"}]}"#.to_vec(),
        br#"{"version":1,"providers":[{"kind":"unknown","auth":"api_key","api_key":"sk-one"}]}"#.to_vec(),
        br#"{"version":1,"providers":[{"kind":"xai","api_key":"sk-one"}]}"#.to_vec(),
        br#"{"version":1,"providers":[{"kind":"xai","auth":"token","api_key":"sk-one"}]}"#.to_vec(),
        br#"{"version":1,"providers":[{"kind":"xai","auth":"api_key","api_key":""}]}"#.to_vec(),
        br#"{"version":1,"providers":[{"kind":"xai","auth":"api_key","api_key":" sk-one "}]}"#.to_vec(),
        br#"{"version":1,"providers":[{"kind":"xai","auth":"api_key","api_key":"bad\nkey"}]}"#.to_vec(),
        format!(
            r#"{{"version":1,"providers":[{{"kind":"xai","auth":"api_key","api_key":"{excessive_key}"}}]}}"#
        )
        .into_bytes(),
        br#"{"version":1,"providers":[{"kind":"synthetic","auth":"plan"}]}"#.to_vec(),
        br#"{"version":1,"providers":[{"kind":"xai","auth":"plan","api_key":"sk-one"}]}"#.to_vec(),
    ];
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
                        vault.first_connection().map(|item| item.auth),
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
        vault.first_connection().map(|item| item.auth),
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

impl super::ProviderVault {
    fn first_connection(&self) -> Option<ProviderConnection> {
        let kind = self.providers().first()?.0;
        let state = self.lock();
        connection_from(self.path.as_deref(), &state, kind)
    }
}
