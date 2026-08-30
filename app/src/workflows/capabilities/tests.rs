use super::{
    AttemptCapabilities, CapabilityError, DirectoryRole, NetworkCapability, PrimarySourceLocation,
    SecretPresence,
};
use crate::agents::{AccessMode, AgentId, AgentRecord, DirectoryGrant, ToolId};
use crate::providers::{ProviderConnection, ProviderKind};
use crate::tools::SUBMIT_WORKFLOW_OUTPUT;
use crate::workflows::definition::{
    AgentAuthority, AgentStep, GuestDirectoryAccess, OutputKey, OutputKind, RequiredOutput,
    RoleKey, StepAction, StepDefinition, StepEnvironment, StepKey, SuccessTransition,
    SystemCommandId, SystemCommandStep,
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

fn commit_step() -> StepDefinition {
    StepDefinition {
        key: StepKey::parse("commit").expect("step"),
        name: "Commit".to_owned(),
        inputs: Vec::new(),
        action: StepAction::SystemCommand(SystemCommandStep {
            command: SystemCommandId::CommitCandidate,
            environment: StepEnvironment::WorkflowDefault,
            required_outputs: vec![RequiredOutput {
                key: OutputKey::parse("committed-candidate").expect("output"),
                kind: OutputKind::CandidateRevision,
            }],
        }),
        on_success: SuccessTransition::CompleteRun,
    }
}

#[test]
fn capability_policy_table() {
    let connection = connection();
    let ceiling = agent(ToolId::ALL.to_vec(), true);

    let over = agent_step(vec![ToolId::List, ToolId::Write], true);
    assert_eq!(
        AttemptCapabilities::derive(&over, &agent(vec![ToolId::List], true), &connection),
        Err(CapabilityError::Authority)
    );

    let planner = AttemptCapabilities::derive(
        &agent_step(vec![ToolId::List, ToolId::Read, ToolId::Run], false),
        &ceiling,
        &connection,
    )
    .expect("planner");
    assert_eq!(
        planner.primary().map(|directory| directory.access),
        Some(AccessMode::ReadOnly)
    );
    assert_eq!(planner.git_admin, AccessMode::ReadOnly);
    assert_eq!(
        planner.source_location,
        PrimarySourceLocation::AttemptWorkspace
    );
    assert_eq!(planner.network, NetworkCapability::ProviderHost);

    let implementer = AttemptCapabilities::derive(
        &agent_step(ToolId::ALL.to_vec(), true),
        &ceiling,
        &connection,
    )
    .expect("implementer");
    assert_eq!(
        implementer.primary().map(|directory| directory.access),
        Some(AccessMode::ReadWrite)
    );
    assert_eq!(
        implementer.source_location,
        PrimarySourceLocation::AttemptWorkspace
    );
    assert_eq!(implementer.git_admin, AccessMode::ReadOnly);
    assert!(
        !implementer
            .tools
            .iter()
            .any(|tool| tool.as_str() == SUBMIT_WORKFLOW_OUTPUT)
    );

    let reviewer = AttemptCapabilities::derive(
        &agent_step(vec![ToolId::List, ToolId::Read, ToolId::Run], false),
        &ceiling,
        &connection,
    )
    .expect("reviewer");
    assert_eq!(
        reviewer.primary().map(|directory| directory.access),
        Some(AccessMode::ReadOnly)
    );
    assert_eq!(reviewer.git_admin, AccessMode::ReadOnly);

    let status =
        AttemptCapabilities::derive(&status_step(), &ceiling, &connection).expect("status");
    assert!(status.tools.is_empty());
    assert_eq!(status.network, NetworkCapability::None);
    assert_eq!(status.secret, SecretPresence::None);
    assert_eq!(status.git_admin, AccessMode::ReadOnly);
    assert_eq!(
        status.source_location,
        PrimarySourceLocation::AttemptWorkspace
    );
    assert_eq!(
        status.primary().map(|directory| directory.access),
        Some(AccessMode::ReadOnly)
    );

    let commit =
        AttemptCapabilities::derive(&commit_step(), &ceiling, &connection).expect("commit");
    assert!(commit.tools.is_empty());
    assert_eq!(commit.network, NetworkCapability::None);
    assert_eq!(commit.secret, SecretPresence::None);
    assert_eq!(commit.git_admin, AccessMode::ReadWrite);
    assert_eq!(commit.source_location, PrimarySourceLocation::UserProject);
    assert_eq!(
        commit.primary().map(|directory| directory.access),
        Some(AccessMode::ReadWrite)
    );
    assert_eq!(
        commit.primary().map(|directory| directory.role),
        Some(DirectoryRole::PrimarySource)
    );
}
