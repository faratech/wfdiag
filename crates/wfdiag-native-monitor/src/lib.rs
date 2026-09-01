//! Native Windows telemetry shared by the Tauri and Reactor UI shells.
//!
//! This crate is the framework-neutral Cargo boundary for the shipping
//! collectors: it deliberately has no dependency on Tauri, `WebView2`, or
//! Reactor.
//!
//! Because the whole crate is `cfg(windows)`, the portable application
//! facade (`wfdiag-app`) mirrors `ProcessQuery`, `ProcessPage`, `ProcessRow`,
//! and `NetworkConnection` in its `ports` module and converts at the Windows
//! boundary. A field added to any of those types here must be mirrored
//! there (the facade's `ports/native.rs` conversions will fail to compile
//! otherwise, which is the intended safety net).

#![cfg(windows)]

// FFI-heavy modules keep unsafe scoped to themselves; the workspace denies
// `unsafe_code` everywhere else.
#[allow(unsafe_code)]
pub mod adapter_monitor;
#[allow(unsafe_code)]
pub mod monitor;

mod runtime;

pub use monitor::*;
pub use runtime::{NativeMonitorRuntime, ProcessQueryOutcome, UiBusMonitorEmitter};
