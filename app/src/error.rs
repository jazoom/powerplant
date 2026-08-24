//! Secret-safe conversion of operational failures into HTTP responses.

use std::{any::type_name, error::Error, fmt};

use axum::response::{IntoResponse, Response};

use crate::responses;

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

pub(crate) fn trace_patch_build_failure(
    operation: &'static str,
    error: &hypergraft::PatchBuildError,
) {
    tracing::error!(
        operation,
        source = type_name::<hypergraft::PatchBuildError>(),
        kind = ?error.kind(),
        "operational request failure"
    );
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
