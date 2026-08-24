//! Secret-safe conversion of operational failures into HTTP responses.

use std::{any::type_name, error::Error, fmt};

use axum::response::{IntoResponse, Response};

use crate::responses;

#[cfg(test)]
static TRACING_TEST_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
static TRACING_TEST_INITIALISED: std::sync::Once = std::sync::Once::new();
#[cfg(test)]
static TRACING_TEST_OUTPUT: std::sync::Mutex<Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
#[derive(Clone, Copy)]
struct TracingTestWriter;

#[cfg(test)]
impl std::io::Write for TracingTestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let output = TRACING_TEST_OUTPUT.lock().unwrap().clone();
        if let Some(output) = output
            && let Ok(mut bytes_out) = output.lock()
        {
            bytes_out.extend_from_slice(bytes);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) struct TracingTestGuard {
    output: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
}

#[cfg(test)]
impl TracingTestGuard {
    pub(crate) fn output(&self) -> String {
        let bytes = self
            .output
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        String::from_utf8(bytes).unwrap_or_default()
    }
}

#[cfg(test)]
impl Drop for TracingTestGuard {
    fn drop(&mut self) {
        if let Ok(mut output) = TRACING_TEST_OUTPUT.lock() {
            *output = None;
        }
        TRACING_TEST_ACTIVE.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
pub(crate) fn tracing_test_guard() -> TracingTestGuard {
    while TRACING_TEST_ACTIVE
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::Acquire,
            std::sync::atomic::Ordering::Relaxed,
        )
        .is_err()
    {
        std::thread::yield_now();
    }
    TRACING_TEST_INITIALISED.call_once(|| {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .without_time()
            .with_ansi(false)
            .with_writer(|| TracingTestWriter)
            .try_init()
            .expect("test tracing subscriber should initialise once");
    });
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    *TRACING_TEST_OUTPUT.lock().unwrap() = Some(output.clone());
    TracingTestGuard { output }
}

pub(crate) type AppResult<T> = Result<T, AppError>;

pub(crate) trait AppResultExt<T> {
    fn with_operation(self, operation: &'static str) -> AppResult<T>;
}

impl<T, E> AppResultExt<T> for Result<T, E>
where
    E: Error + Send + Sync + 'static,
{
    fn with_operation(self, operation: &'static str) -> AppResult<T> {
        self.map_err(|source| AppError::new(operation, source))
    }
}

/// Unexpected failure. Trace the source type only. Never log the source display string.
pub(crate) struct AppError {
    operation: &'static str,
    source_type: &'static str,
    source: Box<dyn Error + Send + Sync>,
}

impl AppError {
    pub(crate) fn new<E>(operation: &'static str, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            operation,
            source_type: type_name::<E>(),
            source: Box::new(source),
        }
    }
}

impl fmt::Debug for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppError")
            .field("operation", &self.operation)
            .field("source_type", &self.source_type)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("operational request failure")
    }
}

impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

impl From<hypergraft::PatchBuildError> for AppError {
    fn from(source: hypergraft::PatchBuildError) -> Self {
        Self::new("build Hypergraft response", source)
    }
}

impl From<hypergraft::InvalidNavigation> for AppError {
    fn from(source: hypergraft::InvalidNavigation) -> Self {
        Self::new("build Hypergraft navigation", source)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        record_operation_failure(self.operation, self.source_type);
        responses::internal_error_response()
    }
}

pub(crate) fn trace_operation_failure<E>(operation: &'static str, _source: &E)
where
    E: Error + 'static,
{
    record_operation_failure(operation, type_name::<E>());
}

fn record_operation_failure(operation: &'static str, source_type: &'static str) {
    tracing::error!(
        operation,
        source = source_type,
        "operational request failure"
    );
}

#[cfg(test)]
mod tests;
