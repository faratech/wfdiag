//! Native Windows telemetry shared by the Tauri and Reactor UI shells.
//!
//! The existing, battle-tested collectors remain in their original source
//! files during the UI migration. This crate is their framework-neutral Cargo
//! boundary: it deliberately has no dependency on Tauri, `WebView2`, or Reactor.

#![cfg(windows)]

#[path = "../../../src-tauri/src/adapter_monitor.rs"]
mod adapter_monitor;
#[path = "../../../src-tauri/src/native_monitor.rs"]
mod monitor;
#[allow(dead_code)]
#[path = "../../../src-tauri/src/security.rs"]
mod security;
#[path = "../../../src-tauri/src/wmi_native.rs"]
mod wmi_native;

mod runtime;

pub use monitor::*;
pub use runtime::{NativeMonitorRuntime, UiBusMonitorEmitter};
