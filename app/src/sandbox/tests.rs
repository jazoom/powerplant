use std::collections::BTreeMap;
use std::path::PathBuf;

use super::{
    GuestAccess, GuestSandbox, MountSpec, SANDBOX_AGENT_LABEL, SANDBOX_OWNER_LABEL,
    SANDBOX_OWNER_VALUE, SandboxError, SandboxSpec, owns_agent, owns_sandbox_owner,
};
use crate::agents::AgentId;

fn spec(dir: &std::path::Path) -> SandboxSpec {
    SandboxSpec {
        mounts: vec![MountSpec {
            guest: "/project".to_owned(),
            host: dir.canonicalize().expect("canonical"),
            read_only: false,
        }],
        workdir: "/project".to_owned(),
        access: GuestAccess::default(),
    }
}

#[test]
fn sandbox_ownership_requires_owner_and_agent_labels() {
    let id = AgentId::generate().expect("id");
    let mut labels = BTreeMap::new();
    assert!(!owns_sandbox_owner(&labels));
    assert!(!owns_agent(&labels, id));

    labels.insert(SANDBOX_OWNER_LABEL.to_owned(), "another-owner".to_owned());
    assert!(!owns_sandbox_owner(&labels));

    labels.insert(
        SANDBOX_OWNER_LABEL.to_owned(),
        SANDBOX_OWNER_VALUE.to_owned(),
    );
    assert!(owns_sandbox_owner(&labels));
    assert!(!owns_agent(&labels, id));

    labels.insert(SANDBOX_AGENT_LABEL.to_owned(), id.as_hex());
    assert!(owns_agent(&labels, id));
    let other = AgentId::generate().expect("other");
    assert!(!owns_agent(&labels, other));
}

#[tokio::test]
async fn start_is_rejected_without_mounts() {
    let sandbox = GuestSandbox::for_test();
    let spec = SandboxSpec {
        mounts: Vec::new(),
        workdir: "/project".to_owned(),
        access: GuestAccess::default(),
    };
    assert!(matches!(
        sandbox.start_with(spec).await,
        Err(SandboxError::NeedProject)
    ));
}

#[tokio::test]
async fn directory_changes_are_rejected_while_the_sandbox_is_starting() {
    let sandbox = GuestSandbox::for_test();
    let dir = tempfile::tempdir().expect("project");
    sandbox.start_with(spec(dir.path())).await.expect("start");
    assert!(matches!(
        sandbox.reject_if_active().await,
        Err(SandboxError::Busy)
    ));
}

#[tokio::test]
async fn directory_changes_are_rejected_while_the_sandbox_is_running() {
    let sandbox = GuestSandbox::for_test();
    let dir = tempfile::tempdir().expect("project");
    sandbox.start_with(spec(dir.path())).await.expect("start");
    sandbox.complete_start();
    assert!(matches!(
        sandbox.reject_if_active().await,
        Err(SandboxError::ProjectLocked)
    ));
}

#[tokio::test]
async fn stop_is_rejected_while_a_command_is_running() {
    let sandbox = GuestSandbox::for_test();
    let dir = tempfile::tempdir().expect("project");
    sandbox.start_with(spec(dir.path())).await.expect("start");
    sandbox.complete_start();
    sandbox.hang_next_command();
    let session = sandbox
        .exec_cmd(super::GuestExec::shell("sleep"))
        .await
        .expect("exec");
    assert!(matches!(sandbox.stop().await, Err(SandboxError::Active)));
    session.kill().await;
    session.close().await;
    sandbox.stop().await.expect("stop after command");
}

#[tokio::test]
async fn a_failed_command_records_the_guest_program() {
    let sandbox = GuestSandbox::for_test();
    let dir = tempfile::tempdir().expect("project");
    sandbox.start_with(spec(dir.path())).await.expect("start");
    sandbox.complete_start();
    sandbox.fail_next_command();
    let mut session = sandbox
        .exec_cmd(super::GuestExec::command(
            "git",
            vec!["status".to_owned(), "--porcelain=v1".to_owned()],
        ))
        .await
        .expect("exec");
    assert_eq!(
        sandbox.last_exec().as_deref(),
        Some("git status --porcelain=v1")
    );
    let mut code = None;
    while let Some(event) = session.recv().await {
        if let super::CommandEvent::Exited(value) = event {
            code = Some(value);
        }
    }
    session.close().await;
    assert_eq!(code, Some(1));
}

#[test]
fn stale_mounts_are_rejected() {
    let sandbox_spec = SandboxSpec {
        mounts: vec![MountSpec {
            guest: "/project".to_owned(),
            host: PathBuf::from("/no/such/powerplant-mount"),
            read_only: false,
        }],
        workdir: "/project".to_owned(),
        access: GuestAccess::default(),
    };
    assert!(matches!(
        super::confirm_mounts(&sandbox_spec),
        Err(SandboxError::DirectoryMissing)
    ));
}
