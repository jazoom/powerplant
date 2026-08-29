use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{
    GuestAccess, GuestSandbox, MountSpec, SANDBOX_OWNER_LABEL, SANDBOX_OWNER_VALUE, SandboxError,
    SandboxSpec, owns_sandbox_owner,
};

fn spec(dir: &Path) -> SandboxSpec {
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
fn sandbox_ownership_requires_the_owner_label() {
    let mut labels = BTreeMap::new();
    assert!(!owns_sandbox_owner(&labels));
    labels.insert(SANDBOX_OWNER_LABEL.to_owned(), "another-owner".to_owned());
    assert!(!owns_sandbox_owner(&labels));
    labels.insert(
        SANDBOX_OWNER_LABEL.to_owned(),
        SANDBOX_OWNER_VALUE.to_owned(),
    );
    assert!(owns_sandbox_owner(&labels));
}

#[tokio::test]
async fn start_from_snapshot_is_rejected_without_mounts() {
    let sandbox = GuestSandbox::for_test();
    let spec = SandboxSpec {
        mounts: Vec::new(),
        workdir: "/project".to_owned(),
        access: GuestAccess::default(),
    };
    assert!(matches!(
        sandbox
            .start_from_snapshot(Path::new("snapshot"), "sha256:deadbeef", spec)
            .await,
        Err(SandboxError::NeedProject)
    ));
}

#[tokio::test]
async fn a_failed_command_records_the_guest_program() {
    let sandbox = GuestSandbox::for_test();
    let dir = tempfile::tempdir().expect("project");
    sandbox
        .start_from_snapshot(Path::new("snapshot"), "sha256:deadbeef", spec(dir.path()))
        .await
        .expect("start");
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
