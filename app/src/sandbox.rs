use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{Mutex as AsyncMutex, Notify};

mod command;
mod project;

#[cfg(test)]
mod tests;

pub(crate) use command::{CommandEvent, CommandSession};

const SANDBOX_NAME: &str = "powerplant";
const SANDBOX_IMAGE: &str = "alpine";
const SANDBOX_OWNER_LABEL: &str = "works.powerplant.owner";
const SANDBOX_OWNER_VALUE: &str = "powerplant";

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
    pub(crate) project: String,
}

impl SandboxView {
    pub(crate) fn missing_message(&self) -> &'static str {
        self.missing
            .map(MissingRuntime::message)
            .unwrap_or_default()
    }
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
    ProjectLocked,
    ProjectStore,
    Active,
    NotRunning,
    Exec,
    #[allow(dead_code)]
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
            Self::ProjectLocked => "Stop the sandbox before you change the project.",
            Self::ProjectStore => "Power Plant could not store the project path. Try again.",
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

struct Live {
    missing: Mutex<Option<MissingRuntime>>,
    overlay: Mutex<Overlay>,
    progress: Mutex<String>,
    last_error: Mutex<Option<&'static str>>,
    project: Mutex<Option<PathBuf>>,
    notify: Notify,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Overlay {
    Idle,
    Starting,
}

pub(crate) struct GuestSandbox {
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
    project_file: PathBuf,
}

#[cfg(test)]
struct ScriptedGuest {
    live: Arc<Live>,
    status: Mutex<GuestStatus>,
    hang_command: Mutex<bool>,
    lock: Arc<AsyncMutex<()>>,
}

impl Live {
    fn new(missing: Option<MissingRuntime>, project: Option<PathBuf>) -> Self {
        Self {
            missing: Mutex::new(missing),
            overlay: Mutex::new(Overlay::Idle),
            progress: Mutex::new(String::new()),
            last_error: Mutex::new(None),
            project: Mutex::new(project),
            notify: Notify::new(),
        }
    }

    fn project(&self) -> Option<PathBuf> {
        lock_mutex(&self.project).clone()
    }

    fn set_project(&self, project: Option<PathBuf>) {
        *lock_mutex(&self.project) = project;
    }

