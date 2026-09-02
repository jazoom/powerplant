use super::*;

impl super::EnvironmentCatalogue {
    pub(crate) fn in_memory() -> Self {
        let scratch = tempfile::tempdir().expect("logs");
        let log_dir = scratch.path().join("logs");
        storage::ensure_private_dir(&log_dir).expect("log dir");
        Self {
            path: None,
            log_dir: Some(log_dir),
            inner: Mutex::new(empty_state()),
            refresh: RefreshState::new(),
            _scratch: Some(scratch),
        }
    }
    pub(crate) fn apply_production_seeds(&self) {
        let mut state = self.lock();
        let mut next = state.clone_state();
        let changed =
            apply_absent_seeds(&mut next, &crate::environments::seeds::production_seeds())
                .expect("seeds");
        if !changed {
            return;
        }
        persist(self.path.as_deref(), &next).expect("persist");
        *state = next;
        drop(state);
        self.bump_refresh();
    }
    pub(crate) fn retired_ids(&self) -> Vec<EnvironmentId> {
        self.lock().retired_environment_ids.clone()
    }
    pub(crate) fn applied_seed_count(&self) -> usize {
        self.lock().applied_seeds.len()
    }
    pub(crate) fn preparation_count(&self) -> usize {
        self.lock().preparations.len()
    }
}

use super::{EnvironmentCatalogue, EnvironmentError, MAXIMUM_ENVIRONMENTS};
use crate::environments::id::{EnvironmentId, PreparationId};
use crate::environments::preparation::{FailureCategory, PreparationPhase, PreparationState};
use crate::environments::recipe::EnvironmentDraft;
use crate::tests::sample_snapshot;

fn draft(name: &str, image: &str, script: &str) -> EnvironmentDraft {
    EnvironmentDraft {
        name: name.to_owned(),
        oci_image: image.to_owned(),
        setup_script: script.to_owned(),
    }
}

fn write_catalogue(path: &std::path::Path, json: &str) {
    std::fs::write(path, json).expect("write");
}

#[test]
fn create_assigns_identifiers_and_queues_preparation() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (record, preparation) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    assert_eq!(record.revision, 1);
    assert_eq!(record.latest_preparation, preparation.id);
    assert!(record.ready_preparation.is_none());
    assert_eq!(preparation.ordinal, 1);
    assert_eq!(preparation.state, PreparationState::Queued);
    assert_eq!(catalogue.list().len(), 1);
}

#[test]
fn create_rejects_duplicate_names_without_case_differences() {
    let catalogue = EnvironmentCatalogue::in_memory();
    catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    assert_eq!(
        catalogue
            .create(draft("alpine git", "alpine/git", ""))
            .err(),
        Some(EnvironmentError::DuplicateName)
    );
}

#[test]
fn identical_updates_do_not_mutate() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (created, _) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    let updated = catalogue
        .update(
            &created.id,
            created.revision,
            draft("Alpine Git", "alpine/git", ""),
        )
        .expect("update");
    assert_eq!(updated.environment.revision, created.revision);
    assert!(updated.preparation.is_none());
    assert_eq!(updated.environment.updated_at_ms, created.updated_at_ms);
}

#[test]
fn name_only_updates_keep_the_recipe_and_skip_preparation() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (created, first) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    let updated = catalogue
        .update(
            &created.id,
            created.revision,
            draft("Renamed", "alpine/git", ""),
        )
        .expect("update");
    assert_eq!(updated.environment.revision, created.revision + 1);
    assert_eq!(updated.environment.recipe_version, created.recipe_version);
    assert_eq!(updated.environment.latest_preparation, first.id);
    assert!(updated.preparation.is_none());
}

#[test]
fn recipe_updates_queue_replacement_and_supersede_queued_work() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (created, first) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    let updated = catalogue
        .update(
            &created.id,
            created.revision,
            draft("Alpine Git", "alpine/git", "apk add curl\n"),
        )
        .expect("update");
    let queued = updated.preparation.expect("queued");
    assert_eq!(queued.ordinal, 2);
    assert_eq!(
        catalogue.preparation(&first.id).expect("first").state,
        PreparationState::Superseded
    );
    assert_eq!(updated.environment.latest_preparation, queued.id);
    assert!(updated.environment.ready_preparation.is_none());
}

