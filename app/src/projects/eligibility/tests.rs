use std::path::PathBuf;

use crate::agents::{AccessMode, AgentId, AgentRecord, DirectoryGrant, ToolId};
use crate::projects::{ProjectId, ProjectRecord, eligibility, eligible_agents, exact_grant};

fn grant(alias: &str, path: &str) -> DirectoryGrant {
    DirectoryGrant {
        alias: alias.to_owned(),
        host_path: PathBuf::from(path),
        access: AccessMode::ReadWrite,
    }
}

fn agent(grants: Vec<DirectoryGrant>) -> AgentRecord {
    let primary = grants
        .first()
        .map(|grant| grant.alias.clone())
        .unwrap_or_else(|| "project".to_owned());
    AgentRecord {
        id: AgentId::generate().expect("agent"),
        revision: 1,
        name: "Agent".to_owned(),
        instructions: String::new(),
        tools: vec![ToolId::List],
        network: crate::agents::NetworkAccess::None,
        directories: grants,
        primary_directory: primary,
    }
}

fn project(path: &str) -> ProjectRecord {
    ProjectRecord {
        id: ProjectId::generate().expect("project"),
        revision: 1,
        name: "Desk".to_owned(),
        host_path: PathBuf::from(path),
        created_at_ms: 1,
    }
}

#[test]
fn eligibility_returns_the_exact_grant_alias_and_access() {
    let project = project("/tmp/code");
    let agent = agent(vec![
        DirectoryGrant {
            alias: "docs".to_owned(),
            host_path: PathBuf::from("/tmp/docs"),
            access: AccessMode::ReadOnly,
        },
        DirectoryGrant {
            alias: "code".to_owned(),
            host_path: PathBuf::from("/tmp/code"),
            access: AccessMode::ReadWrite,
        },
    ]);
    let grant = eligibility(&agent, &project).expect("eligible");
    assert_eq!(grant.alias, "code");
    assert_eq!(grant.access, AccessMode::ReadWrite);
    assert_eq!(
        exact_grant(&agent, &project).map(|item| item.alias.as_str()),
        Some("code")
    );
}

#[test]
fn eligibility_ignores_prefix_paths() {
    let project = project("/tmp/code");
    let parent = agent(vec![grant("parent", "/tmp")]);
    let prefix = agent(vec![grant("prefix", "/tmp/co")]);
    let nested = agent(vec![grant("nested", "/tmp/code/src")]);
    let sibling = agent(vec![grant("sibling", "/tmp/code-extra")]);
    assert!(eligibility(&parent, &project).is_none());
    assert!(eligibility(&prefix, &project).is_none());
    assert!(eligibility(&nested, &project).is_none());
    assert!(eligibility(&sibling, &project).is_none());
}

#[test]
fn eligible_agents_keep_catalogue_order() {
    let project = project("/tmp/code");
    let first = agent(vec![grant("project", "/tmp/other")]);
    let second = agent(vec![grant("project", "/tmp/code")]);
    let third = agent(vec![grant("source", "/tmp/code")]);
    let listed = eligible_agents(&[first.clone(), second.clone(), third.clone()], &project);
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, second.id);
    assert_eq!(listed[1].id, third.id);
}
