use super::{MAXIMUM_TOOL_BYTES, mark_truncated, redact};
use crate::agents::{AccessMode, AgentId, AgentRecord, DirectoryGrant, DirectoryPolicy, ToolId};

fn policy() -> DirectoryPolicy {
    DirectoryPolicy::from_record_with_primary(
        &AgentRecord {
            id: AgentId::generate().expect("id"),
            revision: 1,
            name: "Agent".to_owned(),
            instructions: String::new(),
            tools: ToolId::ALL.to_vec(),
            network: crate::agents::NetworkAccess::None,
            directories: vec![
                DirectoryGrant {
                    alias: "project".to_owned(),
                    host_path: "/tmp/project".into(),
                    access: AccessMode::ReadWrite,
                },
                DirectoryGrant {
                    alias: "docs".to_owned(),
                    host_path: "/tmp/docs".into(),
                    access: AccessMode::ReadOnly,
                },
            ],
            primary_directory: "project".to_owned(),
        },
        "project",
    )
}

#[test]
fn guest_path_accepts_each_grant() {
    let policy = policy();
    assert_eq!(policy.resolve("").expect("default").0, "/project");
    assert_eq!(
        policy.resolve("src/main.rs").expect("relative"),
        ("/project/src/main.rs".to_owned(), AccessMode::ReadWrite)
    );
    assert_eq!(
        policy.resolve("/access/docs/readme").expect("docs").0,
        "/access/docs/readme"
    );
}

#[test]
fn guest_path_rejects_escape_and_control() {
    let policy = policy();
    assert_eq!(
        policy.resolve(".."),
        Err("Stay inside a granted directory.")
    );
    assert_eq!(
        policy.resolve("/etc/passwd"),
        Err("Stay inside a granted directory.")
    );
    assert_eq!(
        policy.resolve("/project/../../secret"),
        Err("Stay inside a granted directory.")
    );
    assert_eq!(
        policy.resolve("/tmp/\u{0000}x"),
        Err("That path is not valid.")
    );
}

#[test]
fn write_paths_are_read_only_outside_writable_grants() {
    let policy = policy();
    assert_eq!(
        policy.resolve("/access/docs/readme").expect("docs").1,
        AccessMode::ReadOnly
    );
    assert!(
        policy
            .resolve("/project/note.txt")
            .expect("write")
            .1
            .is_writable()
    );
}

#[test]
fn redact_removes_the_vault_secret() {
    assert_eq!(
        redact("token sk-secret in output", Some("sk-secret")),
        "token [redacted] in output"
    );
    assert_eq!(redact("plain", None), "plain");
    assert_eq!(redact("plain", Some("")), "plain");
}

#[test]
fn truncated_tool_output_carries_a_bounded_marker() {
    let mut output = "x".repeat(MAXIMUM_TOOL_BYTES);
    mark_truncated(&mut output);
    assert_eq!(output.len(), MAXIMUM_TOOL_BYTES);
    assert!(output.ends_with("[output truncated]"));
}
