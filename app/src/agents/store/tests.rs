use std::fs;

use super::AgentStore;
use crate::agents::record::{AccessMode, AgentDraft, AgentError, DirectoryGrant, MAXIMUM_AGENTS};
use crate::agents::tool_id::ToolId;

fn draft(dir: &std::path::Path, name: &str) -> AgentDraft {
    AgentDraft {
        name: name.to_owned(),
        instructions: String::new(),
        tools: vec![ToolId::List],
        directories: vec![DirectoryGrant {
            alias: "project".to_owned(),
            host_path: dir.to_path_buf(),
            access: AccessMode::ReadWrite,
        }],
        primary_directory: "project".to_owned(),
    }
}

#[test]
fn import_from_legacy_project_is_idempotent() {
    let data = tempfile::tempdir().expect("data");
    let project_dir = tempfile::tempdir().expect("project");
    let project_file = data.path().join("project.json");
    let agents_dir = data.path().join("agents");
    fs::write(
        &project_file,
        format!(
            "{{\n  \"version\": 1,\n  \"path\": \"{}\"\n}}\n",
            project_dir.path().display()
        ),
    )
    .expect("legacy");

    let first = AgentStore::open(agents_dir.clone(), &project_file).expect("import");
    assert_eq!(first.count(), 1);
    assert!(!project_file.exists());
    let imported = first.list();
    assert_eq!(imported[0].name, "Default agent");
    assert!(imported[0].instructions.is_empty());
    assert_eq!(imported[0].tools.len(), ToolId::ALL.len());

    fs::write(
        &project_file,
        format!(
            "{{\n  \"version\": 1,\n  \"path\": \"{}\"\n}}\n",
            project_dir.path().display()
        ),
    )
    .expect("legacy again");
    let second = AgentStore::open(agents_dir, &project_file).expect("reopen");
    assert_eq!(second.count(), 1);
    assert_eq!(second.list()[0].id, imported[0].id);
    assert!(!project_file.exists());
}

#[test]
fn corrupt_record_fails_startup() {
    let data = tempfile::tempdir().expect("data");
    let agents_dir = data.path().join("agents");
    fs::create_dir_all(&agents_dir).expect("dir");
    fs::write(agents_dir.join("not-an-id.json"), b"{not json").expect("corrupt");
    let project_file = data.path().join("project.json");
    assert_eq!(
        AgentStore::open(agents_dir, &project_file).err(),
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
    let updated = store.update(&created.id, next).expect("update");
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
