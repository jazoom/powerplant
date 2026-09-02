use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::{
    agents::AgentId,
    projects::{MAXIMUM_PROJECTS, ProjectId},
    providers::{ChatTurn, Role},
    sessions::{
        SESSION_LIFETIME,
        job::{Job, JobId, JobSnapshot},
        tokens::SessionId,
    },
    workflows::{RunId, WorkflowId},
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
}

struct Conversation {
    turns: Vec<ChatTurn>,
    job: Option<Arc<Job>>,
    preferred_workflow: Option<WorkflowId>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ConversationKey {
    pub(crate) project_id: ProjectId,
    pub(crate) agent_id: AgentId,
}

struct StoredSession {
    conversations: HashMap<ConversationKey, Conversation>,
    // One in-flight command per session. finish_turn and fail_turn clear it.
    active: Option<(ConversationKey, JobId)>,
    last_agents: HashMap<ProjectId, AgentId>,
    recent_projects: Vec<ProjectId>,
    expires_at: Instant,
}

#[derive(Clone)]
pub(crate) struct SessionSnapshot {
    pub(crate) turns: Vec<ChatTurn>,
    pub(crate) job: Option<JobSnapshot>,
    pub(crate) session_busy: bool,
    pub(crate) preferred_workflow: Option<WorkflowId>,
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
                conversations: HashMap::new(),
                active: None,
                last_agents: HashMap::new(),
                recent_projects: Vec::new(),
                expires_at,
            },
        );
    }

    pub(crate) fn contains_live(&self, id: &SessionId) -> bool {
        let mut sessions = self.lock();
        live(&mut sessions, id, self.clock.now()).is_some()
    }

    pub(crate) fn contains_expired(&self, id: &SessionId) -> bool {
        self.lock()
            .get(id)
            .is_some_and(|session| session.expires_at <= self.clock.now())
    }

    pub(crate) fn busy(&self, id: &SessionId) -> bool {
        let mut sessions = self.lock();
        live(&mut sessions, id, self.clock.now()).is_some_and(|session| session.active.is_some())
    }

    pub(crate) fn snapshot(
        &self,
        id: &SessionId,
        key: &ConversationKey,
    ) -> Option<SessionSnapshot> {
        let mut sessions = self.lock();
        live(&mut sessions, id, self.clock.now()).map(|session| snapshot_session(key, session))
    }

    pub(crate) fn set_preferred_workflow(
        &self,
        id: &SessionId,
        key: ConversationKey,
        workflow: WorkflowId,
    ) {
        let mut sessions = self.lock();
        let Some(session) = live_mut(&mut sessions, id, self.clock.now()) else {
            return;
        };
        let conversation = session
            .conversations
            .entry(key)
            .or_insert_with(|| Conversation {
                turns: Vec::new(),
                job: None,
                preferred_workflow: None,
            });
        conversation.preferred_workflow = Some(workflow);
    }

    pub(crate) fn remember_conversation(&self, id: &SessionId, key: ConversationKey) {
        let mut sessions = self.lock();
        let Some(session) = live_mut(&mut sessions, id, self.clock.now()) else {
            return;
        };
        session.last_agents.insert(key.project_id, key.agent_id);
        session
            .recent_projects
            .retain(|item| *item != key.project_id);
        session.recent_projects.insert(0, key.project_id);
        session.recent_projects.truncate(MAXIMUM_PROJECTS);
        session
            .last_agents
            .retain(|project, _| session.recent_projects.contains(project));
    }

    pub(crate) fn last_agent(&self, id: &SessionId, project: &ProjectId) -> Option<AgentId> {
        let mut sessions = self.lock();
        live(&mut sessions, id, self.clock.now())
            .and_then(|session| session.last_agents.get(project).copied())
    }

    pub(crate) fn forget_last_agent(&self, id: &SessionId, project: &ProjectId) {
        let mut sessions = self.lock();
        let Some(session) = live_mut(&mut sessions, id, self.clock.now()) else {
            return;
        };
        session.last_agents.remove(project);
    }

    pub(crate) fn recent_projects(&self, id: &SessionId) -> Vec<ProjectId> {
        let mut sessions = self.lock();
        live(&mut sessions, id, self.clock.now())
            .map(|session| session.recent_projects.clone())
            .unwrap_or_default()
    }

    pub(crate) fn begin_turn(
        &self,
        id: &SessionId,
        key: ConversationKey,
        run_id: RunId,
        message: String,
    ) -> Result<BegunTurn, BeginTurnError> {
        let mut sessions = self.lock();
        let session =
            live_mut(&mut sessions, id, self.clock.now()).ok_or(BeginTurnError::MissingSession)?;
        if session.active.is_some() {
            return Err(BeginTurnError::Conflict);
        }
        let job_id = JobId::generate().map_err(|_| BeginTurnError::JobId)?;
        let conversation = session
            .conversations
            .entry(key)
            .or_insert_with(|| Conversation {
                turns: Vec::new(),
                job: None,
                preferred_workflow: None,
            });
        conversation.turns.push(ChatTurn {
            role: Role::User,
            text: message,
        });
        let job = Job::new(job_id, run_id, conversation.turns.len());
        conversation.job = Some(job.clone());
        session.active = Some((key, job_id));
        Ok(BegunTurn {
            job,
            turns: conversation.turns.clone(),
        })
    }

    pub(crate) fn finish_turn(
        &self,
        id: &SessionId,
        key: &ConversationKey,
        job_id: &JobId,
        reply: String,
    ) -> bool {
        self.complete_turn(id, key, job_id, reply)
    }

    pub(crate) fn fail_turn(
        &self,
        id: &SessionId,
        key: &ConversationKey,
        job_id: &JobId,
        partial: String,
    ) -> bool {
        self.complete_turn(id, key, job_id, partial)
    }

    pub(crate) fn rollback_turn(
        &self,
        id: &SessionId,
        key: &ConversationKey,
        job_id: &JobId,
    ) -> bool {
        let mut sessions = self.lock();
        let Some(session) = live_mut(&mut sessions, id, self.clock.now()) else {
            return false;
        };
        if session.active != Some((*key, *job_id)) {
            return false;
        }
        if let Some(conversation) = session.conversations.get_mut(key) {
            conversation.turns.pop();
            if conversation
                .job
                .as_ref()
                .is_some_and(|job| job.id() == *job_id)
            {
                conversation.job = None;
            }
        }
        session.active = None;
        true
    }

    pub(crate) fn job(
        &self,
        id: &SessionId,
        key: &ConversationKey,
        job_id: &JobId,
    ) -> Option<Arc<Job>> {
        let mut sessions = self.lock();
        live(&mut sessions, id, self.clock.now()).and_then(|session| {
            session
                .conversations
                .get(key)
                .and_then(|conversation| conversation.job.as_ref())
                .filter(|job| job.id() == *job_id)
                .cloned()
        })
    }

    pub(crate) fn remove(&self, id: &SessionId) {
        let mut sessions = self.lock();
        cancel_and_remove(&mut sessions, id);
    }

    pub(crate) fn expired_ids(&self) -> Vec<SessionId> {
        let now = self.clock.now();
        self.lock()
            .iter()
            .filter(|(_, session)| session.expires_at <= now)
            .map(|(id, _)| *id)
            .collect()
    }

    // Only the active job may complete the turn. A stale writer cannot overwrite a later command.
    fn complete_turn(
        &self,
        id: &SessionId,
        key: &ConversationKey,
        job_id: &JobId,
        reply: String,
    ) -> bool {
        let mut sessions = self.lock();
        let Some(session) = live_mut(&mut sessions, id, self.clock.now()) else {
            return false;
        };
        if session.active != Some((*key, *job_id)) {
            return false;
        }
        if let Some(conversation) = session.conversations.get_mut(key)
            && !reply.trim().is_empty()
        {
            conversation.turns.push(ChatTurn {
                role: Role::Assistant,
                text: reply,
            });
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

fn snapshot_session(key: &ConversationKey, session: &StoredSession) -> SessionSnapshot {
    let conversation = session.conversations.get(key);
    SessionSnapshot {
        turns: conversation
            .map(|conversation| conversation.turns.clone())
            .unwrap_or_default(),
        job: conversation
            .and_then(|conversation| conversation.job.as_ref().map(|job| job.snapshot())),
        session_busy: session.active.is_some(),
        preferred_workflow: conversation.and_then(|conversation| conversation.preferred_workflow),
    }
}

fn live<'a>(
    sessions: &'a mut HashMap<SessionId, StoredSession>,
    id: &SessionId,
    now: Instant,
) -> Option<&'a StoredSession> {
    sessions.get(id).filter(|session| session.expires_at > now)
}

fn live_mut<'a>(
    sessions: &'a mut HashMap<SessionId, StoredSession>,
    id: &SessionId,
    now: Instant,
) -> Option<&'a mut StoredSession> {
    if sessions
        .get(id)
        .is_some_and(|session| session.expires_at <= now)
    {
        return None;
    }
    sessions.get_mut(id)
}

fn cancel_and_remove(sessions: &mut HashMap<SessionId, StoredSession>, id: &SessionId) {
    if let Some(session) = sessions.get(id) {
        cancel_jobs(session);
    }
    sessions.remove(id);
}

fn cancel_jobs(session: &StoredSession) {
    for conversation in session.conversations.values() {
        if let Some(job) = &conversation.job {
            job.request_cancel();
        }
    }
}
