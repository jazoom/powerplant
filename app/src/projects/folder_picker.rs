use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;

#[cfg(test)]
use std::collections::VecDeque;
#[cfg(test)]
use std::sync::Mutex;

#[derive(Clone)]
pub(crate) struct ProjectFolderPicker {
    inner: Arc<Inner>,
}

struct Inner {
    permit: Arc<Semaphore>,
    source: Source,
}

enum Source {
    Native,
    #[cfg(test)]
    Scripted(Mutex<VecDeque<Option<PathBuf>>>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FolderPick {
    Selected(PathBuf),
    Cancelled,
    Busy,
}

impl ProjectFolderPicker {
    pub(crate) fn native() -> Self {
        Self {
            inner: Arc::new(Inner {
                permit: Arc::new(Semaphore::new(1)),
                source: Source::Native,
            }),
        }
    }

    pub(crate) async fn pick(&self) -> FolderPick {
        // One host dialog at a time. The permit stays held until the dialog future settles.
        let Ok(_permit) = self.inner.permit.clone().try_acquire_owned() else {
            return FolderPick::Busy;
        };
        match &self.inner.source {
            Source::Native => match rfd::AsyncFileDialog::new()
                .set_title("Choose project folder")
                .pick_folder()
                .await
            {
                Some(handle) => FolderPick::Selected(handle.path().to_path_buf()),
                None => FolderPick::Cancelled,
            },
            #[cfg(test)]
            Source::Scripted(script) => match lock(script).pop_front().flatten() {
                Some(path) => FolderPick::Selected(path),
                None => FolderPick::Cancelled,
            },
        }
    }
}

#[cfg(test)]
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