#[test]
fn stale_commands_conflict() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (created, _) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    catalogue
        .update(
            &created.id,
            created.revision,
            draft("Renamed", "alpine/git", ""),
        )
        .expect("update");
    assert_eq!(
        catalogue
            .update(
                &created.id,
                created.revision,
                draft("Later", "alpine/git", "")
            )
            .err(),
        Some(EnvironmentError::Conflict)
    );
    assert_eq!(
        catalogue
            .retry_preparation(&created.id, created.revision, &created.recipe_version)
            .err(),
        Some(EnvironmentError::Conflict)
    );
    assert_eq!(
        catalogue.delete(&created.id, created.revision).err(),
        Some(EnvironmentError::Conflict)
    );
}

#[test]
fn retry_requires_the_current_recipe_and_increments_ordinal() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (created, first) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    assert_eq!(
        catalogue
            .retry_preparation(&created.id, created.revision, &created.recipe_version)
            .err(),
        Some(EnvironmentError::Busy)
    );
    catalogue
        .claim_oldest_queued()
        .expect("claim")
        .expect("item");
    catalogue
        .finish_failed(&first.id, FailureCategory::SetupExit, first.log)
        .expect("fail");
    let current = catalogue.get(&created.id).expect("current");
    let retried = catalogue
        .retry_preparation(&current.id, current.revision, &current.recipe_version)
        .expect("retry");
    assert_eq!(retried.ordinal, 2);
    assert_eq!(retried.recipe_version, created.recipe_version);
}

#[test]
fn delete_retires_the_identifier_and_keeps_preparations() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (created, first) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    catalogue
        .delete(&created.id, created.revision)
        .expect("delete");
    assert!(catalogue.get(&created.id).is_none());
    assert!(catalogue.retired_ids().contains(&created.id));
    assert_eq!(
        catalogue.preparation(&first.id).expect("kept").state,
        PreparationState::Cancelled
    );
    let (later, _) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("recreate");
    assert_ne!(later.id, created.id);
}

#[test]
fn ready_pointer_survives_a_failed_replacement() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (created, first) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    catalogue.claim_oldest_queued().expect("claim");
    let snapshot = sample_snapshot(first.id);
    catalogue
        .finish_ready(&first.id, snapshot, first.log)
        .expect("ready");
    let ready = catalogue.get(&created.id).expect("ready");
    assert_eq!(ready.ready_preparation, Some(first.id));
    let updated = catalogue
        .update(
            &ready.id,
            ready.revision,
            draft("Alpine Git", "alpine/git", "false\n"),
        )
        .expect("replace");
    let replacement = updated.preparation.expect("replacement");
    catalogue.claim_oldest_queued().expect("claim replacement");
    catalogue
        .finish_failed(&replacement.id, FailureCategory::SetupExit, replacement.log)
        .expect("fail");
    let after = catalogue.get(&created.id).expect("after");
    assert_eq!(after.ready_preparation, Some(first.id));
}

#[test]
fn stale_success_does_not_activate_after_deletion() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (created, first) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    catalogue.claim_oldest_queued().expect("claim");
    catalogue
        .delete(&created.id, created.revision)
        .expect("delete");
    let snapshot = sample_snapshot(first.id);
    assert_eq!(
        catalogue.finish_ready(&first.id, snapshot, first.log).err(),
        Some(EnvironmentError::Conflict)
    );
    let finished = catalogue.preparation(&first.id).expect("cancelled");
    assert_eq!(finished.state, PreparationState::Cancelled);
    assert!(finished.snapshot.is_none());
}

#[test]
fn delete_cancels_an_active_preparation() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (created, preparation) = catalogue
        .create(draft("Alpine Git", "alpine/git", ""))
        .expect("create");
    catalogue
        .claim_oldest_queued()
        .expect("claim")
        .expect("preparation");

    catalogue
        .delete(&created.id, created.revision)
        .expect("delete");

    let cancelled = catalogue
        .preparation(&preparation.id)
        .expect("cancelled preparation");
    assert_eq!(cancelled.state, PreparationState::Cancelled);
    assert_eq!(cancelled.phase, PreparationPhase::Finished);
    assert!(cancelled.finished_at_ms.is_some());
}

