//! Win32 lifecycle, single-instance, global-shortcut and tray signals.
//!
//! Everything here originates outside Reactor: the isolated window subclass,
//! the instance-activation wait, and the notification-area icon. They reach
//! the component as one coalesced wake and are drained into these messages.

#![deny(unsafe_code)]

use crate::platform::window;

/// One native-window or single-instance signal.
#[derive(Clone)]
pub(crate) enum NativeMsg {
    /// The degraded-path instance/lifecycle poll was cancelled.
    InstanceWaitCancelled,
    /// The bounded window-hook retry elapsed.
    WindowHookRetryReady,
    /// The bounded window-hook retry could not enter the background queue.
    WindowHookRetryRejected,
    /// Another launch asked this instance to the foreground.
    InstanceActivated,
    WindowLifecycleChanged(window::WindowLifecycleSnapshot),
    GlobalShortcut(window::GlobalShortcutEvent),
    TrayCommand(u8),
}