    fn project_display(&self) -> String {
        self.project()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn missing(&self) -> Option<MissingRuntime> {
        *lock_mutex(&self.missing)
    }

    fn set_missing(&self, missing: Option<MissingRuntime>) {
        *lock_mutex(&self.missing) = missing;
    }

    fn overlay(&self) -> Overlay {
        *lock_mutex(&self.overlay)
    }

    fn begin_start(&self) -> Result<bool, SandboxError> {
        if let Some(missing) = self.missing() {
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
                self.set_missing(Some(missing));
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

    fn snapshot_with_error(&self, status: GuestStatus, error: Option<&'static str>) -> SandboxView {
        let missing = self.missing();
        let project = self.project_display();
        if missing.is_some() {
            return SandboxView {
                missing,
                status: GuestStatus::Stopped,
                progress: String::new(),
                error: "",
                project,
            };
        }
        if self.overlay() == Overlay::Starting {
            return SandboxView {
                missing: None,
                status: GuestStatus::Starting,
                progress: lock_mutex(&self.progress).clone(),
                error: "",
                project,
            };
        }
        SandboxView {
            missing: None,
            status,
            progress: String::new(),
            error: error.unwrap_or_else(|| lock_mutex(&self.last_error).unwrap_or("")),
            project,
        }
    }

    fn snapshot(&self, status: GuestStatus) -> SandboxView {
        self.snapshot_with_error(status, None)
    }

    async fn wait_until_changed(&self, previous: SandboxView, hold: Duration, status: GuestStatus) {
        let notified = self.notify.notified();
        if self.snapshot(status) != previous {
            return;
        }
        let _ = tokio::time::timeout(hold, notified).await;
    }
}

impl GuestSandbox {
    pub(crate) async fn prepare(project_file: PathBuf) -> Self {
        if !microsandbox::setup::is_installed() {
            tracing::info!("installing microsandbox runtime");
            if microsandbox::setup::install().await.is_err() {
                tracing::error!(
                    operation = "install microsandbox runtime",
                    "operational request failure"
                );
            }
        }
        let project = project::load(&project_file);
        Self {
            inner: Inner::Microsandbox(MicrosandboxGuest {
                live: Arc::new(Live::new(inspect_runtime(), project)),
                lock: Arc::new(AsyncMutex::new(())),
                project_file,
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            inner: Inner::Scripted(ScriptedGuest {
                live: Arc::new(Live::new(None, None)),
                status: Mutex::new(GuestStatus::Stopped),
                hang_command: Mutex::new(false),
                lock: Arc::new(AsyncMutex::new(())),
            }),
        }
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
    pub(crate) fn project(&self) -> Option<PathBuf> {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.live.project(),
            #[cfg(test)]
            Inner::Scripted(guest) => guest.live.project(),
        }
    }

    // Persist and memory must stay aligned with the mounted guest path.
    pub(crate) async fn set_project(&self, path: Option<PathBuf>) -> Result<(), SandboxError> {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.set_project(path).await,
            #[cfg(test)]
            Inner::Scripted(guest) => guest.set_project(path),
        }
    }

    pub(crate) async fn view(&self) -> SandboxView {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.view().await,
            #[cfg(test)]
            Inner::Scripted(guest) => guest.live.snapshot(*lock_mutex(&guest.status)),
        }
    }

    pub(crate) async fn wait_until_changed(&self, previous: SandboxView, hold: Duration) {
        match &self.inner {
            Inner::Microsandbox(guest) => {
                let status = if guest.live.overlay() == Overlay::Starting {
                    GuestStatus::Starting
                } else {
                    current_status().await.unwrap_or(GuestStatus::Unavailable)
                };
                guest.live.wait_until_changed(previous, hold, status).await;
            }
            #[cfg(test)]
            Inner::Scripted(guest) => {
                let status = *lock_mutex(&guest.status);
                guest.live.wait_until_changed(previous, hold, status).await;
            }
        }
    }

    pub(crate) async fn start(&self) -> Result<(), SandboxError> {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.start().await,
            #[cfg(test)]
            Inner::Scripted(guest) => {
                if guest.live.project().is_none() {
                    return Err(SandboxError::NeedProject);
                }
                if *lock_mutex(&guest.status) == GuestStatus::Running
                    && guest.live.overlay() == Overlay::Idle
                {
                    return Ok(());
                }
                if guest.live.overlay() == Overlay::Starting {
                    return Ok(());
                }
                let _guard = guest.lock.try_lock().map_err(|_| SandboxError::Active)?;
                guest.live.begin_start().map(|_| ())
            }
        }
    }

    pub(crate) async fn exec(&self, command: &str) -> Result<CommandSession, SandboxError> {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.exec(command).await,
            #[cfg(test)]
            Inner::Scripted(guest) => guest.exec(command),
        }
    }

    pub(crate) async fn stop(&self) -> Result<(), SandboxError> {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.stop().await,
            #[cfg(test)]
            Inner::Scripted(guest) => guest.stop(),
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn remove(&self) -> Result<(), SandboxError> {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.remove().await,
            #[cfg(test)]
            Inner::Scripted(guest) => guest.remove(),
        }
    }
}

impl MicrosandboxGuest {
    async fn view(&self) -> SandboxView {
        if self.live.missing().is_some() || self.live.overlay() == Overlay::Starting {
            return self.live.snapshot(GuestStatus::Stopped);
        }
        match current_status().await {
            Ok(status) => self.live.snapshot(status),
            Err(error) => self
                .live
                .snapshot_with_error(GuestStatus::Unavailable, Some(error.message())),
        }
    }

    async fn set_project(&self, path: Option<PathBuf>) -> Result<(), SandboxError> {
        reject_project_change(self.live.overlay(), current_status().await?)?;
        let _guard = self.lock.try_lock().map_err(|_| SandboxError::Active)?;
        reject_project_change(self.live.overlay(), current_status().await?)?;
        let resolved = path.as_deref().map(project::resolve_dir).transpose()?;
        project::persist(Some(&self.project_file), resolved.as_deref())?;
        self.live.set_project(resolved);
        Ok(())
    }

    async fn start(&self) -> Result<(), SandboxError> {
        let project = required_project(&self.live)?;
        if self.live.missing().is_none()
            && current_status().await? == GuestStatus::Running
            && running_mount_matches(&project).await?
        {
            return Ok(());
        }
        if self.live.overlay() == Overlay::Starting {
            return Ok(());
        }
        let lock = self.lock.clone();
        let _busy = lock.try_lock().map_err(|_| SandboxError::Active)?;
        if !self.live.begin_start()? {
            return Ok(());
        }
        drop(_busy);
        let live = self.live.clone();
        tokio::spawn(async move {
            let _guard = lock.lock().await;
            let result = async {
                let project = required_project(&live)?;
                start_sandbox(&live, project).await
            }
            .await;
            live.finish_start(result);
        });
        Ok(())
    }

