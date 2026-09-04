use super::*;

impl super::AgentStore {
    pub(crate) fn in_memory() -> Self {
        Self {
            dir: None,
            inner: Mutex::new(BTreeMap::new()),
        }
    }
}

use super::AgentStore;
use crate::agents::record::{
    AccessMode, AgentDraft, AgentError, DirectoryGrant, MAXIMUM_AGENTS, NetworkAccess,
};
use crate::agents::tool_id::ToolId;

fn draft(dir: &std::path::Path, name: &str) -> AgentDraft {
    AgentDraft {
        name: name.to_owned(),
        instructions: String::new(),
        tools: vec![ToolId::List],
        network: NetworkAccess::None,
        directories: vec![DirectoryGrant {
            alias: "project".to_owned(),
            host_path: dir.to_path_buf(),
            access: AccessMode::ReadWrite,
        }],
        primary_directory: "project".to_owned(),
    }
}

#[test]
fn corrupt_record_fails_startup() {
    let data = tempfile::tempdir().expect("data");
    let agents_dir = data.path().join("agents");
    std::fs::create_dir_all(&agents_dir).expect("dir");
    std::fs::write(agents_dir.join("not-an-id.json"), b"{not json").expect("corrupt");
    assert_eq!(
        AgentStore::open(agents_dir).err(),
        Some(AgentError::Corrupt)
    );
}

#[test]
fn create_and_update_persist_and_bump_revision() {
    let store = AgentStore::in_memory();
    let dir = tempfile::tempdir().expect("dir");
    let created = store.create(draft(dir.path(), "One")).expect("create");
    assert_eq!(created.revision, 1);
    let mut next = draft(dir.path(), "Two");
    next.instructions = "Keep interfaces stable.".to_owned();
    let updated = store
        .update(&created.id, created.revision, next)
        .expect("update");
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.name, "Two");
    assert_eq!(
        store.get(&created.id).expect("get").instructions,
        "Keep interfaces stable."
    );
}

#[test]
fn catalogue_count_is_bounded() {
    let store = AgentStore::in_memory();
    let mut dirs = Vec::new();
    for index in 0..MAXIMUM_AGENTS {
        let dir = tempfile::tempdir().expect("dir");
        store
            .create(draft(dir.path(), &format!("Agent {index}")))
            .expect("create");
        dirs.push(dir);
    }
    let extra = tempfile::tempdir().expect("extra");
    assert_eq!(
        store.create(draft(extra.path(), "Overflow")).err(),
        Some(AgentError::Full)
    );
    drop(dirs);
}

#[test]
fn stale_update_preserves_the_latest_record() {
    let store = AgentStore::in_memory();
    let dir = tempfile::tempdir().expect("dir");
    let created = store.create(draft(dir.path(), "One")).expect("create");
    let updated = store
        .update(&created.id, created.revision, draft(dir.path(), "Two"))
        .expect("update");
    assert_eq!(
        store
            .update(&created.id, created.revision, draft(dir.path(), "Three"))
            .err(),
        Some(AgentError::Conflict)
    );
    let current = store.get(&created.id).expect("current");
    assert_eq!(current.revision, updated.revision);
    assert_eq!(current.name, "Two");
}

#[test]
fn stale_delete_preserves_the_latest_record() {
    let store = AgentStore::in_memory();
    let dir = tempfile::tempdir().expect("dir");
    let created = store.create(draft(dir.path(), "One")).expect("create");
    let updated = store
        .update(&created.id, created.revision, draft(dir.path(), "Two"))
        .expect("update");
    assert_eq!(
        store.delete(&created.id, created.revision).err(),
        Some(AgentError::Conflict)
    );
    let current = store.get(&created.id).expect("current");
    assert_eq!(current.revision, updated.revision);
    assert_eq!(current.name, "Two");
}
