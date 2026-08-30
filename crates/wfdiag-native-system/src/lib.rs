//! UI-framework-neutral host and architecture information for `WFDiag`.
//!
//! Collection is synchronous at the lowest layer so command-line and test
//! callers can use it directly. [`SystemRuntime`] owns a dedicated worker for
//! native UI shells that must keep Windows registry and token queries off the
//! dispatcher thread.

#![deny(unsafe_code)]

mod architecture;
mod runtime;
mod system_info;

use serde::{Deserialize, Serialize};
use std::fmt;

pub use architecture::{
    ArchitectureInfo, ArchitectureSnapshot, ProcessorArchitecture, get_architecture_info,
    get_architecture_json, get_architecture_snapshot,
};
pub use runtime::{
    NativeSystemProvider, SystemCompleted, SystemPayload, SystemProvider, SystemRequest,
    SystemRequestKind, SystemRuntime,
};
pub use system_info::{SystemInfo, get_system_info};

/// Collection/runtime error independent of Tauri, Reactor, or another UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum SystemError {
    Collection(String),
    Serialization(String),
    Spawn(String),
    Disconnected,
    WorkerPanicked,
}

impl fmt::Display for SystemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Collection(reason) => write!(formatter, "system information failed: {reason}"),
            Self::Serialization(reason) => {
                write!(
                    formatter,
                    "system information serialization failed: {reason}"
                )
            }
            Self::Spawn(reason) => write!(formatter, "failed to start system worker: {reason}"),
            Self::Disconnected => formatter.write_str("system worker is disconnected"),
            Self::WorkerPanicked => formatter.write_str("system worker panicked"),
        }
    }
}

impl std::error::Error for SystemError {}