    async fn exec(&self, command: &str) -> Result<CommandSession, SandboxError> {
        if let Some(missing) = self.live.missing() {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        let project = required_project(&self.live)?;
        let lifecycle = self
            .lock
            .clone()
            .try_lock_owned()
            .map_err(|_| SandboxError::Active)?;
        Ok(exec_command(command, &project)
            .await?
            .with_lifecycle(lifecycle))
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        if let Some(missing) = self.live.missing() {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        let _guard = self.lock.try_lock().map_err(|_| SandboxError::Active)?;
        stop_sandbox().await
    }

    #[allow(dead_code)]
    async fn remove(&self) -> Result<(), SandboxError> {
        if let Some(missing) = self.live.missing() {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        let _guard = self.lock.try_lock().map_err(|_| SandboxError::Active)?;
        remove_sandbox().await
    }
}

#[cfg(test)]
impl ScriptedGuest {
    fn set_project(&self, path: Option<PathBuf>) -> Result<(), SandboxError> {
        reject_project_change(self.live.overlay(), *lock_mutex(&self.status))?;
        let _guard = self.lock.try_lock().map_err(|_| SandboxError::Active)?;
        reject_project_change(self.live.overlay(), *lock_mutex(&self.status))?;
        let resolved = path.as_deref().map(project::resolve_dir).transpose()?;
        self.live.set_project(resolved);
        Ok(())
    }

    fn exec(&self, command: &str) -> Result<CommandSession, SandboxError> {
        if let Some(missing) = self.live.missing() {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        if self.live.project().is_none() {
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
        let session = if *lock_mutex(&self.hang_command) {
            *lock_mutex(&self.hang_command) = false;
            command::ScriptedCommand::hang()
        } else {
            command::ScriptedCommand::output(command.to_owned(), 0)
        };
        Ok(CommandSession::scripted(session).with_lifecycle(lifecycle))
    }

    fn stop(&self) -> Result<(), SandboxError> {
        if let Some(missing) = self.live.missing() {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        let _guard = self.lock.try_lock().map_err(|_| SandboxError::Active)?;
        *lock_mutex(&self.status) = GuestStatus::Stopped;
        Ok(())
    }

    fn remove(&self) -> Result<(), SandboxError> {
        self.stop()
    }
}

// A live or starting guest still mounts the previous project.
fn reject_project_change(overlay: Overlay, status: GuestStatus) -> Result<(), SandboxError> {
    if overlay == Overlay::Starting {
        return Err(SandboxError::Busy);
    }
    match status {
        GuestStatus::Running | GuestStatus::Starting => Err(SandboxError::ProjectLocked),
        GuestStatus::Unavailable => Err(SandboxError::Inspect),
        GuestStatus::Stopped | GuestStatus::Crashed => Ok(()),
    }
}

async fn current_status() -> Result<GuestStatus, SandboxError> {
    match microsandbox::Sandbox::get(SANDBOX_NAME).await {
        Ok(handle) => {
            ensure_owned(&handle)?;
            Ok(map_status(handle.status_snapshot()))
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => Ok(GuestStatus::Stopped),
        Err(error) => Err(map_error(error, SandboxError::Inspect)),
    }
}

async fn start_sandbox(live: &Live, project: PathBuf) -> Result<(), SandboxError> {
    let project = project::resolve_dir(&project)?;
    live.set_project(Some(project.clone()));
    match microsandbox::Sandbox::get(SANDBOX_NAME).await {
        Ok(handle) => {
            ensure_owned(&handle)?;
            if !mount_matches(&handle, &project) {
                return replace_with_project(handle, live, &project).await;
            }
            match map_status(handle.status_snapshot()) {
                GuestStatus::Running => reconnect(handle).await,
                GuestStatus::Starting => Ok(()),
                GuestStatus::Stopped => start_existing(handle).await,
                GuestStatus::Crashed => recover_crashed(handle, live, &project).await,
                GuestStatus::Unavailable => Err(SandboxError::Inspect),
            }
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => {
            create_detached(live, &project, false).await
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

async fn start_existing(handle: microsandbox::sandbox::SandboxHandle) -> Result<(), SandboxError> {
    match handle.start_detached().await {
        Ok(sandbox) => {
            sandbox.detach().await;
            Ok(())
        }
        Err(error) => Err(map_error(error, SandboxError::Start)),
    }
}

async fn recover_crashed(
    handle: microsandbox::sandbox::SandboxHandle,
    live: &Live,
    project: &Path,
) -> Result<(), SandboxError> {
    match handle.start_detached().await {
        Ok(sandbox) => {
            sandbox.detach().await;
            Ok(())
        }
        Err(_) => replace_with_project(handle, live, project).await,
    }
}

async fn replace_with_project(
    handle: microsandbox::sandbox::SandboxHandle,
    live: &Live,
    project: &Path,
) -> Result<(), SandboxError> {
    let _ = handle.stop().await;
    match microsandbox::Sandbox::remove(SANDBOX_NAME).await {
        Ok(()) | Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => {
            create_detached(live, project, true).await
        }
        Err(error) => Err(map_error(error, SandboxError::Start)),
    }
}

async fn create_detached(live: &Live, project: &Path, replace: bool) -> Result<(), SandboxError> {
    let mut builder = microsandbox::Sandbox::builder(SANDBOX_NAME)
        .image(SANDBOX_IMAGE)
        .label(SANDBOX_OWNER_LABEL, SANDBOX_OWNER_VALUE)
        .volume(project::GUEST_PROJECT, |mount| mount.bind(project))
        .workdir(project::GUEST_PROJECT)
        .detached(true);
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

fn required_project(live: &Live) -> Result<PathBuf, SandboxError> {
    let Some(path) = live.project() else {
        return Err(SandboxError::NeedProject);
    };
    project::resolve_dir(&path)
}

async fn running_mount_matches(project: &Path) -> Result<bool, SandboxError> {
    match microsandbox::Sandbox::get(SANDBOX_NAME).await {
        Ok(handle) => {
            ensure_owned(&handle)?;
            Ok(mount_matches(&handle, project))
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => Ok(false),
        Err(error) => Err(map_error(error, SandboxError::Inspect)),
    }
}

fn mount_matches(handle: &microsandbox::sandbox::SandboxHandle, project: &Path) -> bool {
    handle
        .config()
        .ok()
        .and_then(|config| project::mounted_bind(&config))
        .is_some_and(|host| host == project)
}

async fn exec_command(command: &str, project: &Path) -> Result<CommandSession, SandboxError> {
    let handle = match microsandbox::Sandbox::get(SANDBOX_NAME).await {
        Ok(handle) => handle,
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => {
            return Err(SandboxError::NotRunning);
        }
        Err(error) => return Err(map_error(error, SandboxError::Exec)),
    };
    ensure_owned(&handle)?;
    if map_status(handle.status_snapshot()) != GuestStatus::Running {
        return Err(SandboxError::NotRunning);
    }
    if !mount_matches(&handle, project) {
        return Err(SandboxError::NeedProject);
    }
    let sandbox = handle
        .connect()
        .await
        .map_err(|error| map_error(error, SandboxError::Exec))?;
    let cwd = project::GUEST_PROJECT.to_owned();
    let script = command.to_owned();
    match sandbox
        .exec_stream_with("sh", |options| options.args(["-c", &script]).cwd(cwd))
        .await
    {
        Ok(exec) => Ok(CommandSession::microsandbox(sandbox, exec)),
        Err(error) => {
            sandbox.detach().await;
            Err(command::map_exec_error(error))
        }
    }
}

async fn stop_sandbox() -> Result<(), SandboxError> {
    match microsandbox::Sandbox::get(SANDBOX_NAME).await {
        Ok(handle) => {
            ensure_owned(&handle)?;
            handle
                .stop()
                .await
                .map_err(|error| map_error(error, SandboxError::Stop))
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => Ok(()),
        Err(error) => Err(map_error(error, SandboxError::Stop)),
    }
}

#[allow(dead_code)]
async fn remove_sandbox() -> Result<(), SandboxError> {
    match microsandbox::Sandbox::get(SANDBOX_NAME).await {
        Ok(handle) => {
            ensure_owned(&handle)?;
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

fn ensure_owned(handle: &microsandbox::sandbox::SandboxHandle) -> Result<(), SandboxError> {
    let config = handle.config().map_err(|_| SandboxError::Inspect)?;
    if owns_sandbox(&config.spec.labels) {
        Ok(())
    } else {
        Err(SandboxError::Ownership)
    }
}

fn owns_sandbox(labels: &BTreeMap<String, String>) -> bool {
    labels.get(SANDBOX_OWNER_LABEL).map(String::as_str) == Some(SANDBOX_OWNER_VALUE)
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
