use std::time::Duration;

use crate::agents::AgentId;
use crate::providers::{ChatTurn, Role};
use crate::sessions::{self, BeginTurnError, SESSION_LIFETIME};

fn agent() -> AgentId {
    AgentId::generate().expect("agent")
}

#[test]
fn parallel_commands_cannot_overwrite_a_completed_turn() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    let agent = agent();
    store.insert(id);

    let first = store
        .begin_turn(&id, agent, "First".to_owned())
        .expect("first");
    assert!(matches!(
        store.begin_turn(&id, agent, "Second".to_owned()),
        Err(BeginTurnError::Conflict)
    ));
    assert_eq!(
        store.snapshot(&id, &agent).expect("session").turns,
        [ChatTurn {
            role: Role::User,
            text: "First".to_owned(),
        }]
    );

    assert!(store.finish_turn(&id, &agent, &first.job.id(), "Done".to_owned()));

    let second = store
        .begin_turn(&id, agent, "Third".to_owned())
        .expect("third");
    assert!(!store.finish_turn(&id, &agent, &first.job.id(), "Late".to_owned()));

    let snapshot = store.snapshot(&id, &agent).expect("session");
    assert_eq!(
        snapshot.turns,
        [
            ChatTurn {
                role: Role::User,
                text: "First".to_owned(),
            },
            ChatTurn {
                role: Role::Assistant,
                text: "Done".to_owned(),
            },
            ChatTurn {
                role: Role::User,
                text: "Third".to_owned(),
            },
        ]
    );

    assert!(store.fail_turn(&id, &agent, &second.job.id(), "partial".to_owned()));
    let snapshot = store.snapshot(&id, &agent).expect("session");
    assert_eq!(
        snapshot.turns.last().map(|turn| turn.text.as_str()),
        Some("partial")
    );
}

#[test]
fn one_session_job_blocks_another_agent() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    let first = agent();
    let second = agent();
    store.insert(id);
    store
        .begin_turn(&id, first, "First".to_owned())
        .expect("first");
    assert!(matches!(
        store.begin_turn(&id, second, "Second".to_owned()),
        Err(BeginTurnError::Conflict)
    ));
    assert!(
        store
            .snapshot(&id, &second)
            .expect("session")
            .turns
            .is_empty()
    );
    assert!(store.snapshot(&id, &first).expect("session").session_busy);
}

#[test]
fn transcripts_are_independent_per_agent() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    let first = agent();
    let second = agent();
    store.insert(id);
    let begun = store
        .begin_turn(&id, first, "Hello".to_owned())
        .expect("begin");
    assert!(store.finish_turn(&id, &first, &begun.job.id(), "Hi".to_owned()));
    assert_eq!(
        store
            .snapshot(&id, &first)
            .expect("first")
            .turns
            .iter()
            .map(|turn| turn.text.as_str())
            .collect::<Vec<_>>(),
        ["Hello", "Hi"]
    );
    assert!(
        store
            .snapshot(&id, &second)
            .expect("second")
            .turns
            .is_empty()
    );
}

#[test]
fn expired_sessions_cannot_be_resolved() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    let agent = agent();
    store.insert(id);
    assert!(store.snapshot(&id, &agent).is_some());

    store.advance_clock(SESSION_LIFETIME + Duration::from_secs(1));
    assert!(store.snapshot(&id, &agent).is_none());
    assert!(!store.contains(&id));
}

#[test]
fn purge_removes_expired_sessions_without_a_lookup() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    store.insert(id);

    store.advance_clock(SESSION_LIFETIME + Duration::from_secs(1));
    assert!(store.contains(&id));
    store.purge_expired();
    assert!(!store.contains(&id));
    assert!(!store.contains_live(&id));
}

#[test]
fn live_sessions_survive_purge() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    let agent = agent();
    store.insert(id);

    store.advance_clock(SESSION_LIFETIME.saturating_sub(Duration::from_secs(1)));
    store.purge_expired();
    assert!(store.snapshot(&id, &agent).is_some());
}

#[test]
fn remove_cancels_the_active_job() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    let agent = agent();
    store.insert(id);
    let begun = store
        .begin_turn(&id, agent, "Hello".to_owned())
        .expect("begin");
    store.remove(&id);
    assert!(begun.job.cancel_requested());
    assert!(store.snapshot(&id, &agent).is_none());
}

#[test]
fn job_lookup_requires_the_agent_identity() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    let first = agent();
    let second = agent();
    store.insert(id);
    let begun = store
        .begin_turn(&id, first, "Hello".to_owned())
        .expect("begin");
    assert!(store.job(&id, &first, &begun.job.id()).is_some());
    assert!(store.job(&id, &second, &begun.job.id()).is_none());
}
