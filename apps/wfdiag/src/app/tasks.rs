//! The background tasks the shell still owns.
//!
//! Every worker wait, every reply poll and every request-id drain moved into
//! `wfdiag-app`. What is left is genuinely UI-side: two dismiss/focus timers,
//! the degraded tray/activation poll (#207), the window-hook backoff, one
//! external launch, and the two file writes that follow an export the user
//! already chose a destination for.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::{PALETTE_FOCUS_DELAY, PALETTE_RESTORE_DELAY, WINDOW_COMMAND_POLL};
use crate::app::message::Message;
use crate::app::native_msg::NativeMsg;
use crate::dialogs::about::state::AboutMsg;
use crate::dialogs::export::msg::ExportMsg;
use crate::dialogs::palette::msg::PaletteFocusAction;
use crate::dialogs::palette::msg::PaletteMsg;
use crate::dialogs::update_notice::state::UpdateNoticeMsg;
use crate::platform::external::launch_external_action;
use crate::platform::save_picker::{ValidatedExportPath, ValidatedSupportPackagePaths};
use crate::platform::{instance, save_picker, window};
use std::time::{Duration, Instant};
use wfdiag_native_export::SupportPackagePayload;
use wfdiag_native_update::UpdateInfo;
use wfdiag_native_update::policy::AboutExternalAction;
use windows_reactor::*;

pub(crate) fn spawn_palette_focus_delay(
    context: &ComponentContext<WfdiagShell>,
    epoch: u64,
    action: PaletteFocusAction,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            let delay = match action {
                PaletteFocusAction::FocusQuery => PALETTE_FOCUS_DELAY,
                // ContentDialog::Hide completes asynchronously. Restoring a
                // XAML element while the popup is still unwinding succeeds
                // transiently, then WinUI moves focus to its InputSite.
                PaletteFocusAction::RestorePrevious => PALETTE_RESTORE_DELAY,
            };
            let deadline = Instant::now() + delay;
            loop {
                if cancellation.is_cancelled() {
                    return Message::Palette(PaletteMsg::FocusCancelled { epoch });
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::Palette(PaletteMsg::FocusReady { epoch, action });
                }
                std::thread::sleep(remaining.min(Duration::from_millis(25)));
            }
        },
        Message::Palette(PaletteMsg::FocusRejected { epoch }),
    )
}

pub(crate) fn spawn_window_hook_retry(
    context: &ComponentContext<WfdiagShell>,
    delay: Duration,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            let deadline = Instant::now() + delay;
            loop {
                if cancellation.is_cancelled() {
                    return Message::Native(NativeMsg::WindowHookRetryRejected);
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::Native(NativeMsg::WindowHookRetryReady);
                }
                std::thread::sleep(remaining.min(Duration::from_millis(100)));
            }
        },
        Message::Native(NativeMsg::WindowHookRetryRejected),
    )
}

/// The degraded tray/activation poll (#207).
///
/// Only armed when Windows refuses the event-driven registration; the healthy
/// path has no polling task at all.
pub(crate) fn spawn_instance_watch(
    context: &ComponentContext<WfdiagShell>,
    lifecycle_revision: u64,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::Native(NativeMsg::InstanceWaitCancelled);
            }
            if let Some(snapshot) = window::lifecycle_snapshot_if_changed(lifecycle_revision) {
                return Message::Native(NativeMsg::WindowLifecycleChanged(snapshot));
            }
            if instance::activation_requested() {
                return Message::Native(NativeMsg::InstanceActivated);
            }
            if let Some(shortcut) = window::take_global_shortcut() {
                return Message::Native(NativeMsg::GlobalShortcut(shortcut));
            }
            let command = window::take_tray_command();
            if command != window::TRAY_COMMAND_NONE {
                return Message::Native(NativeMsg::TrayCommand(command));
            }
            std::thread::sleep(WINDOW_COMMAND_POLL);
        },
        Message::Native(NativeMsg::InstanceWaitCancelled),
    )
}

