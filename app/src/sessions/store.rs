use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::{
    providers::{ChatTurn, Role},
    sessions::{
        SESSION_LIFETIME,
        job::{Job, JobId, JobSnapshot},
        tokens::SessionId,
    },
};

#[cfg(test)]
mod tests;

pub(crate) struct SessionStore {
    sessions: Mutex<HashMap<SessionId, StoredSession>>,
    clock: Clock,
}

struct Clock {
    offset_ms: AtomicU64,
}

impl Clock {
    fn real() -> Self {
        Self {
            offset_ms: AtomicU64::new(0),
        }
    }

    fn now(&self) -> Instant {
        Instant::now() + Duration::from_millis(self.offset_ms.load(Ordering::SeqCst))
    }

    #[cfg(test)]
    fn advance(&self, duration: Duration) {
        let millis = u64::try_from(duration.as_millis()).expect("duration fits u64");
        self.offset_ms.fetch_add(millis, Ordering::SeqCst);
    }
}

struct StoredSession {
    turns: Vec<ChatTurn>,
    job: Option<Arc<Job>>,
    // One in-flight command per session. finish_turn and fail_turn clear it.
    active: Option<JobId>,
    expires_at: Instant,
}

#[derive(Clone)]
pub(crate) struct SessionSnapshot {
    pub(crate) id: SessionId,
    pub(crate) turns: Vec<ChatTurn>,
    pub(crate) job: Option<JobSnapshot>,
}

pub(crate) struct BegunTurn {
    pub(crate) job: Arc<Job>,
    pub(crate) turns: Vec<ChatTurn>,
}

#[derive(Debug)]
pub(crate) enum BeginTurnError {
    MissingSession,
    Conflict,
    JobId,
}

impl SessionStore {
    pub(crate) fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            clock: Clock::real(),
        }
    }

    pub(crate) fn insert(&self, id: SessionId) {
        let expires_at = self.clock.now() + SESSION_LIFETIME;
        self.lock().insert(
            id,
            StoredSession {
                turns: Vec::new(),
                job: None,
                active: None,
                expires_at,
            },
        );
    }

    pub(crate) fn snapshot(&self, id: &SessionId) -> Option<SessionSnapshot> {
        let mut sessions = self.lock();
        live(&mut sessions, id, self.clock.now()).map(|session| snapshot_session(*id, session))
    }

    pub(crate) fn begin_turn(
        &self,
        id: &SessionId,
        message: String,
    ) -> Result<BegunTurn, BeginTurnError> {
        self.start_turn(id, message, false)
    }

    pub(crate) fn begin_command(
        &self,
        id: &SessionId,
        message: String,
    ) -> Result<BegunTurn, BeginTurnError> {
        self.start_turn(id, message, true)
    }

    fn start_turn(
        &self,
        id: &SessionId,
        message: String,
        command: bool,
    ) -> Result<BegunTurn, BeginTurnError> {
        let mut sessions = self.lock();
        let session =
            live_mut(&mut sessions, id, self.clock.now()).ok_or(BeginTurnError::MissingSession)?;
        if session.active.is_some() {
            return Err(BeginTurnError::Conflict);
        }
        let job_id = JobId::generate().map_err(|_| BeginTurnError::JobId)?;
        session.turns.push(ChatTurn {
            role: Role::User,
            text: message,
        });
        let job = if command {
            Job::command(job_id, session.turns.len())
        } else {
            Job::new(job_id, session.turns.len())
        };
        session.job = Some(job.clone());
        session.active = Some(job_id);
        Ok(BegunTurn {
            job,
            turns: session.turns.clone(),
        })
    }

    pub(crate) fn finish_turn(&self, id: &SessionId, job_id: &JobId, reply: String) -> bool {
        self.complete_turn(id, job_id, reply)
    }

    pub(crate) fn fail_turn(&self, id: &SessionId, job_id: &JobId, partial: String) -> bool {
        self.complete_turn(id, job_id, partial)
    }

    pub(crate) fn job(&self, id: &SessionId, job_id: &JobId) -> Option<Arc<Job>> {
        let mut sessions = self.lock();
        live(&mut sessions, id, self.clock.now()).and_then(|session| {
            session
                .job
                .as_ref()
                .filter(|job| job.id() == *job_id)
                .cloned()
        })
    }

    pub(crate) fn remove(&self, id: &SessionId) {
        let mut sessions = self.lock();
        cancel_and_remove(&mut sessions, id);
    }

    pub(crate) fn purge_expired(&self) {
        let now = self.clock.now();
        self.lock().retain(|_, session| {
            if session.expires_at <= now {
                cancel_job(session);
                false
            } else {
                true
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn advance_clock(&self, duration: Duration) {
        self.clock.advance(duration);
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, id: &SessionId) -> bool {
        self.lock().contains_key(id)
    }

    // Only the active job may complete the turn. A stale writer cannot overwrite a later command.
    fn complete_turn(&self, id: &SessionId, job_id: &JobId, reply: String) -> bool {
        let mut sessions = self.lock();
        let Some(session) = live_mut(&mut sessions, id, self.clock.now()) else {
            return false;
        };
        if session.active != Some(*job_id) {
            return false;
        }
        if !reply.trim().is_empty() {
            let role = if session.job.as_ref().is_some_and(|job| job.plain_output()) {
                Role::Command
            } else {
                Role::Assistant
            };
            session.turns.push(ChatTurn { role, text: reply });
        }
        session.active = None;
        true
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<SessionId, StoredSession>> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn snapshot_session(id: SessionId, session: &StoredSession) -> SessionSnapshot {
    SessionSnapshot {
        id,
        turns: session.turns.clone(),
        job: session.job.as_ref().map(|job| job.snapshot()),
    }
}

fn live<'a>(
    sessions: &'a mut HashMap<SessionId, StoredSession>,
    id: &SessionId,
    now: Instant,
) -> Option<&'a StoredSession> {
    evict_if_expired(sessions, id, now);
    sessions.get(id)
}

fn live_mut<'a>(
    sessions: &'a mut HashMap<SessionId, StoredSession>,
    id: &SessionId,
    now: Instant,
) -> Option<&'a mut StoredSession> {
    evict_if_expired(sessions, id, now);
    sessions.get_mut(id)
}

fn evict_if_expired(
    sessions: &mut HashMap<SessionId, StoredSession>,
    id: &SessionId,
    now: Instant,
) {
    if sessions
        .get(id)
        .is_some_and(|session| session.expires_at <= now)
    {
        cancel_and_remove(sessions, id);
    }
}

// Start cancellation before the session and API key leave the store.
fn cancel_and_remove(sessions: &mut HashMap<SessionId, StoredSession>, id: &SessionId) {
    if let Some(session) = sessions.get(id) {
        cancel_job(session);
    }
    sessions.remove(id);
}

fn cancel_job(session: &StoredSession) {
    if let Some(job) = &session.job {
        job.request_cancel();
    }
}
