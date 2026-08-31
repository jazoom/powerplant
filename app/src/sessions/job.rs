//! Server-owned chat jobs. Events keep a monotonic sequence for observation.

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::rand_core::TryRng;
use rand::rngs::SysRng;
use tokio::sync::Notify;

use crate::hex;
use crate::workflows::RunId;

#[cfg(test)]
pub(crate) const JOB_ID_LENGTH: usize = 32;

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub(crate) struct JobId([u8; 16]);

impl JobId {
    pub(crate) fn generate() -> Result<Self, JobIdError> {
        let mut bytes = [0u8; 16];
        SysRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| JobIdError::RandomUnavailable)?;
        Ok(Self(bytes))
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        hex::decode(value).map(Self)
    }

    pub(crate) fn as_hex(&self) -> String {
        hex::encode(&self.0)
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.as_hex())
    }
}

impl std::fmt::Debug for JobId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JobId(")?;
        formatter.write_str(&self.as_hex())?;
        formatter.write_str(")")
    }
}

#[derive(Debug)]
pub(crate) enum JobIdError {
    RandomUnavailable,
}

impl std::fmt::Display for JobIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("system random source unavailable")
    }
}

impl std::error::Error for JobIdError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JobStatus {
    Running,
    AwaitingDecision,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum JobEventKind {
    Output { delta: String },
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JobEvent {
    pub(crate) seq: u64,
    pub(crate) kind: JobEventKind,
}

#[derive(Clone, Debug)]
pub(crate) struct JobSnapshot {
    pub(crate) id: JobId,
    pub(crate) run_id: RunId,
    pub(crate) status: JobStatus,
    pub(crate) output: String,
    pub(crate) latest_seq: u64,
    pub(crate) assistant_index: usize,
    pub(crate) error: Option<String>,
    pub(crate) cancel_requested: bool,
    pub(crate) step_label: String,
    pub(crate) workflow_name: String,
}

pub(crate) struct Job {
    id: JobId,
    run_id: RunId,
    assistant_index: usize,
    inner: Mutex<JobInner>,
    notify: Notify,
    cancel: AtomicBool,
    step_label: Mutex<String>,
    workflow_name: Mutex<String>,
}

struct JobInner {
    status: JobStatus,
    events: Vec<JobEvent>,
    output: String,
    latest_seq: u64,
    error: Option<String>,
}

impl Job {
    pub(crate) fn new(id: JobId, run_id: RunId, assistant_index: usize) -> Arc<Self> {
        Arc::new(Self {
            id,
            run_id,
            assistant_index,
            inner: Mutex::new(JobInner {
                status: JobStatus::Running,
                events: Vec::new(),
                output: String::new(),
                latest_seq: 0,
                error: None,
            }),
            notify: Notify::new(),
            cancel: AtomicBool::new(false),
            step_label: Mutex::new(String::new()),
            workflow_name: Mutex::new(String::new()),
        })
    }

    pub(crate) fn id(&self) -> JobId {
        self.id
    }

    pub(crate) fn run_id(&self) -> RunId {
        self.run_id
    }

    pub(crate) fn set_step_label(&self, label: String) {
        *self
            .step_label
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = label;
        self.notify.notify_waiters();
    }

    pub(crate) fn step_label(&self) -> String {
        self.step_label
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn set_workflow_name(&self, name: String) {
        *self
            .workflow_name
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = name;
        self.notify.notify_waiters();
    }

    pub(crate) fn workflow_name(&self) -> String {
        self.workflow_name
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn assistant_index(&self) -> usize {
        self.assistant_index
    }

    pub(crate) fn is_running(&self) -> bool {
        self.lock().status == JobStatus::Running
    }

    pub(crate) fn set_awaiting_decision(&self) -> Option<u64> {
        let mut inner = self.lock();
        if inner.status != JobStatus::Running {
            return None;
        }
        inner.status = JobStatus::AwaitingDecision;
        inner.latest_seq += 1;
        let seq = inner.latest_seq;
        inner.events.push(JobEvent {
            seq,
            kind: JobEventKind::Completed,
        });
        drop(inner);
        self.notify.notify_waiters();
        Some(seq)
    }

    pub(crate) fn resume(&self) -> bool {
        let mut inner = self.lock();
        if inner.status != JobStatus::AwaitingDecision {
            return false;
        }
        inner.status = JobStatus::Running;
        inner.error = None;
        drop(inner);
        self.notify.notify_waiters();
        true
    }

    pub(crate) fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub(crate) fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub(crate) async fn cancelled(&self) {
        loop {
            if self.cancel_requested() {
                return;
            }
            let notified = self.notify.notified();
            if self.cancel_requested() {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn snapshot(&self) -> JobSnapshot {
        let inner = self.lock();
        JobSnapshot {
            id: self.id,
            run_id: self.run_id,
            status: inner.status,
            output: inner.output.clone(),
            latest_seq: inner.latest_seq,
            assistant_index: self.assistant_index,
            error: inner.error.clone(),
            cancel_requested: self.cancel.load(Ordering::SeqCst),
            step_label: self.step_label(),
            workflow_name: self.workflow_name(),
        }
    }

    pub(crate) fn latest_seq(&self) -> u64 {
        self.lock().latest_seq
    }

    pub(crate) fn events_after(&self, cursor: u64) -> Vec<JobEvent> {
        self.lock()
            .events
            .iter()
            .filter(|event| event.seq > cursor)
            .cloned()
            .collect()
    }

    pub(crate) fn output_up_to(&self, cursor: u64) -> String {
        let inner = self.lock();
        let mut text = String::new();
        for event in &inner.events {
            if event.seq > cursor {
                break;
            }
            if let JobEventKind::Output { delta } = &event.kind {
                text.push_str(delta);
            }
        }
        text
    }

    pub(crate) fn has_output_at_or_before(&self, cursor: u64) -> bool {
        self.lock()
            .events
            .iter()
            .any(|event| event.seq <= cursor && matches!(event.kind, JobEventKind::Output { .. }))
    }

    pub(crate) fn push_output(&self, delta: String) -> Option<u64> {
        if delta.is_empty() {
            return None;
        }
        let mut inner = self.lock();
        if inner.status != JobStatus::Running {
            return None;
        }
        inner.latest_seq += 1;
        let seq = inner.latest_seq;
        inner.output.push_str(&delta);
        inner.events.push(JobEvent {
            seq,
            kind: JobEventKind::Output { delta },
        });
        drop(inner);
        self.notify.notify_waiters();
        Some(seq)
    }

    pub(crate) fn finish(&self, status: JobStatus, error: Option<&str>) -> Option<u64> {
        if !matches!(
            status,
            JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
        ) {
            return None;
        }
        let mut inner = self.lock();
        if !matches!(
            inner.status,
            JobStatus::Running | JobStatus::AwaitingDecision
        ) {
            return None;
        }
        inner.status = status;
        inner.error = error.map(str::to_owned);
        inner.latest_seq += 1;
        let seq = inner.latest_seq;
        inner.events.push(JobEvent {
            seq,
            kind: match status {
                JobStatus::Completed => JobEventKind::Completed,
                JobStatus::Failed => JobEventKind::Failed,
                JobStatus::Cancelled => JobEventKind::Cancelled,
                JobStatus::Running | JobStatus::AwaitingDecision => {
                    unreachable!("terminal status checked above")
                }
            },
        });
        drop(inner);
        self.notify.notify_waiters();
        Some(seq)
    }

    pub(crate) async fn wait_after(&self, cursor: u64, timeout: Duration) {
        if timeout.is_zero() {
            return;
        }
        let deadline = Instant::now() + timeout;
        loop {
            if self.latest_seq() > cursor || !self.is_running() {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            let notified = self.notify.notified();
            if self.latest_seq() > cursor || !self.is_running() {
                return;
            }
            if tokio::time::timeout(remaining, notified).await.is_err() {
                return;
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, JobInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
