//! Routing the Win32 lifecycle, instance, shortcut and tray signals.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::native_msg::NativeMsg;
use crate::platform::{instance, window};
use windows_reactor::*;

impl WfdiagShell {
    /// One native-window or single-instance signal.
    pub(crate) fn route_native(&mut self, message: NativeMsg, context: &ComponentContext<Self>) {
        match message {
            NativeMsg::InstanceWaitCancelled => self.shell.instance_wait = None,
            NativeMsg::WindowHookRetryReady => {
                self.shell.window_hook_retry_task = None;
                self.ensure_window_hook(context);
            }
            NativeMsg::WindowHookRetryRejected => {
                self.shell.window_hook_retry_task = None;
                self.shell.status = "Native window integration retry was interrupted; the next UI action will retry"
                        .to_string();
            }
            NativeMsg::InstanceActivated => {
                // Another launch asked this instance to the foreground.
                instance::activate_main_window();
                self.arm_instance_watch(context, self.shell.window_lifecycle_revision);
            }
            NativeMsg::WindowLifecycleChanged(observed) => {
                // Coalesce rapid deactivate/reactivate or hide/show pairs so
                // a queued intermediate snapshot cannot cause a visible
                // pause/resume flicker after the window is already usable.
                let current = window::lifecycle_snapshot();
                let snapshot = if current.revision == observed.revision {
                    observed
                } else {
                    current
                };
                self.arm_instance_watch(context, snapshot.revision);
                self.apply_window_lifecycle(snapshot, context);
                if snapshot.focused && self.palette.open {
                    // Re-activation (Alt+Tab, AppActivate, or an instance
                    // handoff) must preserve the palette's editing target.
                    // This is lifecycle-driven and adds no idle polling.
                    let _ = self.palette.query_reference.request_focus();
                }
            }
            NativeMsg::GlobalShortcut(shortcut) => {
                self.arm_instance_watch(context, self.shell.window_lifecycle_revision);
                self.handle_global_shortcut(shortcut, context);
            }
            NativeMsg::TrayCommand(command) => {
                self.arm_instance_watch(context, self.shell.window_lifecycle_revision);
                self.handle_tray_command(command, context);
            }
        }
    }
}
