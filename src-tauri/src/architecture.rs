//! Compatibility re-exports for the canonical native system crate.
//!
//! Keeping this module path stable avoids churn in diagnostic collectors while
//! Tauri and native UI shells share the same architecture implementation.

pub use wfdiag_native_system::{ArchitectureInfo, ProcessorArchitecture, get_architecture_info};
