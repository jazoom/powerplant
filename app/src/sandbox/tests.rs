use std::collections::BTreeMap;
use std::path::PathBuf;

use super::{
    GuestSandbox, SANDBOX_OWNER_LABEL, SANDBOX_OWNER_VALUE, SandboxError, owns_sandbox, project,
};

#[test]
fn sandbox_ownership_requires_the_power_plant_label() {
    let mut labels = BTreeMap::new();
    assert!(!owns_sandbox(&labels));

    labels.insert(SANDBOX_OWNER_LABEL.to_owned(), "another-owner".to_owned());
    assert!(!owns_sandbox(&labels));

    labels.insert(
        SANDBOX_OWNER_LABEL.to_owned(),
        SANDBOX_OWNER_VALUE.to_owned(),
    );
    assert!(owns_sandbox(&labels));
}

#[test]
fn project_file_round_trips_an_absolute_path() {
    let dir = tempfile::tempdir().expect("data dir");
    let file = dir.path().join("project.json");
    let project = PathBuf::from("/home/dev/app");
    project::persist(Some(&file), Some(&project)).expect("persist");
    assert_eq!(project::load(&file), Some(project));
    project::persist(Some(&file), None).expect("clear");
    assert!(project::load(&file).is_none());
}

#[tokio::test]
async fn set_project_is_rejected_while_the_sandbox_is_starting() {
    let sandbox = GuestSandbox::for_test();
    let dir = tempfile::tempdir().expect("project");
    sandbox
        .set_project(Some(dir.path().to_path_buf()))
        .await
        .expect("project");
    sandbox.start().await.expect("start");
    let other = tempfile::tempdir().expect("other");
    assert!(matches!(
        sandbox.set_project(Some(other.path().to_path_buf())).await,
        Err(SandboxError::Busy)
    ));
    assert_eq!(
        sandbox.project().as_deref(),
        Some(dir.path().canonicalize().expect("canonical").as_path())
    );
}

#[tokio::test]
async fn set_project_is_rejected_while_the_sandbox_is_running() {
    let sandbox = GuestSandbox::for_test();
    let dir = tempfile::tempdir().expect("project");
    sandbox
        .set_project(Some(dir.path().to_path_buf()))
        .await
        .expect("project");
    sandbox.start().await.expect("start");
    sandbox.complete_start();
    let other = tempfile::tempdir().expect("other");
    assert!(matches!(
        sandbox.set_project(Some(other.path().to_path_buf())).await,
        Err(SandboxError::ProjectLocked)
    ));
    assert_eq!(
        sandbox.project().as_deref(),
        Some(dir.path().canonicalize().expect("canonical").as_path())
    );
}

#[tokio::test]
async fn stop_is_rejected_while_a_command_is_running() {
    let sandbox = GuestSandbox::for_test();
    let dir = tempfile::tempdir().expect("project");
    sandbox
        .set_project(Some(dir.path().to_path_buf()))
        .await
        .expect("project");
    sandbox.start().await.expect("start");
    sandbox.complete_start();
    sandbox.hang_next_command();
    let session = sandbox.exec("sleep").await.expect("exec");
    assert!(matches!(sandbox.stop().await, Err(SandboxError::Active)));
    session.kill().await;
    session.close().await;
    sandbox.stop().await.expect("stop after command");
}