#[test]
fn scheduler_claims_oldest_queued_and_keeps_order() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let (_, first) = catalogue
        .create(draft("One", "alpine/git", ""))
        .expect("one");
    let (_, second) = catalogue
        .create(draft("Two", "alpine/git", ""))
        .expect("two");
    let claimed = catalogue
        .claim_oldest_queued()
        .expect("claim")
        .expect("item");
    assert_eq!(claimed.state, PreparationState::Preparing);
    assert_eq!(claimed.phase, PreparationPhase::CreatingGuest);
    let next = catalogue
        .claim_oldest_queued()
        .expect("next")
        .expect("item");
    let ids = [claimed.id, next.id];
    assert!(ids.contains(&first.id));
    assert!(ids.contains(&second.id));
    assert!(catalogue.claim_oldest_queued().expect("empty").is_none());
}

#[test]
fn restart_interrupts_preparing_work_and_keeps_queued_work() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("environments.json");
    let logs = dir.path().join("logs");
    let first_id;
    let second_id;
    {
        let catalogue =
            EnvironmentCatalogue::open_with_seeds(path.clone(), logs.clone(), &[]).expect("open");
        let (_, first) = catalogue
            .create(draft("One", "alpine/git", ""))
            .expect("one");
        let (_, second) = catalogue
            .create(draft("Two", "alpine/git", ""))
            .expect("two");
        let claimed = catalogue
            .claim_oldest_queued()
            .expect("claim")
            .expect("item");
        first_id = claimed.id;
        second_id = if claimed.id == first.id {
            second.id
        } else {
            first.id
        };
    }
    let catalogue = EnvironmentCatalogue::open_with_seeds(path, logs, &[]).expect("reopen");
    assert_eq!(
        catalogue.preparation(&first_id).expect("first").state,
        PreparationState::Interrupted
    );
    assert_eq!(
        catalogue
            .preparation(&first_id)
            .expect("first")
            .failure
            .expect("failure")
            .category,
        FailureCategory::ProcessRestarted
    );
    assert_eq!(
        catalogue.preparation(&second_id).expect("second").state,
        PreparationState::Queued
    );
}

#[test]
fn persist_survives_reopen() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("environments.json");
    let logs = dir.path().join("logs");
    let id;
    {
        let catalogue =
            EnvironmentCatalogue::open_with_seeds(path.clone(), logs.clone(), &[]).expect("open");
        let (record, _) = catalogue
            .create(draft("Planner", "alpine/git", "apk add git\n"))
            .expect("create");
        id = record.id;
    }
    let catalogue = EnvironmentCatalogue::open_with_seeds(path, logs, &[]).expect("reopen");
    let loaded = catalogue.get(&id).expect("loaded");
    assert_eq!(loaded.name, "Planner");
    assert_eq!(loaded.recipe.setup_script, "apk add git\n");
}

#[test]
fn corrupt_files_fail_open() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("environments.json");
    let logs = dir.path().join("logs");
    write_catalogue(&path, "{");
    assert_eq!(
        EnvironmentCatalogue::open_with_seeds(path, logs, &[]).err(),
        Some(EnvironmentError::Corrupt)
    );
}

#[test]
fn ready_pointers_must_name_a_ready_preparation_for_the_same_environment() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("environments.json");
    let logs = dir.path().join("logs");
    let env = EnvironmentId::generate().expect("env");
    let prep = PreparationId::generate().expect("prep");
    let other = PreparationId::generate().expect("other");
    let json = serde_json::json!({
        "file-version": 1,
        "applied-seeds": [],
        "retired-environment-ids": [],
        "environments": [{
            "id": env.as_hex(),
            "revision": 1,
            "name": "Alpine Git",
            "recipe": { "oci-image": "alpine/git", "setup-script": "" },
            "recipe-version": crate::environments::recipe::EnvironmentRecipe::from_draft(&draft("Alpine Git", "alpine/git", "")).expect("recipe").1.version().as_hex(),
            "ready-preparation": other.as_hex(),
            "latest-preparation": prep.as_hex(),
            "created-at-ms": 1,
            "updated-at-ms": 1
        }],
        "preparations": [{
            "id": prep.as_hex(),
            "environment-id": env.as_hex(),
            "ordinal": 1,
            "environment-revision": 1,
            "recipe-version": crate::environments::recipe::EnvironmentRecipe::from_draft(&draft("Alpine Git", "alpine/git", "")).expect("recipe").1.version().as_hex(),
            "state": "queued",
            "phase": "waiting",
            "requested-at-ms": 1,
            "started-at-ms": null,
            "finished-at-ms": null,
            "log": { "captured-bytes": 0, "truncated": false },
            "failure": null,
            "snapshot": null
        }]
    });
    write_catalogue(&path, &json.to_string());
    assert_eq!(
        EnvironmentCatalogue::open_with_seeds(path, logs, &[]).err(),
        Some(EnvironmentError::Corrupt)
    );
}

