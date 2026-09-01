//! Structured error types for the wfdiag application.
//!
//! This module provides rich error types that serialize to JSON for
//! better frontend error handling and debugging.

use serde::Serialize;
use thiserror::Error;

/// Main error type for diagnostic operations.
#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "details")]
pub enum DiagError {
    /// Session-related errors
    #[error("No active diagnostic session")]
    NoActiveSession,

    #[error("Session ID mismatch: expected {expected}, got {actual}")]
    SessionMismatch { expected: String, actual: String },

    /// Task-related errors
    #[error("No valid tasks provided for diagnostics")]
    NoValidTasks,

    #[error("Diagnostic task failed: {task_id}")]
    TaskFailed { task_id: String, reason: String },

    #[error("Task not found: {task_id}")]
    TaskNotFound { task_id: String },

    /// Export-related errors
    #[error("Unsupported export format: {format}")]
    UnsupportedFormat { format: String },

    #[error("Serialization failed")]
    SerializationError { reason: String },

    /// Storage and file operation errors
    #[error("Storage operation failed: {operation}")]
    StorageError { operation: String, reason: String },

    #[error("File operation failed: {path}")]
    FileError { path: String, reason: String },

    #[error("Parent directory does not exist: {path}")]
    ParentNotExists { path: String },

    #[error("Path validation failed: {path}")]
    PathValidation { path: String, reason: String },

    /// Monitoring-related errors
    #[error("No active monitoring session")]
    NoActiveMonitoring,

    #[error("Failed to create system monitor")]
    MonitorCreationFailed { reason: String },

    /// AI service errors
    #[error("AI service unavailable: {provider}")]
    AiUnavailable { provider: String, reason: String },

    #[error("AI analysis failed")]
    AiAnalysisFailed { reason: String },

    /// Security and API key errors
    #[error("API key operation failed")]
    ApiKeyError { operation: String, reason: String },

    /// Platform-specific errors
    #[error("Operation not supported on this platform")]
    PlatformNotSupported { operation: String },

    /// WMI query errors (Windows-specific)
    #[error("WMI query failed: {query}")]
    WmiError { query: String, code: Option<i32> },

    /// Generic internal error for unexpected failures
    #[error("Internal error: {message}")]
    Internal { message: String },
}

impl DiagError {
    /// Create a task failed error
    pub fn task_failed(task_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::TaskFailed {
            task_id: task_id.into(),
            reason: reason.into(),
        }
    }

    /// Create a storage error
    pub fn storage(operation: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::StorageError {
            operation: operation.into(),
            reason: reason.into(),
        }
    }

    /// Create a file error
    pub fn file(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::FileError {
            path: path.into(),
            reason: reason.into(),
        }
    }

    /// Create a path validation error
    pub fn path_validation(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::PathValidation {
            path: path.into(),
            reason: reason.into(),
        }
    }

    /// Create an AI unavailable error
    pub fn ai_unavailable(provider: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::AiUnavailable {
            provider: provider.into(),
            reason: reason.into(),
        }
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Create a serialization error
    pub fn serialization(reason: impl Into<String>) -> Self {
        Self::SerializationError {
            reason: reason.into(),
        }
    }

    /// Create a WMI error
    pub fn wmi(query: impl Into<String>, code: Option<i32>) -> Self {
        Self::WmiError {
            query: query.into(),
            code,
        }
    }

    /// Create a monitor creation failed error
    pub fn monitor_failed(reason: impl Into<String>) -> Self {
        Self::MonitorCreationFailed {
            reason: reason.into(),
        }
    }

    /// Create an API key error
    pub fn api_key(operation: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ApiKeyError {
            operation: operation.into(),
            reason: reason.into(),
        }
    }
}

/// Convert `DiagError` to String for Tauri command return types.
///
/// This provides JSON-serialized error information that the frontend
/// can parse for structured error handling.
impl From<DiagError> for String {
    fn from(e: DiagError) -> String {
        serde_json::to_string(&e).unwrap_or_else(|_| e.to_string())
    }
}

/// Convenience trait for converting anyhow errors to `DiagError`
pub trait IntoDiagError<T> {
    /// Replace any error with the `DiagError` `f` builds.
    ///
    /// # Errors
    /// Returns `f()` whenever `self` is `Err`.
    fn with_context(self, f: impl FnOnce() -> DiagError) -> Result<T, DiagError>;
}

impl<T, E: std::error::Error> IntoDiagError<T> for Result<T, E> {
    fn with_context(self, f: impl FnOnce() -> DiagError) -> Result<T, DiagError> {
        self.map_err(|_| f())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_serialization() {
        let err = DiagError::TaskFailed {
            task_id: "cpu_info".to_string(),
            reason: "WMI query timeout".to_string(),
        };

        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("TaskFailed"));
        assert!(json.contains("cpu_info"));
    }

    #[test]
    fn test_error_to_string() {
        let err = DiagError::NoActiveSession;
        let s: String = err.into();
        assert!(s.contains("NoActiveSession"));
    }
}
