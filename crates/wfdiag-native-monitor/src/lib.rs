//! Native Windows telemetry shared by the Tauri and Reactor UI shells.
//!
//! The existing, battle-tested collectors remain in their original source
//! files during the UI migration. This crate is their framework-neutral Cargo
//! boundary: it deliberately has no dependency on Tauri, `WebView2`, or Reactor.

#![cfg(windows)]

// FFI-heavy modules keep unsafe scoped to themselves; the workspace denies
// `unsafe_code` everywhere else.
#[allow(unsafe_code)]
#[path = "../../../src-tauri/src/adapter_monitor.rs"]
mod adapter_monitor;
#[allow(unsafe_code)]
#[path = "../../../src-tauri/src/native_monitor.rs"]
mod monitor;

// `native_monitor.rs` still says `crate::wmi_native::…`; the implementation
// compiles once in `wfdiag-native-core`.
#[allow(unused_imports)]
mod wmi_native {
    pub use wfdiag_native_core::wmi::*;
}

mod runtime;

pub use monitor::*;
pub use runtime::{NativeMonitorRuntime, ProcessQueryOutcome, UiBusMonitorEmitter};
