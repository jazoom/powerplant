use std::sync::{Arc, Mutex};

pub(crate) struct WorkflowExecution {
    held: Mutex<bool>,
}

pub(crate) struct ExecutionGuard {
    execution: Arc<WorkflowExecution>,
}

impl WorkflowExecution {
    pub(crate) fn new() -> Self {
        Self {
            held: Mutex::new(false),
        }
    }

    pub(crate) fn acquire(self: &Arc<Self>) -> Result<ExecutionGuard, ()> {
        let mut held = lock(&self.held);
        if *held {
            return Err(());
        }
        *held = true;
        Ok(ExecutionGuard {
            execution: Arc::clone(self),
        })
    }

    fn release(&self) {
        *lock(&self.held) = false;
    }
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        self.execution.release();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
