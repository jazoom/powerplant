use std::path::PathBuf;

use super::super::definition::{AgentAuthority, GuestDirectoryAccess, SystemCommandId};
use super::{guest_command, intersect_authority, settle_transient_job};
use crate::agents::{AccessMode, AgentId, DirectoryPolicy, PolicyGrant};
use crate::sandbox::GUEST_PROJECT;
use crate::sessions::JobStatus;

#[test]
fn repository_status_uses_the_fixed_guest_command() {
    let exec = guest_command(SystemCommandId::RepositoryStatus);
    assert_eq!(exec.program, "git");
    assert_eq!(
        exec.args,
        ["status".to_owned(), "--porcelain=v1".to_owned()]
    );
    assert_eq!(exec.cwd, GUEST_PROJECT);
    assert!(exec.stdin.is_none());
}

#[test]
fn a_secondary_authority_does_not_expose_the_project_mount() {
    let host = DirectoryPolicy::from_grants(
        vec![
            PolicyGrant {
                alias: "project".to_owned(),
                guest_path: GUEST_PROJECT.to_owned(),
                host_path: PathBuf::from("/host/project"),
                access: AccessMode::ReadWrite,
            },
            PolicyGrant {
                alias: "docs".to_owned(),
                guest_path: "/access/docs".to_owned(),
                host_path: PathBuf::from("/host/docs"),
                access: AccessMode::ReadOnly,
            },
        ],
        "project".to_owned(),
    );
    let authority = AgentAuthority::new(
        Vec::new(),
        vec![GuestDirectoryAccess {
            alias: "docs".to_owned(),
            access: AccessMode::ReadOnly,
        }],
    )
    .expect("authority");
    let policy = intersect_authority(&authority, &host).expect("intersection");
    assert_eq!(policy.primary_guest(), "/access/docs");
    assert_eq!(
        policy.resolve(""),
        Ok(("/access/docs".to_owned(), AccessMode::ReadOnly))
    );
    assert!(policy.resolve(GUEST_PROJECT).is_err());
}

#[test]
fn terminal_settlement_releases_the_active_session_turn() {
    let state = crate::state::for_test(crate::config::RuntimeConfig::development_for_test());
    let token = crate::sessions::generate_session_token().expect("token");
    let session_id = token.id();
    let agent_id = AgentId::generate().expect("agent");
    state.sessions.insert(session_id);
    let begun = state
        .sessions
        .begin_turn(
            &session_id,
            agent_id,
            crate::workflows::RunId::generate().expect("run"),
            "Hello".to_owned(),
        )
        .expect("turn");
    settle_transient_job(
        &state,
        &session_id,
        &agent_id,
        &begun.job,
        JobStatus::Failed,
        Some("Operational error"),
    );
    let snapshot = state
        .sessions
        .snapshot(&session_id, &agent_id)
        .expect("session");
    assert!(!snapshot.session_busy);
    assert_eq!(begun.job.snapshot().status, JobStatus::Failed);
}

#[test]
fn unregistered_command_text_cannot_become_a_system_command() {
    assert!(SystemCommandId::parse("rm -rf /").is_none());
    assert!(SystemCommandId::parse("git status --porcelain=v1").is_none());
    assert_eq!(
        SystemCommandId::parse("repository-status"),
        Some(SystemCommandId::RepositoryStatus)
    );
}
