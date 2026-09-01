//! Native Windows telemetry shared by the Tauri and Reactor UI shells.
//!
//! This crate is the framework-neutral Cargo boundary for the shipping
//! collectors: it deliberately has no dependency on Tauri, `WebView2`, or
//! Reactor.

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
