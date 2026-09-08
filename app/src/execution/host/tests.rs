use std::sync::Arc;
use std::time::Duration;

use crate::sessions::{Job, JobId};

use super::{COMMAND_TIMEOUT, run_shell};

fn job() -> Arc<Job> {
    Job::for_conversation(
        JobId::generate().expect("job"),
        crate::conversations::ConversationId::generate().expect("conversation"),
        1,
    )
}

#[tokio::test]
async fn host_output_is_bounded_even_when_both_pipes_produce_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = run_shell(
        "head -c 262144 /dev/zero; head -c 262144 /dev/zero >&2",
        directory.path(),
        &job(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert!(output.len() <= crate::tools::MAXIMUM_TOOL_BYTES);
    assert!(output.contains("truncated"));
}

#[tokio::test]
async fn timeout_covers_descendant_pipes_after_the_shell_exits() {
    let directory = tempfile::tempdir().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        run_shell(
            "sleep 30 & exit 0",
            directory.path(),
            &job(),
            Duration::from_millis(50),
        ),
    )
    .await
    .expect("descendant pipes must not bypass the deadline");
    assert_eq!(result.unwrap_err(), "The command exceeded the time limit.");
}

#[tokio::test]
async fn host_timeout_stops_descendant_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    let error = run_shell(
        "(sleep 0.2; touch escaped) & wait",
        directory.path(),
        &job(),
        Duration::from_millis(50),
    )
    .await
    .unwrap_err();
    assert_eq!(error, "The command exceeded the time limit.");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!directory.path().join("escaped").exists());
}

#[tokio::test]
async fn cancelled_jobs_never_spawn_a_command() {
    let directory = tempfile::tempdir().unwrap();
    let job = job();
    job.request_cancel();
    let error = run_shell("touch dispatched", directory.path(), &job, COMMAND_TIMEOUT)
        .await
        .unwrap_err();
    assert_eq!(error, "Stopped.");
    assert!(!directory.path().join("dispatched").exists());
}

#[tokio::test]
async fn subprocess_environment_contains_only_allowed_variables() {
    let directory = tempfile::tempdir().unwrap();
    let output = run_shell("printenv", directory.path(), &job(), COMMAND_TIMEOUT)
        .await
        .unwrap();
    let allowed = [
        "PATH", "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "TERM", "TMPDIR", "PWD", "SHLVL", "_",
    ];
    for line in output.lines() {
        let (name, _) = line.split_once('=').unwrap();
        assert!(
            allowed.contains(&name),
            "unexpected inherited variable: {name}"
        );
    }
    assert!(output.contains("PATH="));
}
