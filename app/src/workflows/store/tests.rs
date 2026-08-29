use std::fs;

use super::{StoreError, WorkflowRunStore};
use crate::workflows::definition::{
    PinnedWorkflowDefinition, WorkflowDefinition, test_named_definition,
};
use crate::workflows::id::{AttemptId, RunId};
use crate::workflows::run::{RunState, WorkflowRun};

fn definition(name: &str) -> WorkflowDefinition {
    test_named_definition(name)
}

fn run_named(name: &str, created_at_ms: u64) -> WorkflowRun {
    let definition = definition(name);
    let environments = crate::workflows::test_environment_set(&definition);
    WorkflowRun::create(
        RunId::generate().expect("run"),
        created_at_ms,
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    )
}

#[test]
fn a_source_definition_edit_cannot_alter_an_earlier_run() {
    let dir = tempfile::tempdir().expect("dir");
    let store = WorkflowRunStore::open(dir.path().to_path_buf()).expect("open");
    let first = definition("Original");
    let version = first.version();
    let environments = crate::workflows::test_environment_set(&first);
    let run = store
        .create(WorkflowRun::create(
            RunId::generate().expect("run"),
            1,
            PinnedWorkflowDefinition::pin(None, first),
            environments,
        ))
        .expect("create");
    let later = definition("Edited");
    assert_ne!(later.version(), version);
    let loaded = WorkflowRunStore::open(dir.path().to_path_buf())
        .expect("reopen")
        .get(&run.id)
        .expect("run");
    assert_eq!(loaded.pinned.version, version);
    assert_eq!(loaded.pinned.definition.name(), "Original");
}

#[test]
fn summaries_sort_by_creation_time_then_identifier() {
    let store = WorkflowRunStore::in_memory();
    let later = store.create(run_named("Later", 20)).expect("later");
    let earlier = store.create(run_named("Earlier", 10)).expect("earlier");
    let summaries = store.summaries();
    assert_eq!(summaries[0].id, earlier.id);
    assert_eq!(summaries[1].id, later.id);
}

#[test]
fn recovery_marks_active_work_as_interrupted() {
    let dir = tempfile::tempdir().expect("dir");
    let store = WorkflowRunStore::open(dir.path().to_path_buf()).expect("open");
    let run = store.create(run_named("Active", 1)).expect("create");
    let attempt = AttemptId::generate().expect("attempt");
    store
        .mutate(&run.id, |run| run.start_attempt(attempt, 2))
        .expect("start");
    let reopened = WorkflowRunStore::open(dir.path().to_path_buf()).expect("reopen");
    let loaded = reopened.get(&run.id).expect("run");
    assert_eq!(loaded.state, RunState::Interrupted);
    assert_eq!(
        loaded.attempts[0].state,
        crate::workflows::run::AttemptState::Interrupted
    );
}

#[test]
fn malformed_json_fails_startup() {
    let dir = tempfile::tempdir().expect("dir");
    fs::write(
        dir.path().join(format!("{}.json", "a".repeat(32))),
        b"{not json",
    )
    .expect("write");
    assert_eq!(
        WorkflowRunStore::open(dir.path().to_path_buf()).err(),
        Some(StoreError::Corrupt)
    );
}

#[test]
fn filename_disagreement_fails_startup() {
    let dir = tempfile::tempdir().expect("dir");
    let store = WorkflowRunStore::open(dir.path().to_path_buf()).expect("open");
    let run = store.create(run_named("Named", 1)).expect("create");
    let original = dir.path().join(format!("{}.json", run.id.as_hex()));
    let renamed = dir.path().join(format!("{}.json", "b".repeat(32)));
    fs::rename(original, renamed).expect("rename");
    assert_eq!(
        WorkflowRunStore::open(dir.path().to_path_buf()).err(),
        Some(StoreError::Corrupt)
    );
}

#[test]
fn digest_disagreement_fails_startup() {
    let dir = tempfile::tempdir().expect("dir");
    let store = WorkflowRunStore::open(dir.path().to_path_buf()).expect("open");
    let run = store.create(run_named("Named", 1)).expect("create");
    let path = dir.path().join(format!("{}.json", run.id.as_hex()));
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
    value["version"] = serde_json::Value::String("0".repeat(64));
    fs::write(&path, serde_json::to_vec(&value).expect("bytes")).expect("write");
    assert_eq!(
        WorkflowRunStore::open(dir.path().to_path_buf()).err(),
        Some(StoreError::Corrupt)
    );
}

