use std::collections::HashMap;
use std::sync::Mutex;

use crate::{
    providers::{ChatTurn, ProviderConnection},
    sessions::tokens::SessionId,
};

pub(crate) struct SessionStore {
    sessions: Mutex<HashMap<SessionId, StoredSession>>,
}

struct StoredSession {
    connection: ProviderConnection,
    turns: Vec<ChatTurn>,
}

#[derive(Clone)]
pub(crate) struct SessionSnapshot {
    pub(crate) id: SessionId,
    pub(crate) connection: ProviderConnection,
    pub(crate) turns: Vec<ChatTurn>,
}

impl SessionStore {
    pub(crate) fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn insert(&self, id: SessionId, connection: ProviderConnection) {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                id,
                StoredSession {
                    connection,
                    turns: Vec::new(),
                },
            );
    }

    pub(crate) fn snapshot(&self, id: &SessionId) -> Option<SessionSnapshot> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id)
            .map(|session| SessionSnapshot {
                id: *id,
                connection: session.connection.clone(),
                turns: session.turns.clone(),
            })
    }

    pub(crate) fn replace_turns(&self, id: &SessionId, turns: Vec<ChatTurn>) -> bool {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(session) = sessions.get_mut(id) else {
            return false;
        };
        session.turns = turns;
        true
    }

    pub(crate) fn remove(&self, id: &SessionId) {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
    }
}