pub(crate) fn spawn_update_notice_timer(
    context: &ComponentContext<WfdiagShell>,
    epoch: u64,
    timer_generation: u64,
    duration: Duration,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            let deadline = Instant::now() + duration;
            loop {
                if cancellation.is_cancelled() {
                    return Message::UpdateNotice(UpdateNoticeMsg::TimerCancelled {
                        epoch,
                        timer_generation,
                    });
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::UpdateNotice(UpdateNoticeMsg::Expired {
                        epoch,
                        timer_generation,
                    });
                }
                std::thread::sleep(remaining.min(Duration::from_millis(100)));
            }
        },
        Message::UpdateNotice(UpdateNoticeMsg::TimerRejected {
            epoch,
            timer_generation,
        }),
    )
}

pub(crate) fn spawn_about_external_action(
    context: &ComponentContext<WfdiagShell>,
    epoch: u64,
    action: AboutExternalAction,
    update: Option<UpdateInfo>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |_| {
            Message::About(AboutMsg::ExternalFinished {
                epoch,
                result: launch_external_action(action, update.as_ref()),
            })
        },
        Message::About(AboutMsg::ExternalRejected { epoch }),
    )
}

pub(crate) fn spawn_export_file_write(
    context: &ComponentContext<WfdiagShell>,
    epoch: u64,
    path: ValidatedExportPath,
    content: String,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            let result = if cancellation.is_cancelled() {
                Err("The export was cancelled".to_string())
            } else {
                // Match Tauri's `save_results_to_file`: the picker preflights
                // the destination, then the write path is policy-validated
                // again immediately before filesystem mutation.
                save_picker::revalidate_export_path(&path)
                    .map_err(|error| error.to_string())
                    .and_then(|path| {
                        std::fs::write(path.as_path(), content)
                            .map(|()| path.into_path())
                            .map_err(|error| error.to_string())
                    })
            };
            Message::Export(ExportMsg::FileSaved {
                epoch,
                result: Box::new(result),
            })
        },
        Message::Export(ExportMsg::FileSaved {
            epoch,
            result: Box::new(Err(
                "The Reactor background queue rejected the export write".to_string(),
            )),
        }),
    )
}

pub(crate) fn spawn_support_package_write(
    context: &ComponentContext<WfdiagShell>,
    epoch: u64,
    paths: ValidatedSupportPackagePaths,
    payload: SupportPackagePayload,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            let write = || -> Result<ValidatedSupportPackagePaths, String> {
                if cancellation.is_cancelled() {
                    return Err("Support-package generation was cancelled".to_string());
                }
                let validated = save_picker::revalidate_support_package_paths(&paths)
                    .map_err(|error| error.to_string())?;
                std::fs::write(&validated.json, payload.json.as_bytes()).map_err(|error| {
                    format!("could not write {}: {error}", validated.json.display())
                })?;
                if cancellation.is_cancelled() {
                    return Err("Support-package generation was cancelled".to_string());
                }
                let validated = save_picker::revalidate_support_package_paths(&paths)
                    .map_err(|error| error.to_string())?;
                std::fs::write(&validated.text, payload.text.as_bytes()).map_err(|error| {
                    format!("could not write {}: {error}", validated.text.display())
                })?;
                if cancellation.is_cancelled() {
                    return Err("Support-package generation was cancelled".to_string());
                }
                let validated = save_picker::revalidate_support_package_paths(&paths)
                    .map_err(|error| error.to_string())?;
                std::fs::write(&validated.html, payload.html.as_bytes()).map_err(|error| {
                    format!("could not write {}: {error}", validated.html.display())
                })?;
                Ok(validated)
            };
            Message::Export(ExportMsg::SupportPackageSaved {
                epoch,
                result: Box::new(write()),
            })
        },
        Message::Export(ExportMsg::SupportPackageSaved {
            epoch,
            result: Box::new(Err(
                "The Reactor background queue rejected the support-package write".to_string(),
            )),
        }),
    )
}
