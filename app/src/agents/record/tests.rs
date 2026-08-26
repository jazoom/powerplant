use std::path::PathBuf;

use super::{
    AccessMode, AgentDraft, AgentError, AgentRecord, DirectoryGrant, MAXIMUM_GRANTS,
    MAXIMUM_INSTRUCTION_BYTES, MAXIMUM_NAME_BYTES, canonical_directory,
};
use crate::agents::id::AgentId;
use crate::agents::tool_id::ToolId;

fn draft(dir: &std::path::Path) -> AgentDraft {
    AgentDraft {
        name: "Repository maintainer".to_owned(),
        instructions: String::new(),
        tools: ToolId::ALL.to_vec(),
        directories: vec![DirectoryGrant {
            alias: "project".to_owned(),
            host_path: dir.to_path_buf(),
            access: AccessMode::ReadWrite,
        }],
        primary_directory: "project".to_owned(),
    }
}

#[test]
fn valid_draft_canonicalises_the_host_path() {
    let dir = tempfile::tempdir().expect("dir");
    let validated = draft(dir.path()).validate().expect("valid");
    assert_eq!(
        validated.directories[0].host_path,
        dir.path().canonicalize().expect("canonical")
    );
}

#[test]
fn bounds_reject_empty_and_oversized_names() {
    let dir = tempfile::tempdir().expect("dir");
    let mut item = draft(dir.path());
    item.name = "  ".to_owned();
    assert_eq!(item.validate().err(), Some(AgentError::Name));
    let mut item = draft(dir.path());
    item.name = "a".repeat(MAXIMUM_NAME_BYTES + 1);
    assert_eq!(item.validate().err(), Some(AgentError::Name));
}

#[test]
fn instructions_may_be_empty_but_not_oversized() {
    let dir = tempfile::tempdir().expect("dir");
    assert!(draft(dir.path()).validate().is_ok());
    let mut item = draft(dir.path());
    item.instructions = "a".repeat(MAXIMUM_INSTRUCTION_BYTES + 1);
    assert_eq!(item.validate().err(), Some(AgentError::Instructions));
}

#[test]
fn unknown_or_duplicate_tools_are_rejected() {
    let dir = tempfile::tempdir().expect("dir");
    let mut item = draft(dir.path());
    item.tools = vec![ToolId::List, ToolId::List];
    assert_eq!(item.validate().err(), Some(AgentError::Tools));
}

#[test]
fn write_tools_need_a_writable_grant() {
    let dir = tempfile::tempdir().expect("dir");
    let mut item = draft(dir.path());
    item.directories[0].access = AccessMode::ReadOnly;
    item.tools = vec![ToolId::Write];
    assert_eq!(
        item.clone().validate().err(),
        Some(AgentError::ToolConflict)
    );
    item.tools = vec![ToolId::List, ToolId::Read];
    assert!(item.validate().is_ok());
}

#[test]
fn overlapping_host_grants_are_rejected() {
    let dir = tempfile::tempdir().expect("dir");
    let nested = dir.path().join("nested");
    std::fs::create_dir(&nested).expect("nested");
    let mut item = draft(dir.path());
    item.directories.push(DirectoryGrant {
        alias: "nested".to_owned(),
        host_path: nested,
        access: AccessMode::ReadOnly,
    });
    assert_eq!(item.validate().err(), Some(AgentError::NestedPath));
}

#[test]
fn duplicate_aliases_are_rejected() {
    let dir = tempfile::tempdir().expect("dir");
    let other = tempfile::tempdir().expect("other");
    let mut item = draft(dir.path());
    item.directories.push(DirectoryGrant {
        alias: "project".to_owned(),
        host_path: other.path().to_path_buf(),
        access: AccessMode::ReadOnly,
    });
    assert_eq!(item.validate().err(), Some(AgentError::DuplicateAlias));
}

#[test]
fn grant_count_is_bounded() {
    let mut dirs = Vec::new();
    for _ in 0..=MAXIMUM_GRANTS {
        dirs.push(tempfile::tempdir().expect("dir"));
    }
    let directories = dirs
        .iter()
        .enumerate()
        .map(|(index, dir)| DirectoryGrant {
            alias: format!("d{index}"),
            host_path: dir.path().to_path_buf(),
            access: AccessMode::ReadWrite,
        })
        .collect();
    let item = AgentDraft {
        name: "Many".to_owned(),
        instructions: String::new(),
        tools: vec![ToolId::List],
        directories,
        primary_directory: "d0".to_owned(),
    };
    assert_eq!(item.validate().err(), Some(AgentError::GrantCount));
}

#[test]
fn relative_and_missing_paths_are_rejected() {
    let mut item = AgentDraft {
        name: "Agent".to_owned(),
        instructions: String::new(),
        tools: Vec::new(),
        directories: vec![DirectoryGrant {
            alias: "project".to_owned(),
            host_path: PathBuf::from("relative"),
            access: AccessMode::ReadWrite,
        }],
        primary_directory: "project".to_owned(),
    };
    assert_eq!(item.clone().validate().err(), Some(AgentError::Path));
    item.directories[0].host_path = PathBuf::from("/no/such/powerplant-agent-dir");
    assert_eq!(item.validate().err(), Some(AgentError::PathMissing));
}

#[test]
fn file_round_trip_keeps_the_identifier() {
    let dir = tempfile::tempdir().expect("dir");
    let validated = draft(dir.path()).validate().expect("valid");
    let record = AgentRecord {
        id: AgentId::generate().expect("id"),
        revision: 3,
        name: validated.name,
        instructions: validated.instructions,
        tools: validated.tools,
        directories: validated.directories,
        primary_directory: validated.primary_directory,
    };
    let restored = AgentRecord::from_file(record.to_file()).expect("file");
    assert_eq!(restored.id, record.id);
    assert_eq!(restored.revision, 3);
    assert_eq!(restored.name, "Repository maintainer");
}

#[test]
fn canonical_directory_rejects_a_file() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("file");
    std::fs::write(&file, b"x").expect("file");
    assert_eq!(
        canonical_directory(&file).err(),
        Some(AgentError::NotADirectory)
    );
}

#[test]
fn aliases_must_start_with_a_letter() {
    let dir = tempfile::tempdir().expect("dir");
    let mut item = draft(dir.path());
    item.directories[0].alias = "1docs".to_owned();
    assert_eq!(item.validate().err(), Some(AgentError::Alias));
    let mut item = draft(dir.path());
    item.directories[0].alias = "docs/src".to_owned();
    assert_eq!(item.validate().err(), Some(AgentError::Alias));
}
