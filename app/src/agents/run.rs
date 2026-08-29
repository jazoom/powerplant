use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::id::AgentId;

pub(crate) struct AgentLeaseCoordinator {
    leases: Mutex<HashSet<AgentId>>,
}

pub(crate) struct LeaseGuard {
    coordinator: Arc<AgentLeaseCoordinator>,
    agent_id: AgentId,
}

impl AgentLeaseCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            leases: Mutex::new(HashSet::new()),
        }
    }

    pub(crate) fn acquire(self: &Arc<Self>, agent_id: AgentId) -> Result<LeaseGuard, ()> {
        let mut leases = lock(&self.leases);
        if !leases.insert(agent_id) {
            return Err(());
        }
        Ok(LeaseGuard {
            coordinator: Arc::clone(self),
            agent_id,
        })
    }

    fn release(&self, agent_id: AgentId) {
        lock(&self.leases).remove(&agent_id);
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        self.coordinator.release(self.agent_id);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