#[test]
fn invalid_states_fail_startup() {
    let dir = tempfile::tempdir().expect("dir");
    let store = WorkflowRunStore::open(dir.path().to_path_buf()).expect("open");
    let run = store.create(run_named("Named", 1)).expect("create");
    let path = dir.path().join(format!("{}.json", run.id.as_hex()));
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
    value["state"] = serde_json::json!({"type": "completed"});
    fs::write(&path, serde_json::to_vec(&value).expect("bytes")).expect("write");
    assert_eq!(
        WorkflowRunStore::open(dir.path().to_path_buf()).err(),
        Some(StoreError::Corrupt)
    );
}

#[test]
fn non_monotonic_transitions_fail_startup() {
    let dir = tempfile::tempdir().expect("dir");
    let store = WorkflowRunStore::open(dir.path().to_path_buf()).expect("open");
    let run = store.create(run_named("Named", 1)).expect("create");
    let attempt = AttemptId::generate().expect("attempt");
    store
        .mutate(&run.id, |run| run.start_attempt(attempt, 2))
        .expect("start");
    let path = dir.path().join(format!("{}.json", run.id.as_hex()));
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
    value["transitions"][0]["sequence"] = serde_json::json!(3);
    fs::write(&path, serde_json::to_vec(&value).expect("bytes")).expect("write");
    assert_eq!(
        WorkflowRunStore::open(dir.path().to_path_buf()).err(),
        Some(StoreError::Corrupt)
    );
}

#[test]
fn a_terminal_attempt_without_a_result_fails_startup() {
    let dir = tempfile::tempdir().expect("dir");
    let store = WorkflowRunStore::open(dir.path().to_path_buf()).expect("open");
    let run = store.create(run_named("Named", 1)).expect("create");
    let attempt = AttemptId::generate().expect("attempt");
    store
        .mutate(&run.id, |run| run.start_attempt(attempt, 2))
        .expect("start");
    store
        .mutate(&run.id, |run| run.complete_attempt(attempt, 3))
        .expect("complete");
    let path = dir.path().join(format!("{}.json", run.id.as_hex()));
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
    value["attempts"][0]["result"] = serde_json::Value::Null;
    fs::write(&path, serde_json::to_vec(&value).expect("bytes")).expect("write");
    assert_eq!(
        WorkflowRunStore::open(dir.path().to_path_buf()).err(),
        Some(StoreError::Corrupt)
    );
}

#[test]
fn a_transition_with_the_wrong_cause_fails_startup() {
    let dir = tempfile::tempdir().expect("dir");
    let store = WorkflowRunStore::open(dir.path().to_path_buf()).expect("open");
    let run = store.create(run_named("Named", 1)).expect("create");
    let attempt = AttemptId::generate().expect("attempt");
    store
        .mutate(&run.id, |run| run.start_attempt(attempt, 2))
        .expect("start");
    store
        .mutate(&run.id, |run| run.complete_attempt(attempt, 3))
        .expect("complete");
    let path = dir.path().join(format!("{}.json", run.id.as_hex()));
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
    value["transitions"][1]["cause"] = serde_json::json!("attempt-failed");
    fs::write(&path, serde_json::to_vec(&value).expect("bytes")).expect("write");
    assert_eq!(
        WorkflowRunStore::open(dir.path().to_path_buf()).err(),
        Some(StoreError::Corrupt)
    );
}

#[test]
fn persisted_bytes_omit_secrets_prompts_and_command_output() {
    let dir = tempfile::tempdir().expect("dir");
    let store = WorkflowRunStore::open(dir.path().to_path_buf()).expect("open");
    let run = store.create(run_named("Named", 1)).expect("create");
    let attempt = AttemptId::generate().expect("attempt");
    store
        .mutate(&run.id, |run| run.start_attempt(attempt, 2))
        .expect("start");
    store
        .mutate(&run.id, |run| run.complete_attempt(attempt, 3))
        .expect("complete");
    let bytes = fs::read(dir.path().join(format!("{}.json", run.id.as_hex()))).expect("read");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(!text.contains("sk-"));
    assert!(!text.contains("api_key"));
    assert!(!text.contains("Hello from the user"));
    assert!(!text.contains("git status"));
    assert!(!text.contains("M src/main.rs"));
}

#[test]
fn owner_only_permissions_cover_the_run_directory() {
    let dir = tempfile::tempdir().expect("dir");
    let runs = dir.path().join("workflow-runs");
    let store = WorkflowRunStore::open(runs.clone()).expect("open");
    store.create(run_named("Named", 1)).expect("create");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&runs).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
