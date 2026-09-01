use super::compose_role;
use crate::agents::policy::DirectoryPolicy;
use crate::agents::record::{AccessMode, AgentRecord, DirectoryGrant};
use crate::agents::{AgentId, ToolId};

#[test]
fn composed_preamble_omits_host_paths() {
    let record = AgentRecord {
        id: AgentId::generate().expect("id"),
        revision: 2,
        name: "Maintainer".to_owned(),
        instructions: "Keep public interfaces stable.".to_owned(),
        tools: vec![ToolId::List, ToolId::Read],
        directories: vec![DirectoryGrant {
            alias: "project".to_owned(),
            host_path: "/home/user/src/secret-repo".into(),
            access: AccessMode::ReadWrite,
        }],
        primary_directory: "project".to_owned(),
    };
    let policy = DirectoryPolicy::from_record_with_primary(&record, &record.primary_directory);
    let preamble = compose_role(
        &record.name,
        "",
        &record.instructions,
        &record.tools,
        &policy,
    );
    assert!(preamble.contains("Power Plant contract"));
    assert!(preamble.contains("Keep public interfaces stable."));
    assert!(preamble.contains("/project"));
    assert!(preamble.contains("- list"));
    assert!(preamble.contains("- read"));
    assert!(!preamble.contains("secret-repo"));
    assert!(!preamble.contains("/home/user"));
}
