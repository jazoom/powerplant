use super::{
    AttemptCapabilities, CapabilityError, DirectoryRole, NetworkCapability, SecretPresence,
};
use crate::agents::{AccessMode, AgentId, AgentRecord, DirectoryGrant, ToolId};
use crate::providers::{ProviderConnection, ProviderKind};
use crate::tools::SUBMIT_WORKFLOW_OUTPUT;
use crate::workflows::definition::{
    AgentAuthority, AgentStep, GuestDirectoryAccess, RoleKey, StepAction, StepDefinition,
    StepEnvironment, StepKey, SuccessTransition, SystemCommandId, SystemCommandStep,
};

fn agent(tools: Vec<ToolId>, writable: bool) -> AgentRecord {
    AgentRecord {
        id: AgentId::generate().expect("id"),
        revision: 1,
        name: "Agent".to_owned(),
        instructions: String::new(),
        tools,
        directories: vec![DirectoryGrant {
            alias: "project".to_owned(),
            host_path: "/tmp/project".into(),
            access: if writable {
                AccessMode::ReadWrite
            } else {
                AccessMode::ReadOnly
            },
        }],
        primary_directory: "project".to_owned(),
    }
}

fn connection() -> ProviderConnection {
    ProviderConnection::with_key(ProviderKind::Xai, "sk-test", "grok-4.6")
}

fn agent_step(tools: Vec<ToolId>, writable: bool) -> StepDefinition {
    let authority = AgentAuthority::new(
        tools,
        vec![GuestDirectoryAccess {
            alias: "project".to_owned(),
            access: if writable {
                AccessMode::ReadWrite
            } else {
                AccessMode::ReadOnly
            },
        }],
    )
    .expect("authority");
    StepDefinition {
        key: StepKey::parse("work").expect("step"),
        name: "Work".to_owned(),
        inputs: Vec::new(),
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse("agent").expect("role"),
            environment: StepEnvironment::WorkflowDefault,
            authority,
            required_outputs: Vec::new(),
        }),
        on_success: SuccessTransition::CompleteRun,
    }
}

fn status_step() -> StepDefinition {
    StepDefinition {
        key: StepKey::parse("status").expect("step"),
        name: "Status".to_owned(),
        inputs: Vec::new(),
        action: StepAction::SystemCommand(SystemCommandStep {
            command: SystemCommandId::RepositoryStatus,
            environment: StepEnvironment::WorkflowDefault,
            required_outputs: Vec::new(),
        }),
        on_success: SuccessTransition::CompleteRun,
    }
}

#[test]
fn capability_policy_table() {
    let connection = connection();

    let ceiling = agent(vec![ToolId::List], true);
    let over = agent_step(vec![ToolId::List, ToolId::Write], true);
    assert_eq!(
        AttemptCapabilities::derive(&over, &ceiling, &connection),
        Err(CapabilityError::Authority)
    );

    let agent_caps = AttemptCapabilities::derive(
        &agent_step(vec![ToolId::List], true),
        &agent(vec![ToolId::List, ToolId::Read], true),
        &connection,
    )
    .expect("agent");
    assert_eq!(agent_caps.git_admin, AccessMode::ReadOnly);
    assert!(
        !agent_caps
            .tools
            .iter()
            .any(|tool| tool.as_str() == SUBMIT_WORKFLOW_OUTPUT)
    );
    assert_eq!(agent_caps.network, NetworkCapability::ProviderHost);
    assert_eq!(agent_caps.secret, SecretPresence::ProviderPlaceholder);
    assert_eq!(
        agent_caps.primary().map(|directory| directory.role),
        Some(DirectoryRole::PrimarySource)
    );

    let command_caps = AttemptCapabilities::derive(
        &status_step(),
        &agent(ToolId::ALL.to_vec(), true),
        &connection,
    )
    .expect("command");
    assert_eq!(command_caps.git_admin, AccessMode::ReadOnly);
    assert_eq!(command_caps.network, NetworkCapability::None);
    assert_eq!(command_caps.secret, SecretPresence::None);
    assert!(command_caps.tools.is_empty());
}
