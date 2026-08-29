use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{Mutex as AsyncMutex, Notify};

use crate::agents::{AgentId, AgentStore, DirectoryPolicy};

mod access;
mod command;

#[cfg(test)]
mod tests;

pub(crate) use crate::agents::GUEST_PROJECT;
pub(crate) use access::GuestAccess;
pub(crate) use command::{CommandEvent, CommandSession};

const LEGACY_SANDBOX_NAME: &str = "powerplant";
const SANDBOX_IMAGE: &str = "alpine/git";
const SANDBOX_OWNER_LABEL: &str = "works.powerplant.owner";
const SANDBOX_OWNER_VALUE: &str = "powerplant";
const SANDBOX_AGENT_LABEL: &str = "works.powerplant.agent";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GuestStatus {
    Running,
    Starting,
    Stopped,
    Crashed,
    Unavailable,
}

impl GuestStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Starting => "starting",
            Self::Stopped => "stopped",
            Self::Crashed => "crashed",
            Self::Unavailable => "unavailable",
        }
    }

    pub(crate) fn is_starting(self) -> bool {
        self == Self::Starting
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MissingRuntime {
    Msb,
    Libkrunfw,
    Both,
}

impl MissingRuntime {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Msb => {
                "Power Plant cannot find the microsandbox program (`msb`). Install the microsandbox runtime, then start Power Plant again."
            }
            Self::Libkrunfw => {
                "Power Plant cannot find the microsandbox library (`libkrunfw`). Install the microsandbox runtime, then start Power Plant again."
            }
            Self::Both => {
                "Power Plant cannot find the microsandbox runtime (`msb` and `libkrunfw`). Install the microsandbox runtime, then start Power Plant again."
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SandboxView {
    pub(crate) missing: Option<MissingRuntime>,
    pub(crate) status: GuestStatus,
    pub(crate) progress: String,
    pub(crate) error: &'static str,
}

impl SandboxView {
    pub(crate) fn missing_message(&self) -> &'static str {
        self.missing
            .map(MissingRuntime::message)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MountSpec {
    pub(crate) guest: String,
    pub(crate) host: PathBuf,
    pub(crate) read_only: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct SandboxSpec {
    pub(crate) mounts: Vec<MountSpec>,
    pub(crate) workdir: String,
    pub(crate) access: GuestAccess,
}

impl SandboxSpec {
    pub(crate) fn from_policy(policy: &DirectoryPolicy, access: GuestAccess) -> Self {
        Self {
            mounts: policy
                .grants()
                .iter()
                .map(|grant| MountSpec {
                    guest: grant.guest_path.clone(),
                    host: grant.host_path.clone(),
                    read_only: !grant.access.is_writable(),
                })
                .collect(),
            workdir: policy.primary_guest().to_owned(),
            access,
        }
    }
}

impl PartialEq for SandboxSpec {
    fn eq(&self, other: &Self) -> bool {
        self.workdir == other.workdir
            && self.mounts == other.mounts
            && self.access.host == other.access.host
            && self.access.secret.as_ref().map(|secret| secret.expose())
                == other.access.secret.as_ref().map(|secret| secret.expose())
    }
}

impl Eq for SandboxSpec {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OrphanSandbox {
    pub(crate) name: String,
}

#[derive(Debug)]
pub(crate) enum SandboxError {
    Missing(MissingRuntime),
    Busy,
    Start,
    Stop,
    Inspect,
    Ownership,
    NeedProject,
    DirectoryMissing,
    NotADirectory,
    DirectoryAccess,
    StaleMount,
    ProjectLocked,
    Active,
    NotRunning,
    Exec,
    Remove,
}

impl SandboxError {
    pub(crate) fn message(&self) -> &'static str {
        match self {
            Self::Missing(missing) => missing.message(),
            Self::Busy => "Wait until the sandbox finishes starting.",
            Self::Start => "Power Plant could not start the sandbox. Try again.",
            Self::Stop => "Power Plant could not stop the sandbox. Try again.",
            Self::Inspect => "Power Plant could not read the sandbox status. Try again.",
            Self::Ownership => {
                "Power Plant cannot use the sandbox name because another sandbox owns it."
            }
            Self::NeedProject => "Choose a project directory.",
            Self::DirectoryMissing => "That directory does not exist.",
            Self::NotADirectory => "That path is not a directory.",
            Self::DirectoryAccess => "Power Plant cannot access that directory.",
            Self::StaleMount => "A granted directory is no longer at the saved path.",
            Self::ProjectLocked => "Stop the sandbox before you change the directories.",
            Self::Active => "Wait until the running command finishes.",
            Self::NotRunning => "Start the sandbox.",
            Self::Exec => "Power Plant could not run the command. Try again.",
            Self::Remove => "Power Plant could not remove the sandbox. Try again.",
        }
    }
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for SandboxError {}

struct RuntimePrep {
    missing: Mutex<Option<MissingRuntime>>,
}

struct Live {
    overlay: Mutex<Overlay>,
    progress: Mutex<String>,
    last_error: Mutex<Option<&'static str>>,
    spec: Mutex<Option<SandboxSpec>>,
    notify: Notify,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Overlay {
    Idle,
    Starting,
}

pub(crate) struct GuestExec {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) stdin: Option<Vec<u8>>,
    pub(crate) cwd: String,
}

impl GuestExec {
    pub(crate) fn shell(command: &str) -> Self {
        Self {
            program: "sh".to_owned(),
            args: vec!["-c".to_owned(), command.to_owned()],
            stdin: None,
            cwd: GUEST_PROJECT.to_owned(),
        }
    }

    pub(crate) fn command(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            stdin: None,
            cwd: GUEST_PROJECT.to_owned(),
        }
    }

    pub(crate) fn with_stdin(mut self, stdin: Vec<u8>) -> Self {
        self.stdin = Some(stdin);
        self
    }

    pub(crate) fn in_dir(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = cwd.into();
        self
    }

    #[cfg(test)]
    fn display(&self) -> String {
        let mut line = self.program.clone();
        for arg in &self.args {
            line.push(' ');
            line.push_str(arg);
        }
        line
    }
}

pub(crate) struct SandboxFleet {
    runtime: Arc<RuntimePrep>,
    handles: Mutex<HashMap<AgentId, Arc<GuestSandbox>>>,
    orphans: Mutex<Vec<OrphanSandbox>>,
    scripted: bool,
}

pub(crate) struct GuestSandbox {
    id: AgentId,
    name: String,
    runtime: Arc<RuntimePrep>,
    inner: Inner,
}

enum Inner {
    Microsandbox(MicrosandboxGuest),
    #[cfg(test)]
    Scripted(ScriptedGuest),
}

struct MicrosandboxGuest {
    live: Arc<Live>,
    lock: Arc<AsyncMutex<()>>,
}

#[cfg(test)]
struct ScriptedGuest {
    live: Arc<Live>,
    status: Mutex<GuestStatus>,
    hang_command: Mutex<bool>,
    fail_command: Mutex<bool>,
    last_exec: Mutex<Option<String>>,
    lock: Arc<AsyncMutex<()>>,
}

impl Live {
    fn new() -> Self {
        Self {
            overlay: Mutex::new(Overlay::Idle),
            progress: Mutex::new(String::new()),
            last_error: Mutex::new(None),
            spec: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    fn overlay(&self) -> Overlay {
        *lock_mutex(&self.overlay)
    }

    fn begin_start(&self, missing: Option<MissingRuntime>) -> Result<bool, SandboxError> {
        if let Some(missing) = missing {
            return Err(SandboxError::Missing(missing));
        }
        let mut overlay = lock_mutex(&self.overlay);
        if *overlay == Overlay::Starting {
            return Ok(false);
        }
        *overlay = Overlay::Starting;
        drop(overlay);
        *lock_mutex(&self.progress) = "Starting the virtual machine".to_owned();
        *lock_mutex(&self.last_error) = None;
        self.notify.notify_waiters();
        Ok(true)
    }

    fn finish_start(&self, result: Result<(), SandboxError>) {
        *lock_mutex(&self.overlay) = Overlay::Idle;
        *lock_mutex(&self.progress) = String::new();
        match result {
            Ok(()) => *lock_mutex(&self.last_error) = None,
            Err(SandboxError::Missing(missing)) => {
                *lock_mutex(&self.last_error) = Some(missing.message());
            }
            Err(error) => *lock_mutex(&self.last_error) = Some(error.message()),
        }
        self.notify.notify_waiters();
    }

    fn set_progress(&self, progress: String) {
        *lock_mutex(&self.progress) = progress;
        self.notify.notify_waiters();
    }

    fn snapshot_with_error(
        &self,
        missing: Option<MissingRuntime>,
        status: GuestStatus,
        error: Option<&'static str>,
    ) -> SandboxView {
        if missing.is_some() {
            return SandboxView {
                missing,
                status: GuestStatus::Stopped,
                progress: String::new(),
                error: "",
            };
        }
        if self.overlay() == Overlay::Starting {
            return SandboxView {
                missing: None,
                status: GuestStatus::Starting,
                progress: lock_mutex(&self.progress).clone(),
                error: "",
            };
        }
        SandboxView {
            missing: None,
            status,
            progress: String::new(),
            error: error.unwrap_or_else(|| lock_mutex(&self.last_error).unwrap_or("")),
        }
    }

    fn snapshot(&self, missing: Option<MissingRuntime>, status: GuestStatus) -> SandboxView {
        self.snapshot_with_error(missing, status, None)
    }

    async fn wait_until_changed(
        &self,
        missing: Option<MissingRuntime>,
        previous: SandboxView,
        hold: Duration,
        status: GuestStatus,
    ) {
        let notified = self.notify.notified();
        if self.snapshot(missing, status) != previous {
            return;
        }
        let _ = tokio::time::timeout(hold, notified).await;
    }
}

impl SandboxFleet {
    pub(crate) async fn prepare(agents: &AgentStore) -> Self {
        if !microsandbox::setup::is_installed() {
            tracing::info!("installing microsandbox runtime");
            if microsandbox::setup::install().await.is_err() {
                tracing::error!(
                    operation = "install microsandbox runtime",
                    "operational request failure"
                );
            }
        }
        let runtime = Arc::new(RuntimePrep {
            missing: Mutex::new(inspect_runtime()),
        });
        let orphans = if lock_mutex(&runtime.missing).is_none() {
            collect_orphans(agents).await
        } else {
            Vec::new()
        };
        Self {
            runtime,
            handles: Mutex::new(HashMap::new()),
            orphans: Mutex::new(orphans),
            scripted: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            runtime: Arc::new(RuntimePrep {
                missing: Mutex::new(None),
            }),
            handles: Mutex::new(HashMap::new()),
            orphans: Mutex::new(Vec::new()),
            scripted: true,
        }
    }

    pub(crate) fn missing(&self) -> Option<MissingRuntime> {
        *lock_mutex(&self.runtime.missing)
    }

    pub(crate) fn missing_message(&self) -> &'static str {
        self.missing()
            .map(MissingRuntime::message)
            .unwrap_or_default()
    }

    pub(crate) fn handle(&self, id: AgentId) -> Arc<GuestSandbox> {
        let mut handles = lock_mutex(&self.handles);
        handles
            .entry(id)
            .or_insert_with(|| Arc::new(self.new_handle(id)))
            .clone()
    }

    pub(crate) fn orphans(&self) -> Vec<OrphanSandbox> {
        lock_mutex(&self.orphans).clone()
    }

    pub(crate) async fn remove_orphan(&self, name: &str) -> Result<(), SandboxError> {
        let allowed = lock_mutex(&self.orphans)
            .iter()
            .any(|orphan| orphan.name == name);
        if !allowed {
            return Err(SandboxError::Ownership);
        }
        remove_named(name).await?;
        lock_mutex(&self.orphans).retain(|orphan| orphan.name != name);
        Ok(())
    }

    pub(crate) async fn remove_agent(&self, id: AgentId) -> Result<(), SandboxError> {
        let handle = self.handle(id);
        handle.remove().await?;
        lock_mutex(&self.handles).remove(&id);
        Ok(())
    }

    fn new_handle(&self, id: AgentId) -> GuestSandbox {
        let name = sandbox_name(&id);
        let live = Arc::new(Live::new());
        let inner = if self.scripted {
            #[cfg(test)]
            {
                Inner::Scripted(ScriptedGuest {
                    live,
                    status: Mutex::new(GuestStatus::Stopped),
                    hang_command: Mutex::new(false),
                    fail_command: Mutex::new(false),
                    last_exec: Mutex::new(None),
                    lock: Arc::new(AsyncMutex::new(())),
                })
            }
            #[cfg(not(test))]
            {
                Inner::Microsandbox(MicrosandboxGuest {
                    live,
                    lock: Arc::new(AsyncMutex::new(())),
                })
            }
        } else {
            Inner::Microsandbox(MicrosandboxGuest {
                live,
                lock: Arc::new(AsyncMutex::new(())),
            })
        };
        GuestSandbox {
            id,
            name,
            runtime: self.runtime.clone(),
            inner,
        }
    }
}

impl GuestSandbox {
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        SandboxFleet::for_test().new_handle(AgentId::generate().expect("id"))
    }

    #[cfg(test)]
    pub(crate) fn complete_start(&self) {
        match &self.inner {
            Inner::Microsandbox(_) => {}
            Inner::Scripted(guest) => {
                *lock_mutex(&guest.status) = GuestStatus::Running;
                guest.live.finish_start(Ok(()));
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn hang_next_command(&self) {
        match &self.inner {
            Inner::Microsandbox(_) => {}
            Inner::Scripted(guest) => *lock_mutex(&guest.hang_command) = true,
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_next_command(&self) {
        match &self.inner {
            Inner::Microsandbox(_) => {}
            Inner::Scripted(guest) => *lock_mutex(&guest.fail_command) = true,
        }
    }

    #[cfg(test)]
    pub(crate) fn last_exec(&self) -> Option<String> {
        match &self.inner {
            Inner::Microsandbox(_) => None,
            Inner::Scripted(guest) => lock_mutex(&guest.last_exec).clone(),
        }
    }

    pub(crate) fn missing(&self) -> Option<MissingRuntime> {
        *lock_mutex(&self.runtime.missing)
    }

    pub(crate) async fn view(&self) -> SandboxView {
        let missing = self.missing();
        match &self.inner {
            Inner::Microsandbox(guest) => guest.view(&self.name, self.id, missing).await,
            #[cfg(test)]
            Inner::Scripted(guest) => guest.live.snapshot(missing, *lock_mutex(&guest.status)),
        }
    }

    pub(crate) async fn wait_until_changed(&self, previous: SandboxView, hold: Duration) {
        let missing = self.missing();
        match &self.inner {
            Inner::Microsandbox(guest) => {
                let status = if guest.live.overlay() == Overlay::Starting {
                    GuestStatus::Starting
                } else {
                    current_status(&self.name, self.id)
                        .await
                        .unwrap_or(GuestStatus::Unavailable)
                };
                guest
                    .live
                    .wait_until_changed(missing, previous, hold, status)
                    .await;
            }
            #[cfg(test)]
            Inner::Scripted(guest) => {
                let status = *lock_mutex(&guest.status);
                guest
                    .live
                    .wait_until_changed(missing, previous, hold, status)
                    .await;
            }
        }
    }

    pub(crate) async fn start_with(&self, spec: SandboxSpec) -> Result<(), SandboxError> {
        confirm_mounts(&spec)?;
        match &self.inner {
            Inner::Microsandbox(guest) => {
                guest.start(self.id, &self.name, spec, self.missing()).await
            }
            #[cfg(test)]
            Inner::Scripted(guest) => guest.start(spec, self.missing()),
        }
    }

    pub(crate) async fn exec_cmd(
        &self,
        request: GuestExec,
    ) -> Result<CommandSession, SandboxError> {
        match &self.inner {
            Inner::Microsandbox(guest) => {
                guest
                    .exec(&self.name, self.id, request, self.missing())
                    .await
            }
            #[cfg(test)]
            Inner::Scripted(guest) => guest.exec(request, self.missing()),
        }
    }

    pub(crate) async fn spec_matches(&self, spec: &SandboxSpec) -> Result<bool, SandboxError> {
        match &self.inner {
            Inner::Microsandbox(_) => running_matches(&self.name, self.id, spec).await,
            #[cfg(test)]
            Inner::Scripted(guest) => Ok(lock_mutex(&guest.live.spec)
                .as_ref()
                .is_some_and(|stored| stored == spec)),
        }
    }

    pub(crate) async fn stop(&self) -> Result<(), SandboxError> {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.stop(&self.name, self.id, self.missing()).await,
            #[cfg(test)]
            Inner::Scripted(guest) => guest.stop(self.missing()),
        }
    }

    pub(crate) async fn remove(&self) -> Result<(), SandboxError> {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.remove(&self.name, self.id, self.missing()).await,
            #[cfg(test)]
            Inner::Scripted(guest) => guest.remove(self.missing()),
        }
    }

    pub(crate) async fn reject_if_active(&self) -> Result<(), SandboxError> {
        let view = self.view().await;
        if view.status.is_starting() {
            return Err(SandboxError::Busy);
        }
        match view.status {
            GuestStatus::Running | GuestStatus::Starting => Err(SandboxError::ProjectLocked),
            GuestStatus::Unavailable => Err(SandboxError::Inspect),
            GuestStatus::Stopped | GuestStatus::Crashed => Ok(()),
        }
    }
}

impl MicrosandboxGuest {
    async fn view(&self, name: &str, id: AgentId, missing: Option<MissingRuntime>) -> SandboxView {
        if missing.is_some() || self.live.overlay() == Overlay::Starting {
            return self.live.snapshot(missing, GuestStatus::Stopped);
        }
        match current_status(name, id).await {
            Ok(status) => self.live.snapshot(missing, status),
            Err(error) => self.live.snapshot_with_error(
                missing,
                GuestStatus::Unavailable,
                Some(error.message()),
            ),
        }
    }

    async fn start(
        &self,
        id: AgentId,
        name: &str,
        spec: SandboxSpec,
        missing: Option<MissingRuntime>,
    ) -> Result<(), SandboxError> {
        if spec.mounts.is_empty() {
            return Err(SandboxError::NeedProject);
        }
        if missing.is_none()
            && current_status(name, id).await? == GuestStatus::Running
            && running_matches(name, id, &spec).await?
        {
            return Ok(());
        }
        if self.live.overlay() == Overlay::Starting {
            return Ok(());
        }
        let lock = self.lock.clone();
        let _busy = lock.try_lock().map_err(|_| SandboxError::Active)?;
        if !self.live.begin_start(missing)? {
            return Ok(());
        }
        drop(_busy);
        let live = self.live.clone();
        let name = name.to_owned();
        tokio::spawn(async move {
            let _guard = lock.lock().await;
            let result = start_sandbox(&live, id, &name, spec).await;
            live.finish_start(result);
        });
        Ok(())
    }

    async fn exec(
        &self,
        name: &str,
        id: AgentId,
        request: GuestExec,
        missing: Option<MissingRuntime>,
    ) -> Result<CommandSession, SandboxError> {
        if let Some(missing) = missing {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        let lifecycle = self
            .lock
            .clone()
            .try_lock_owned()
            .map_err(|_| SandboxError::Active)?;
        Ok(exec_command(name, id, request)
            .await?
            .with_lifecycle(lifecycle))
    }

    async fn stop(
        &self,
        name: &str,
        id: AgentId,
        missing: Option<MissingRuntime>,
    ) -> Result<(), SandboxError> {
        if let Some(missing) = missing {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        let _guard = self.lock.try_lock().map_err(|_| SandboxError::Active)?;
        stop_named(name, id).await
    }

    async fn remove(
        &self,
        name: &str,
        id: AgentId,
        missing: Option<MissingRuntime>,
    ) -> Result<(), SandboxError> {
        if let Some(missing) = missing {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        let _guard = self.lock.try_lock().map_err(|_| SandboxError::Active)?;
        remove_owned(name, id).await
    }
}

#[cfg(test)]
impl ScriptedGuest {
    fn start(
        &self,
        spec: SandboxSpec,
        missing: Option<MissingRuntime>,
    ) -> Result<(), SandboxError> {
        if spec.mounts.is_empty() {
            return Err(SandboxError::NeedProject);
        }
        if *lock_mutex(&self.status) == GuestStatus::Running && self.live.overlay() == Overlay::Idle
        {
            return Ok(());
        }
        if self.live.overlay() == Overlay::Starting {
            return Ok(());
        }
        let _guard = self.lock.try_lock().map_err(|_| SandboxError::Active)?;
        *lock_mutex(&self.live.spec) = Some(spec);
        self.live.begin_start(missing).map(|_| ())
    }

    fn exec(
        &self,
        request: GuestExec,
        missing: Option<MissingRuntime>,
    ) -> Result<CommandSession, SandboxError> {
        if let Some(missing) = missing {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        if lock_mutex(&self.live.spec).is_none() {
            return Err(SandboxError::NeedProject);
        }
        if *lock_mutex(&self.status) != GuestStatus::Running {
            return Err(SandboxError::NotRunning);
        }
        let lifecycle = self
            .lock
            .clone()
            .try_lock_owned()
            .map_err(|_| SandboxError::Active)?;
        *lock_mutex(&self.last_exec) = Some(request.display());
        let session = if *lock_mutex(&self.hang_command) {
            *lock_mutex(&self.hang_command) = false;
            command::ScriptedCommand::hang()
        } else if *lock_mutex(&self.fail_command) {
            *lock_mutex(&self.fail_command) = false;
            command::ScriptedCommand::output(request.display(), 1)
        } else {
            command::ScriptedCommand::output(request.display(), 0)
        };
        Ok(CommandSession::scripted(session).with_lifecycle(lifecycle))
    }

    fn stop(&self, missing: Option<MissingRuntime>) -> Result<(), SandboxError> {
        if let Some(missing) = missing {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        let _guard = self.lock.try_lock().map_err(|_| SandboxError::Active)?;
        *lock_mutex(&self.status) = GuestStatus::Stopped;
        Ok(())
    }

    fn remove(&self, missing: Option<MissingRuntime>) -> Result<(), SandboxError> {
        self.stop(missing)
    }
}

fn sandbox_name(id: &AgentId) -> String {
    format!("pp-{}", id.as_hex())
}

fn confirm_mounts(spec: &SandboxSpec) -> Result<(), SandboxError> {
    if spec.mounts.is_empty() {
        return Err(SandboxError::NeedProject);
    }
    for mount in &spec.mounts {
        let resolved = resolve_dir(&mount.host)?;
        if resolved != mount.host {
            return Err(SandboxError::StaleMount);
        }
    }
    Ok(())
}

fn resolve_dir(path: &Path) -> Result<PathBuf, SandboxError> {
    let metadata = std::fs::metadata(path).map_err(map_fs_error)?;
    if !metadata.is_dir() {
        return Err(SandboxError::NotADirectory);
    }
    std::fs::canonicalize(path).map_err(map_fs_error)
}

fn map_fs_error(error: std::io::Error) -> SandboxError {
    match error.kind() {
        std::io::ErrorKind::NotFound => SandboxError::DirectoryMissing,
        std::io::ErrorKind::NotADirectory => SandboxError::NotADirectory,
        _ => SandboxError::DirectoryAccess,
    }
}

async fn current_status(name: &str, id: AgentId) -> Result<GuestStatus, SandboxError> {
    match microsandbox::Sandbox::get(name).await {
        Ok(handle) => {
            ensure_owned(&handle, id)?;
            Ok(map_status(handle.status_snapshot()))
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => Ok(GuestStatus::Stopped),
        Err(error) => Err(map_error(error, SandboxError::Inspect)),
    }
}

async fn start_sandbox(
    live: &Live,
    id: AgentId,
    name: &str,
    spec: SandboxSpec,
) -> Result<(), SandboxError> {
    confirm_mounts(&spec)?;
    *lock_mutex(&live.spec) = Some(spec.clone());
    match microsandbox::Sandbox::get(name).await {
        Ok(handle) => {
            ensure_owned(&handle, id)?;
            if running_config_matches(&handle, &spec) {
                return match map_status(handle.status_snapshot()) {
                    GuestStatus::Running => reconnect(handle).await,
                    GuestStatus::Starting => Ok(()),
                    GuestStatus::Stopped | GuestStatus::Crashed => {
                        replace_sandbox(handle, live, id, name, &spec).await
                    }
                    GuestStatus::Unavailable => Err(SandboxError::Inspect),
                };
            }
            replace_sandbox(handle, live, id, name, &spec).await
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => {
            create_detached(live, id, name, &spec, false).await
        }
        Err(error) => Err(map_error(error, SandboxError::Start)),
    }
}

async fn reconnect(handle: microsandbox::sandbox::SandboxHandle) -> Result<(), SandboxError> {
    match handle.connect().await {
        Ok(sandbox) => {
            sandbox.detach().await;
            Ok(())
        }
        Err(error) => Err(map_error(error, SandboxError::Start)),
    }
}

async fn replace_sandbox(
    handle: microsandbox::sandbox::SandboxHandle,
    live: &Live,
    id: AgentId,
    name: &str,
    spec: &SandboxSpec,
) -> Result<(), SandboxError> {
    let _ = handle.stop().await;
    match microsandbox::Sandbox::remove(name).await {
        Ok(()) | Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => {
            create_detached(live, id, name, spec, true).await
        }
        Err(error) => Err(map_error(error, SandboxError::Start)),
    }
}

async fn create_detached(
    live: &Live,
    id: AgentId,
    name: &str,
    spec: &SandboxSpec,
    replace: bool,
) -> Result<(), SandboxError> {
    let host = spec.access.host.clone();
    let mut builder = microsandbox::Sandbox::builder(name)
        .image(SANDBOX_IMAGE)
        .label(SANDBOX_OWNER_LABEL, SANDBOX_OWNER_VALUE)
        .label(SANDBOX_AGENT_LABEL, id.as_hex())
        .workdir(&spec.workdir)
        .network(|network| network.policy(access::provider_policy(&host)))
        .detached(true);
    for mount in &spec.mounts {
        let guest = mount.guest.clone();
        let host_path = mount.host.clone();
        let read_only = mount.read_only;
        builder = builder.volume(guest, move |volume| {
            let bound = volume.bind(host_path);
            if read_only { bound.readonly() } else { bound }
        });
    }
    if let Some(secret) = &spec.access.secret {
        let value = secret.expose().to_owned();
        builder =
            builder.secret(|entry| entry.env(access::SECRET_ENV).value(value).allow_host(host));
    }
    if replace {
        builder = builder.replace();
    }
    let (mut progress, task) = match builder.create_detached_with_pull_progress() {
        Ok(started) => started,
        Err(error) => return Err(map_error(error, SandboxError::Start)),
    };
    let mut layers = 0usize;
    while let Some(event) = progress.recv().await {
        live.set_progress(pull_copy(&event, &mut layers));
    }
    match task.await {
        Ok(Ok(sandbox)) => {
            sandbox.detach().await;
            Ok(())
        }
        Ok(Err(error)) => Err(map_error(error, SandboxError::Start)),
        Err(_) => Err(SandboxError::Start),
    }
}

async fn running_matches(
    name: &str,
    id: AgentId,
    spec: &SandboxSpec,
) -> Result<bool, SandboxError> {
    match microsandbox::Sandbox::get(name).await {
        Ok(handle) => {
            ensure_owned(&handle, id)?;
            Ok(running_config_matches(&handle, spec))
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => Ok(false),
        Err(error) => Err(map_error(error, SandboxError::Inspect)),
    }
}

fn running_config_matches(
    handle: &microsandbox::sandbox::SandboxHandle,
    spec: &SandboxSpec,
) -> bool {
    mounts_match(handle, spec) && image_matches(handle) && access_matches(handle, &spec.access)
}

fn access_matches(handle: &microsandbox::sandbox::SandboxHandle, access: &GuestAccess) -> bool {
    let Ok(config) = handle.config() else {
        return false;
    };
    let expected_policy = access::provider_policy(&access.host);
    if serde_json::to_value(config.spec.network.policy.as_ref()).ok()
        != serde_json::to_value(Some(&expected_policy)).ok()
    {
        return false;
    }
    let secrets = config
        .spec
        .network
        .secrets
        .as_ref()
        .map(|config| config.secrets.as_slice())
        .unwrap_or_default();
    match &access.secret {
        Some(secret) => {
            let [entry] = secrets else {
                return false;
            };
            entry.env_var == access::SECRET_ENV
                && entry.value.as_str() == secret.expose()
                && entry.allowed_hosts
                    == [microsandbox::sandbox::HostPattern::Exact(
                        access.host.clone(),
                    )]
        }
        None => secrets.is_empty(),
    }
}

fn mounts_match(handle: &microsandbox::sandbox::SandboxHandle, spec: &SandboxSpec) -> bool {
    let Ok(config) = handle.config() else {
        return false;
    };
    let mut found = Vec::new();
    for mount in &config.spec.mounts {
        if let microsandbox::sandbox::VolumeMount::Bind {
            host,
            guest,
            options,
            ..
        } = mount
        {
            found.push((guest.clone(), host.clone(), options.readonly));
        } else {
            return false;
        }
    }
    if found.len() != spec.mounts.len() {
        return false;
    }
    spec.mounts.iter().all(|expected| {
        found.iter().any(|(guest, host, read_only)| {
            guest == &expected.guest && host == &expected.host && *read_only == expected.read_only
        })
    })
}

fn image_matches(handle: &microsandbox::sandbox::SandboxHandle) -> bool {
    handle
        .config()
        .ok()
        .is_some_and(|config| match config.spec.image {
            microsandbox::sandbox::RootfsSource::Oci(oci) => {
                oci.reference == SANDBOX_IMAGE
                    || oci.reference.starts_with(&format!("{SANDBOX_IMAGE}:"))
            }
            _ => false,
        })
}

async fn exec_command(
    name: &str,
    id: AgentId,
    request: GuestExec,
) -> Result<CommandSession, SandboxError> {
    let handle = match microsandbox::Sandbox::get(name).await {
        Ok(handle) => handle,
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => {
            return Err(SandboxError::NotRunning);
        }
        Err(error) => return Err(map_error(error, SandboxError::Exec)),
    };
    ensure_owned(&handle, id)?;
    if map_status(handle.status_snapshot()) != GuestStatus::Running {
        return Err(SandboxError::NotRunning);
    }
    let sandbox = handle
        .connect()
        .await
        .map_err(|error| map_error(error, SandboxError::Exec))?;
    let cwd = request.cwd;
    let program = request.program;
    let args = request.args;
    let stdin = request.stdin;
    match sandbox
        .exec_stream_with(program, |options| {
            let options = options.args(args).cwd(cwd);
            match stdin {
                Some(bytes) => options.stdin_bytes(bytes),
                None => options,
            }
        })
        .await
    {
        Ok(exec) => Ok(CommandSession::microsandbox(sandbox, exec)),
        Err(error) => {
            sandbox.detach().await;
            Err(command::map_exec_error(error))
        }
    }
}

async fn stop_named(name: &str, id: AgentId) -> Result<(), SandboxError> {
    match microsandbox::Sandbox::get(name).await {
        Ok(handle) => {
            ensure_owned(&handle, id)?;
            handle
                .stop()
                .await
                .map_err(|error| map_error(error, SandboxError::Stop))
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => Ok(()),
        Err(error) => Err(map_error(error, SandboxError::Stop)),
    }
}

async fn remove_owned(name: &str, id: AgentId) -> Result<(), SandboxError> {
    match microsandbox::Sandbox::get(name).await {
        Ok(handle) => {
            ensure_owned(&handle, id)?;
            if matches!(
                map_status(handle.status_snapshot()),
                GuestStatus::Running | GuestStatus::Starting
            ) {
                handle
                    .stop()
                    .await
                    .map_err(|error| map_error(error, SandboxError::Remove))?;
            }
            match handle.remove().await {
                Ok(()) | Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => Ok(()),
                Err(error) => Err(map_error(error, SandboxError::Remove)),
            }
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => Ok(()),
        Err(error) => Err(map_error(error, SandboxError::Remove)),
    }
}

async fn remove_named(name: &str) -> Result<(), SandboxError> {
    match microsandbox::Sandbox::get(name).await {
        Ok(handle) => {
            if !owns_owner(&handle) {
                return Err(SandboxError::Ownership);
            }
            if matches!(
                map_status(handle.status_snapshot()),
                GuestStatus::Running | GuestStatus::Starting
            ) {
                handle
                    .stop()
                    .await
                    .map_err(|error| map_error(error, SandboxError::Remove))?;
            }
            match handle.remove().await {
                Ok(()) | Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => Ok(()),
                Err(error) => Err(map_error(error, SandboxError::Remove)),
            }
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => Ok(()),
        Err(error) => Err(map_error(error, SandboxError::Remove)),
    }
}

async fn collect_orphans(agents: &AgentStore) -> Vec<OrphanSandbox> {
    let Ok(handles) = list_owned().await else {
        return Vec::new();
    };
    let mut orphans = Vec::new();
    for handle in handles {
        let Ok(config) = handle.config() else {
            continue;
        };
        if !owns_sandbox_owner(&config.spec.labels) {
            continue;
        }
        let name = handle.name().to_owned();
        if name == LEGACY_SANDBOX_NAME {
            if remove_named(&name).await.is_err() {
                orphans.push(OrphanSandbox { name });
            }
            continue;
        }
        let Some(agent) = config
            .spec
            .labels
            .get(SANDBOX_AGENT_LABEL)
            .and_then(|value| AgentId::parse(value))
        else {
            orphans.push(OrphanSandbox { name });
            continue;
        };
        if agents.get(&agent).is_none() {
            orphans.push(OrphanSandbox { name });
        }
    }
    orphans
}

async fn list_owned() -> Result<Vec<microsandbox::sandbox::SandboxHandle>, SandboxError> {
    let mut cursor = None;
    let mut sandboxes = Vec::new();
    loop {
        let page = microsandbox::Sandbox::list_with(|list| {
            let list = list
                .label(SANDBOX_OWNER_LABEL, SANDBOX_OWNER_VALUE)
                .limit(100);
            match cursor.clone() {
                Some(value) => list.cursor(value),
                None => list,
            }
        })
        .await
        .map_err(|error| map_error(error, SandboxError::Inspect))?;
        sandboxes.extend(page.sandboxes);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(sandboxes)
}

fn map_status(status: microsandbox::sandbox::SandboxStatus) -> GuestStatus {
    match status {
        microsandbox::sandbox::SandboxStatus::Running
        | microsandbox::sandbox::SandboxStatus::Draining => GuestStatus::Running,
        microsandbox::sandbox::SandboxStatus::Starting => GuestStatus::Starting,
        microsandbox::sandbox::SandboxStatus::Crashed => GuestStatus::Crashed,
        microsandbox::sandbox::SandboxStatus::Created
        | microsandbox::sandbox::SandboxStatus::Paused
        | microsandbox::sandbox::SandboxStatus::Stopped => GuestStatus::Stopped,
    }
}

fn ensure_owned(
    handle: &microsandbox::sandbox::SandboxHandle,
    id: AgentId,
) -> Result<(), SandboxError> {
    let config = handle.config().map_err(|_| SandboxError::Inspect)?;
    if owns_agent(&config.spec.labels, id) {
        Ok(())
    } else {
        Err(SandboxError::Ownership)
    }
}

fn owns_owner(handle: &microsandbox::sandbox::SandboxHandle) -> bool {
    handle
        .config()
        .ok()
        .is_some_and(|config| owns_sandbox_owner(&config.spec.labels))
}

fn owns_sandbox_owner(labels: &BTreeMap<String, String>) -> bool {
    labels.get(SANDBOX_OWNER_LABEL).map(String::as_str) == Some(SANDBOX_OWNER_VALUE)
}

fn owns_agent(labels: &BTreeMap<String, String>, id: AgentId) -> bool {
    owns_sandbox_owner(labels)
        && labels.get(SANDBOX_AGENT_LABEL).map(String::as_str) == Some(&id.as_hex())
}

pub(super) fn map_error(
    error: microsandbox::MicrosandboxError,
    failed: SandboxError,
) -> SandboxError {
    match error {
        microsandbox::MicrosandboxError::LibkrunfwNotFound(_) => {
            SandboxError::Missing(MissingRuntime::Libkrunfw)
        }
        _ => failed,
    }
}

fn pull_copy(event: &microsandbox::sandbox::PullProgress, layers: &mut usize) -> String {
    use microsandbox::sandbox::PullProgress;
    match event {
        PullProgress::Resolving { .. } => "Resolving the image".to_owned(),
        PullProgress::Resolved { layer_count, .. } => {
            *layers = *layer_count;
            format!("Resolved {layer_count} layers")
        }
        PullProgress::LayerDownloadProgress {
            layer_index,
            downloaded_bytes,
            total_bytes,
            ..
        } => match total_bytes {
            Some(total) if *total > 0 => format!(
                "Downloading {}, {}%",
                layer_phrase(*layer_index, *layers),
                downloaded_bytes.saturating_mul(100) / total
            ),
            _ => format!("Downloading {}", layer_phrase(*layer_index, *layers)),
        },
        PullProgress::LayerDownloadComplete { layer_index, .. }
        | PullProgress::LayerDownloadVerifying { layer_index, .. } => {
            format!("Verifying {}", layer_phrase(*layer_index, *layers))
        }
        PullProgress::LayerMaterializeStarted { layer_index, .. }
        | PullProgress::LayerMaterializeProgress { layer_index, .. }
        | PullProgress::LayerMaterializeWriting { layer_index, .. }
        | PullProgress::LayerMaterializeComplete { layer_index, .. } => {
            format!("Writing {}", layer_phrase(*layer_index, *layers))
        }
        PullProgress::StitchMergingTrees { .. }
        | PullProgress::StitchWritingFsmeta
        | PullProgress::StitchWritingVmdk
        | PullProgress::StitchComplete => "Combining image layers".to_owned(),
        PullProgress::Complete { .. } => "Starting the virtual machine".to_owned(),
    }
}

fn layer_phrase(index: usize, layers: usize) -> String {
    let number = index + 1;
    if layers == 0 {
        format!("layer {number}")
    } else {
        format!("layer {number} of {layers}")
    }
}

fn inspect_runtime() -> Option<MissingRuntime> {
    if microsandbox::setup::is_installed() {
        return None;
    }
    let diagnosis = microsandbox::setup::diagnose();
    let mut msb = false;
    let mut libkrunfw = false;
    for section in diagnosis.sections {
        for check in section.checks {
            if check.state != microsandbox::setup::CheckState::Fail {
                continue;
            }
            if check.label == "msb" {
                msb = true;
            }
            if check.label == "libkrunfw" {
                libkrunfw = true;
            }
        }
    }
    match (msb, libkrunfw) {
        (true, false) => Some(MissingRuntime::Msb),
        (false, true) => Some(MissingRuntime::Libkrunfw),
        _ => Some(MissingRuntime::Both),
    }
}

pub(super) fn lock_mutex<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
