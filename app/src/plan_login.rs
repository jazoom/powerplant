use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::providers::ProviderKind;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingPlan {
    pub(crate) kind: ProviderKind,
    pub(crate) verification_uri: String,
    pub(crate) user_code: String,
    pub(crate) error: Option<String>,
}

struct Inner {
    generation: u64,
    task: Option<JoinHandle<()>>,
}

pub(crate) struct PlanLogin {
    pending: watch::Sender<Option<PendingPlan>>,
    inner: Mutex<Inner>,
}

impl PlanLogin {
    pub(crate) fn new() -> Self {
        let (pending, _) = watch::channel(None);
        Self {
            pending,
            inner: Mutex::new(Inner {
                generation: 0,
                task: None,
            }),
        }
    }

    pub(crate) fn snapshot(&self) -> Option<PendingPlan> {
        self.pending.borrow().clone()
    }

    pub(crate) async fn wait_until_changed(
        &self,
        previous: Option<PendingPlan>,
        timeout: Duration,
    ) -> Option<PendingPlan> {
        let mut rx = self.pending.subscribe();
        if timeout.is_zero() {
            return rx.borrow().clone();
        }
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            {
                let current = rx.borrow().clone();
                if current != previous {
                    return current;
                }
            }
            tokio::select! {
                _ = &mut deadline => return rx.borrow().clone(),
                result = rx.changed() => {
                    if result.is_err() {
                        return rx.borrow().clone();
                    }
                }
            }
        }
    }

    pub(crate) fn begin(&self) -> u64 {
        let mut inner = self.lock();
        if let Some(task) = inner.task.take() {
            task.abort();
        }
        self.pending.send_replace(None);
        inner.generation = inner.generation.wrapping_add(1);
        inner.generation
    }

    pub(crate) fn set_pending(&self, generation: u64, pending: PendingPlan) {
        let inner = self.lock();
        if inner.generation == generation {
            self.pending.send_replace(Some(pending));
        }
    }

    pub(crate) fn set_error(&self, generation: u64, message: String) {
        let inner = self.lock();
        if inner.generation != generation {
            return;
        }
        let mut pending = self.pending.borrow().clone();
        if let Some(pending) = pending.as_mut() {
            pending.error = Some(message);
            self.pending.send_replace(Some(pending.clone()));
            return;
        }
        self.pending.send_replace(None);
    }

    pub(crate) fn finish(&self, generation: u64) {
        let mut inner = self.lock();
        if inner.generation == generation {
            self.pending.send_replace(None);
            inner.task = None;
        }
    }

    pub(crate) fn attach_task(&self, generation: u64, task: JoinHandle<()>) {
        let mut inner = self.lock();
        if inner.generation == generation {
            if let Some(previous) = inner.task.replace(task) {
                previous.abort();
            }
        } else {
            task.abort();
        }
    }

    #[cfg(test)]
    pub(crate) fn set_pending_for_test(&self, pending: PendingPlan) {
        self.pending.send_replace(Some(pending));
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
