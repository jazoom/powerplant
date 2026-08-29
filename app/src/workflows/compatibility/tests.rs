use crate::agents::{AccessMode, AgentId, AgentRecord, DirectoryGrant, ToolId};

use super::compatibility_definition;

fn record() -> AgentRecord {
    AgentRecord {
        id: AgentId::generate().expect("id"),
        revision: 1,
        name: "Maintainer".to_owned(),
        instructions: "Keep public interfaces stable.".to_owned(),
        tools: vec![ToolId::List, ToolId::Read],
        directories: vec![DirectoryGrant {
            alias: "project".to_owned(),
            host_path: "/home/user/src/secret-repo".into(),
            access: AccessMode::ReadWrite,
        }],
        primary_directory: "project".to_owned(),
    }
}

#[test]
fn compatibility_definition_omits_host_paths_and_credentials() {
    let definition = compatibility_definition(&record()).expect("definition");
    let bytes = serde_json::to_vec(&definition.to_file()).expect("json");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(text.contains("Keep public interfaces stable."));
    assert!(text.contains("assistant-reply"));
    assert!(!text.contains("secret-repo"));
    assert!(!text.contains("/home/user"));
    assert!(!text.contains("api_key"));
    assert!(!text.contains("sk-"));
}

#[test]
fn compatibility_copies_name_and_leaves_expertise_empty() {
    let definition = compatibility_definition(&record()).expect("definition");
    assert_eq!(definition.name(), "Maintainer");
    assert_eq!(definition.roles()[0].name, "Maintainer");
    assert!(definition.roles()[0].expertise.is_empty());
    assert_eq!(
        definition.roles()[0].prompt_defaults,
        "Keep public interfaces stable."
    );
    let pinned = crate::workflows::definition::PinnedWorkflowDefinition::pin(None, definition);
    assert!(pinned.workflow_id.is_none());
}
