use super::*;
use crate::agents::{AccessMode, AgentId, AgentRecord, DirectoryGrant, NetworkAccess, ToolId};
use crate::projects::{ProjectId, ProjectRecord};

fn project(path: &std::path::Path) -> ProjectRecord {
    ProjectRecord {
        id: ProjectId::generate().expect("project"),
        revision: 4,
        name: "Project".to_owned(),
        host_path: path.to_path_buf(),
        created_at_ms: 1,
    }
}

fn preset(path: &std::path::Path, tools: Vec<ToolId>, network: NetworkAccess) -> AgentRecord {
    AgentRecord {
        id: AgentId::generate().expect("agent"),
        revision: 2,
        name: "Preset".to_owned(),
        instructions: String::new(),
        selection: None,
        tools,
        network,
        directories: vec![DirectoryGrant {
            alias: "project".to_owned(),
            host_path: path.to_path_buf(),
            access: AccessMode::ReadOnly,
        }],
        primary_directory: "project".to_owned(),
    }
}

#[test]
fn a_direct_conversation_grant_has_the_read_only_ceiling() {
    let directory = tempfile::tempdir().expect("directory");
    let project = project(directory.path());
    let conversation = crate::conversations::ConversationId::generate().expect("conversation");
    let grant = ConversationGrant {
        project_id: project.id,
        project_revision: project.revision,
        authority_revision: 1,
        access: AccessMode::ReadOnly,
    };
    let authority = resolve_grant(&grant, &project, conversation, 1, None)
        .expect("authority")
        .effective;

    assert_eq!(
        authority.tools,
        vec![ToolId::List, ToolId::Read, ToolId::Run]
    );
    assert_eq!(authority.grant_access, AccessMode::ReadOnly);
    assert_eq!(authority.network, NetworkAccess::None);
    assert_eq!(authority.policy.primary_guest(), "/project");
}

#[test]
fn an_empty_preset_tool_ceiling_blocks_guest_tools_without_granting_network() {
    let directory = tempfile::tempdir().expect("directory");
    let project = project(directory.path());
    let preset = preset(directory.path(), Vec::new(), NetworkAccess::Public);
    let grant = ConversationGrant {
        project_id: project.id,
        project_revision: project.revision,
        authority_revision: 1,
        access: AccessMode::ReadOnly,
    };
    let authority = resolve_grant(
        &grant,
        &project,
        crate::conversations::ConversationId::generate().expect("conversation"),
        1,
        Some(&preset),
    )
    .expect("authority")
    .effective;

    assert!(authority.tools.is_empty());
    assert_eq!(authority.network, NetworkAccess::None);
}

#[test]
fn a_changed_project_revision_invalidates_the_candidate_authority() {
    let directory = tempfile::tempdir().expect("directory");
    let project = project(directory.path());
    let grant = ConversationGrant {
        project_id: project.id,
        project_revision: project.revision,
        authority_revision: 1,
        access: AccessMode::ReadOnly,
    };
    let authority = resolve_grant(
        &grant,
        &project,
        crate::conversations::ConversationId::generate().expect("conversation"),
        1,
        None,
    )
    .expect("authority")
    .effective;
    let mut changed = project.clone();
    changed.revision += 1;

    assert_eq!(
        authority.revalidate_project(&changed),
        Err(AuthorityError::Stale)
    );
}

#[test]
fn a_preset_directory_ceiling_must_match_the_canonical_project() {
    let project_dir = tempfile::tempdir().expect("project");
    let other_dir = tempfile::tempdir().expect("other");
    let project = project(project_dir.path());
    let preset = preset(other_dir.path(), ToolId::ALL.to_vec(), NetworkAccess::None);
    let grant = ConversationGrant {
        project_id: project.id,
        project_revision: project.revision,
        authority_revision: 1,
        access: AccessMode::ReadOnly,
    };

    assert_eq!(
        resolve_grant(
            &grant,
            &project,
            crate::conversations::ConversationId::generate().expect("conversation"),
            1,
            Some(&preset),
        )
        .err(),
        Some(ConversationAccessError::MissingGrant)
    );
}
