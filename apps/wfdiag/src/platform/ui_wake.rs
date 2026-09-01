//! Process-wide, UI-framework-neutral notification hook for Reactor workers.
//!
//! Worker modules are also built by portable unit-test targets, so they must
//! not call Win32 directly. The Windows executable installs one callback that
//! posts a coalesced native window message; tests leave the hook uninstalled
//! and event delivery remains a no-op beyond the channel send itself.

use std::sync::{Arc, OnceLock};

type WakeCallback = Arc<dyn Fn() + Send + Sync + 'static>;

static UI_WAKE: OnceLock<WakeCallback> = OnceLock::new();

/// Install the process-wide native wake callback.
///
/// Reactor owns one application component for the process lifetime, so the
/// callback is intentionally single-assignment.
pub fn install(callback: impl Fn() + Send + Sync + 'static) -> Result<(), &'static str> {
    UI_WAKE
        .set(Arc::new(callback))
        .map_err(|_| "the Reactor UI wake callback is already installed")
}

/// Notify the UI that at least one worker channel can be drained.
pub fn notify() {
    if let Some(callback) = UI_WAKE.get() {
        callback();
    }
}
