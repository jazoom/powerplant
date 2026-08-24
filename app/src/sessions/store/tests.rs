use std::time::Duration;

use crate::providers::{ChatTurn, ProviderConnection, ProviderKind, Role, SecretString};
use crate::sessions::{self, BeginTurnError, SESSION_LIFETIME};

fn connection() -> ProviderConnection {
    ProviderConnection {
        kind: ProviderKind::Xai,
        api_key: SecretString::new("test-key".to_owned()),
        model: "grok-4.6".to_owned(),
    }
}

#[test]
fn parallel_commands_cannot_overwrite_a_completed_turn() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    store.insert(id, connection());

    let first = store.begin_turn(&id, "First".to_owned()).expect("first");
    assert!(matches!(
        store.begin_turn(&id, "Second".to_owned()),
        Err(BeginTurnError::Conflict)
    ));
    assert_eq!(
        store.snapshot(&id).expect("session").turns,
        [ChatTurn {
            role: Role::User,
            text: "First".to_owned(),
        }]
    );

    assert!(store.finish_turn(&id, &first.job.id(), "Done".to_owned()));

    let second = store.begin_turn(&id, "Third".to_owned()).expect("third");
    assert!(!store.finish_turn(&id, &first.job.id(), "Late".to_owned()));

    let snapshot = store.snapshot(&id).expect("session");
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

    assert!(store.fail_turn(&id, &second.job.id(), "partial".to_owned()));
    let snapshot = store.snapshot(&id).expect("session");
    assert_eq!(
        snapshot.turns.last().map(|turn| turn.text.as_str()),
        Some("partial")
    );
}

#[test]
fn expired_sessions_cannot_be_resolved() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    store.insert(id, connection());
    assert!(store.snapshot(&id).is_some());

    store.advance_clock(SESSION_LIFETIME + Duration::from_secs(1));
    assert!(store.snapshot(&id).is_none());
    assert!(!store.contains(&id));
}

#[test]
fn purge_removes_expired_sessions_without_a_lookup() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    store.insert(id, connection());

    store.advance_clock(SESSION_LIFETIME + Duration::from_secs(1));
    assert!(store.contains(&id));
    store.purge_expired();
    assert!(!store.contains(&id));
    assert!(store.snapshot(&id).is_none());
}

#[test]
fn live_sessions_survive_purge() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    store.insert(id, connection());

    store.advance_clock(SESSION_LIFETIME.saturating_sub(Duration::from_secs(1)));
    store.purge_expired();
    assert!(store.snapshot(&id).is_some());
}

#[test]
fn remove_cancels_the_active_job() {
    let store = super::SessionStore::new();
    let token = sessions::generate_session_token().expect("token");
    let id = token.id();
    store.insert(id, connection());
    let begun = store.begin_turn(&id, "Hello".to_owned()).expect("begin");
    store.remove(&id);
    assert!(begun.job.cancel_requested());
    assert!(store.snapshot(&id).is_none());
}
