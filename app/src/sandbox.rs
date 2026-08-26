use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{Mutex as AsyncMutex, Notify};

#[cfg(test)]
mod tests;

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
}

#[cfg(test)]
struct ScriptedGuest {
    live: Arc<Live>,
    status: Mutex<GuestStatus>,
}

impl Live {
    fn new(missing: Option<MissingRuntime>) -> Self {
        Self {
            missing: Mutex::new(missing),
            overlay: Mutex::new(Overlay::Idle),
            progress: Mutex::new(String::new()),
            last_error: Mutex::new(None),
            notify: Notify::new(),
        }
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
    pub(crate) async fn prepare() -> Self {
        if !microsandbox::setup::is_installed() {
            tracing::info!("installing microsandbox runtime");
            if microsandbox::setup::install().await.is_err() {
                tracing::error!(
                    operation = "install microsandbox runtime",
                    "operational request failure"
                );
            }
        }
        Self {
            inner: Inner::Microsandbox(MicrosandboxGuest {
                live: Arc::new(Live::new(inspect_runtime())),
                lock: Arc::new(AsyncMutex::new(())),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            inner: Inner::Scripted(ScriptedGuest {
                live: Arc::new(Live::new(None)),
                status: Mutex::new(GuestStatus::Stopped),
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
                if *lock_mutex(&guest.status) == GuestStatus::Running
                    && guest.live.overlay() == Overlay::Idle
                {
                    return Ok(());
                }
                guest.live.begin_start().map(|_| ())
            }
        }
    }

    pub(crate) async fn stop(&self) -> Result<(), SandboxError> {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.stop().await,
            #[cfg(test)]
            Inner::Scripted(guest) => {
                if let Some(missing) = guest.live.missing() {
                    return Err(SandboxError::Missing(missing));
                }
                if guest.live.overlay() == Overlay::Starting {
                    return Err(SandboxError::Busy);
                }
                *lock_mutex(&guest.status) = GuestStatus::Stopped;
                Ok(())
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn remove(&self) -> Result<(), SandboxError> {
        match &self.inner {
            Inner::Microsandbox(guest) => guest.remove().await,
            #[cfg(test)]
            Inner::Scripted(guest) => {
                if let Some(missing) = guest.live.missing() {
                    return Err(SandboxError::Missing(missing));
                }
                if guest.live.overlay() == Overlay::Starting {
                    return Err(SandboxError::Busy);
                }
                *lock_mutex(&guest.status) = GuestStatus::Stopped;
                Ok(())
            }
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

    async fn start(&self) -> Result<(), SandboxError> {
        if self.live.missing().is_none() && current_status().await? == GuestStatus::Running {
            return Ok(());
        }
        if !self.live.begin_start()? {
            return Ok(());
        }
        let live = self.live.clone();
        let lock = self.lock.clone();
        tokio::spawn(async move {
            let _guard = lock.lock().await;
            let result = start_sandbox(&live).await;
            live.finish_start(result);
        });
        Ok(())
    }

    async fn stop(&self) -> Result<(), SandboxError> {
        if let Some(missing) = self.live.missing() {
            return Err(SandboxError::Missing(missing));
        }
        if self.live.overlay() == Overlay::Starting {
            return Err(SandboxError::Busy);
        }
        let _guard = self.lock.lock().await;
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
        let _guard = self.lock.lock().await;
        remove_sandbox().await
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

async fn start_sandbox(live: &Live) -> Result<(), SandboxError> {
    match microsandbox::Sandbox::get(SANDBOX_NAME).await {
        Ok(handle) => {
            ensure_owned(&handle)?;
            match map_status(handle.status_snapshot()) {
                GuestStatus::Running => reconnect(handle).await,
                GuestStatus::Starting => Ok(()),
                GuestStatus::Stopped => start_existing(handle).await,
                GuestStatus::Crashed => recover_crashed(handle, live).await,
                GuestStatus::Unavailable => Err(SandboxError::Inspect),
            }
        }
        Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => create_detached(live).await,
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
) -> Result<(), SandboxError> {
    match handle.start_detached().await {
        Ok(sandbox) => {
            sandbox.detach().await;
            Ok(())
        }
        Err(_) => {
            let _ = handle.stop().await;
            match microsandbox::Sandbox::remove(SANDBOX_NAME).await {
                Ok(()) | Err(microsandbox::MicrosandboxError::SandboxNotFound(_)) => {
                    create_detached(live).await
                }
                Err(error) => Err(map_error(error, SandboxError::Start)),
            }
        }
    }
}

async fn create_detached(live: &Live) -> Result<(), SandboxError> {
    let (mut progress, task) = match microsandbox::Sandbox::builder(SANDBOX_NAME)
        .image(SANDBOX_IMAGE)
        .label(SANDBOX_OWNER_LABEL, SANDBOX_OWNER_VALUE)
        .detached(true)
        .create_detached_with_pull_progress()
    {
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

fn map_error(error: microsandbox::MicrosandboxError, failed: SandboxError) -> SandboxError {
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

fn lock_mutex<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