#[test]
fn snapshot_artifact_keys_must_match_their_preparation() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("environments.json");
    let logs = dir.path().join("logs");
    {
        let catalogue =
            EnvironmentCatalogue::open_with_seeds(path.clone(), logs.clone(), &[]).expect("open");
        let (_, preparation) = catalogue
            .create(draft("Alpine Git", "alpine/git", ""))
            .expect("create");
        catalogue.claim_oldest_queued().expect("claim");
        catalogue
            .finish_ready(
                &preparation.id,
                sample_snapshot(preparation.id),
                preparation.log,
            )
            .expect("ready");
    }
    let mut file: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
    file["preparations"][0]["snapshot"]["artifact-key"] =
        serde_json::Value::String(PreparationId::generate().expect("other").as_hex());
    std::fs::write(&path, serde_json::to_vec(&file).expect("encode")).expect("write");

    assert_eq!(
        EnvironmentCatalogue::open_with_seeds(path, logs, &[]).err(),
        Some(EnvironmentError::Corrupt)
    );
}

#[test]
fn missing_logs_are_corrupt_when_bytes_were_captured() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("environments.json");
    let logs = dir.path().join("logs");
    std::fs::create_dir_all(&logs).expect("logs");
    let env = EnvironmentId::generate().expect("env");
    let prep = PreparationId::generate().expect("prep");
    let version = crate::environments::recipe::EnvironmentRecipe::from_draft(&draft(
        "Alpine Git",
        "alpine/git",
        "",
    ))
    .expect("recipe")
    .1
    .version()
    .as_hex();
    let json = serde_json::json!({
        "file-version": 1,
        "applied-seeds": [],
        "retired-environment-ids": [],
        "environments": [{
            "id": env.as_hex(),
            "revision": 1,
            "name": "Alpine Git",
            "recipe": { "oci-image": "alpine/git", "setup-script": "" },
            "recipe-version": version,
            "ready-preparation": null,
            "latest-preparation": prep.as_hex(),
            "created-at-ms": 1,
            "updated-at-ms": 1
        }],
        "preparations": [{
            "id": prep.as_hex(),
            "environment-id": env.as_hex(),
            "ordinal": 1,
            "environment-revision": 1,
            "recipe-version": version,
            "state": "queued",
            "phase": "waiting",
            "requested-at-ms": 1,
            "started-at-ms": null,
            "finished-at-ms": null,
            "log": { "captured-bytes": 12, "truncated": false },
            "failure": null,
            "snapshot": null
        }]
    });
    write_catalogue(&path, &json.to_string());
    assert_eq!(
        EnvironmentCatalogue::open_with_seeds(path, logs, &[]).err(),
        Some(EnvironmentError::Corrupt)
    );
}

#[test]
fn refresh_cursors_treat_missing_and_earlier_generations_as_stale() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let current = catalogue.refresh_cursor();
    assert!(catalogue.cursor_is_current(current));
    assert!(catalogue.cursor_is_stale(None));
    assert!(catalogue.cursor_is_stale(Some(super::RefreshCursor {
        generation: current.generation.wrapping_sub(1),
        sequence: current.sequence,
    })));
    catalogue.bump_refresh();
    assert!(!catalogue.cursor_is_current(current));
    assert!(EnvironmentCatalogue::parse_refresh_cursor("not-a-cursor").is_none());
}

#[test]
fn the_active_catalogue_rejects_more_than_the_bound() {
    let catalogue = EnvironmentCatalogue::in_memory();
    for index in 0..MAXIMUM_ENVIRONMENTS {
        catalogue
            .create(draft(&format!("Env {index}"), "alpine/git", ""))
            .expect("create");
    }
    assert_eq!(
        catalogue.create(draft("Overflow", "alpine/git", "")).err(),
        Some(EnvironmentError::Full)
    );
}
