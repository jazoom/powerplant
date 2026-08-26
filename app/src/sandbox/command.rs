#[cfg(test)]
use std::sync::{Arc, Mutex};

#[cfg(test)]
use tokio::sync::Notify;

use tokio::sync::OwnedMutexGuard;

use super::SandboxError;
#[cfg(test)]
use super::lock_mutex;

pub(crate) enum CommandEvent {
    Output(String),
    Exited(i32),
    Failed,
}

pub(crate) struct CommandSession {
    inner: CommandInner,
    // Drop after the command ends so stop/start cannot run mid-exec.
    _lifecycle: Option<OwnedMutexGuard<()>>,
}

enum CommandInner {
    Microsandbox(Box<MicrosandboxCommand>),
    #[cfg(test)]
    Scripted(ScriptedCommand),
}

struct MicrosandboxCommand {
    sandbox: microsandbox::Sandbox,
    exec: microsandbox::ExecHandle,
}

#[cfg(test)]
pub(super) struct ScriptedCommand {
    events: Mutex<Vec<CommandEvent>>,
    hang: bool,
    killed: Mutex<bool>,
    notify: Arc<Notify>,
}

impl CommandSession {
    pub(super) fn microsandbox(
        sandbox: microsandbox::Sandbox,
        exec: microsandbox::ExecHandle,
    ) -> Self {
        Self {
            inner: CommandInner::Microsandbox(Box::new(MicrosandboxCommand { sandbox, exec })),
            _lifecycle: None,
        }
    }

    #[cfg(test)]
    pub(super) fn scripted(command: ScriptedCommand) -> Self {
        Self {
            inner: CommandInner::Scripted(command),
            _lifecycle: None,
        }
    }

    pub(super) fn with_lifecycle(mut self, lifecycle: OwnedMutexGuard<()>) -> Self {
        self._lifecycle = Some(lifecycle);
        self
    }

    pub(crate) async fn recv(&mut self) -> Option<CommandEvent> {
        match &mut self.inner {
            CommandInner::Microsandbox(command) => loop {
                match command.exec.recv().await? {
                    microsandbox::ExecEvent::Stdout(data)
                    | microsandbox::ExecEvent::Stderr(data) => {
                        if data.is_empty() {
                            continue;
                        }
                        return Some(CommandEvent::Output(
                            String::from_utf8_lossy(&data).into_owned(),
                        ));
                    }
                    microsandbox::ExecEvent::Exited { code } => {
                        return Some(CommandEvent::Exited(code));
                    }
                    microsandbox::ExecEvent::Failed(_) => {
                        return Some(CommandEvent::Failed);
                    }
                    microsandbox::ExecEvent::Started { .. }
                    | microsandbox::ExecEvent::StdinError(_) => {}
                }
            },
            #[cfg(test)]
            CommandInner::Scripted(command) => command.recv().await,
        }
    }

    pub(crate) async fn kill(&self) {
        match &self.inner {
            CommandInner::Microsandbox(command) => {
                let _ = command.exec.kill().await;
            }
            #[cfg(test)]
            CommandInner::Scripted(command) => command.kill(),
        }
    }

    pub(crate) async fn close(self) {
        match self.inner {
            CommandInner::Microsandbox(command) => {
                command.sandbox.detach().await;
            }
            #[cfg(test)]
            CommandInner::Scripted(_) => {}
        }
    }
}

#[cfg(test)]
impl ScriptedCommand {
    pub(super) fn output(text: String, code: i32) -> Self {
        Self {
            events: Mutex::new(vec![CommandEvent::Output(text), CommandEvent::Exited(code)]),
            hang: false,
            killed: Mutex::new(false),
            notify: Arc::new(Notify::new()),
        }
    }

    pub(super) fn hang() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            hang: true,
            killed: Mutex::new(false),
            notify: Arc::new(Notify::new()),
        }
    }

    async fn recv(&self) -> Option<CommandEvent> {
        loop {
            if *lock_mutex(&self.killed) {
                return Some(CommandEvent::Exited(137));
            }
            {
                let mut events = lock_mutex(&self.events);
                if !events.is_empty() {
                    return Some(events.remove(0));
                }
            }
            if !self.hang {
                return None;
            }
            let notified = self.notify.notified();
            if *lock_mutex(&self.killed) {
                return Some(CommandEvent::Exited(137));
            }
            notified.await;
        }
    }

    fn kill(&self) {
        *lock_mutex(&self.killed) = true;
        self.notify.notify_waiters();
    }
}

pub(super) fn map_exec_error(error: microsandbox::MicrosandboxError) -> SandboxError {
    super::map_error(error, SandboxError::Exec)
}
