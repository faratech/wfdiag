//! Background-task spawners and worker-channel drains.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::{
    ELEVATED_RELAUNCH_FLAG, PALETTE_FOCUS_DELAY, PALETTE_RESTORE_DELAY,
    PROVIDER_MODEL_REFRESH_DELAY, SCAN_FINALIZATION_DELAY, WINDOW_COMMAND_POLL,
};
use crate::app::message::{HistoryAckKind, Message, PaletteFocusAction};
use crate::app::policy::reactor_update_throttle;
use crate::platform::external::launch_external_action;
use crate::platform::save_picker::{ValidatedExportPath, ValidatedSupportPackagePaths};
use crate::platform::{instance, save_picker, window};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
use wfdiag_native_ai_chat::ChatWorkerEvent;
use wfdiag_native_ai_provider::{ProviderPreferenceStatusReply, ProviderStatusReply};
use wfdiag_native_ai_report::ReportGeneration;
use wfdiag_native_diagnostics::SharedScanEvidence;
use wfdiag_native_export::{ExportCompleted, SupportPackagePayload};
use wfdiag_native_history::{ComparisonResult, HistoryReply};
use wfdiag_native_issues::projection::{PendingIssueDetection, prepare_issue_detection};
use wfdiag_native_issues::{IssueDetectionCompleted, Timestamp as IssueTimestamp};
use wfdiag_native_system::SystemCompleted;
use wfdiag_native_update::policy::{
    AboutExternalAction, START_DELAY, UpdateThrottle, unix_time_millis,
};
use wfdiag_native_update::{UpdateInfo, UpdateOutcome, UpdateReply};
use windows_reactor::*;

/// Bound one UI publication so a bursty stream cannot monopolize the WinUI
/// dispatcher. Hitting the limit schedules another coalesced native wake.
pub(crate) const NATIVE_EVENT_DRAIN_LIMIT: usize = 512;

/// Drain chat worker events into ONE batched message per wake.
///
/// Chat deltas arrive at token rate, and every published message triggers a
/// full view rebuild. Batching per drain collapses hundreds of rebuilds during
/// a streamed answer into one per wake without changing event order or
/// semantics (the handler applies events in the worker's emission order).
pub(crate) fn drain_chat_events(
    receiver: &Arc<Mutex<std::sync::mpsc::Receiver<ChatWorkerEvent>>>,
    messages: &mut Vec<Message>,
) -> bool {
    let receiver = match receiver.lock() {
        Ok(receiver) => receiver,
        Err(_) => {
            messages.push(Message::ChatWorkerStopped);
            return false;
        }
    };
    let mut batch: Vec<ChatWorkerEvent> = Vec::new();
    let saturated;
    loop {
        match receiver.try_recv() {
            Ok(event) => batch.push(event),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                saturated = false;
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                if batch.is_empty() {
                    messages.push(Message::ChatWorkerStopped);
                }
                saturated = false;
                break;
            }
        }
        if batch.len() >= NATIVE_EVENT_DRAIN_LIMIT {
            saturated = true;
            break;
        }
    }
    if !batch.is_empty() {
        messages.push(Message::ChatWorkerEventsBatch(batch));
    }
    saturated
}

pub(crate) fn drain_native_receiver<T>(
    receiver: &Arc<Mutex<std::sync::mpsc::Receiver<T>>>,
    messages: &mut Vec<Message>,
    mut map: impl FnMut(T) -> Message,
    stopped: Message,
) -> bool {
    let receiver = match receiver.lock() {
        Ok(receiver) => receiver,
        Err(_) => {
            messages.push(stopped);
            return false;
        }
    };

    for _ in 0..NATIVE_EVENT_DRAIN_LIMIT {
        match receiver.try_recv() {
            Ok(event) => messages.push(map(event)),
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                messages.push(stopped);
                return false;
            }
        }
    }
    true
}

pub(crate) fn spawn_ai_status_wait(
    context: &ComponentContext<WfdiagShell>,
    request_id: u64,
    mut reply: ProviderStatusReply,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::AiStatusCancelled { request_id };
            }
            match reply.try_recv() {
                Ok(status) => {
                    return Message::AiStatusFinished {
                        request_id,
                        result: Ok(Box::new(status)),
                    };
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::AiStatusFinished {
                        request_id,
                        result: Err(
                            "Native AI provider worker stopped before replying".to_string(),
                        ),
                    };
                }
            }
        },
        Message::AiStatusRejected { request_id },
    )
}

pub(crate) fn spawn_ai_preference_status_wait(
    context: &ComponentContext<WfdiagShell>,
    request_id: u64,
    mut reply: ProviderPreferenceStatusReply,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::AiStatusCancelled { request_id };
            }
            match reply.try_recv() {
                Ok(result) => {
                    return Message::AiStatusFinished {
                        request_id,
                        result: result.map(Box::new),
                    };
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::AiStatusFinished {
                        request_id,
                        result: Err("Native AI provider worker stopped before applying Settings"
                            .to_string()),
                    };
                }
            }
        },
        Message::AiStatusRejected { request_id },
    )
}

pub(crate) fn spawn_diagnostic_finalization_delay(
    context: &ComponentContext<WfdiagShell>,
    session_id: String,
) -> ComponentTask {
    let rejection_session_id = session_id.clone();
    context.spawn_background_with_rejection(
        move |cancellation| {
            let deadline = Instant::now() + SCAN_FINALIZATION_DELAY;
            loop {
                if cancellation.is_cancelled() {
                    return Message::DiagnosticFinalizationCancelled { session_id };
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::DiagnosticFinalizationElapsed { session_id };
                }
                std::thread::sleep(remaining.min(Duration::from_millis(50)));
            }
        },
        Message::DiagnosticFinalizationRejected {
            session_id: rejection_session_id,
        },
    )
}

pub(crate) fn spawn_history_ack_wait(
    context: &ComponentContext<WfdiagShell>,
    kind: HistoryAckKind,
    mut reply: HistoryReply<()>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::HistoryAckFinished {
                    kind,
                    result: Err("The Reactor background queue rejected the request".to_string()),
                };
            }
            match reply.try_recv() {
                Ok(result) => {
                    return Message::HistoryAckFinished {
                        kind,
                        result: result.map_err(|error| error.to_string()),
                    };
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::HistoryAckFinished {
                        kind,
                        result: Err("Native history worker stopped".to_string()),
                    };
                }
            }
        },
        Message::HistoryAckFinished {
            kind,
            result: Err("The Reactor background queue rejected the request".to_string()),
        },
    )
}

pub(crate) fn spawn_history_save_wait(
    context: &ComponentContext<WfdiagShell>,
    session_id: String,
    mut reply: HistoryReply<()>,
) -> ComponentTask {
    let rejection_session_id = session_id.clone();
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::DiagnosticHistorySaveWaitCancelled { session_id };
            }
            match reply.try_recv() {
                Ok(result) => {
                    return Message::DiagnosticHistorySaveFinished { session_id, result };
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::DiagnosticHistorySaveFinished {
                        session_id,
                        result: Err(
                            "native history worker stopped before acknowledging the scan"
                                .to_string(),
                        ),
                    };
                }
            }
        },
        Message::DiagnosticHistorySaveRejected {
            session_id: rejection_session_id,
        },
    )
}

pub(crate) fn spawn_provider_model_refresh_delay(
    context: &ComponentContext<WfdiagShell>,
    dialog_epoch: u64,
    refresh_revision: u64,
    setup_index: usize,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            let deadline = Instant::now() + PROVIDER_MODEL_REFRESH_DELAY;
            loop {
                if cancellation.is_cancelled() {
                    return Message::ProviderModelsRefreshCancelled { refresh_revision };
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::ProviderModelsRefreshDue {
                        dialog_epoch,
                        refresh_revision,
                        setup_index,
                    };
                }
                std::thread::sleep(remaining.min(Duration::from_millis(50)));
            }
        },
        Message::ProviderModelsRefreshRejected { refresh_revision },
    )
}

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
                    return Message::PaletteFocusCancelled { epoch };
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::PaletteFocusReady { epoch, action };
                }
                std::thread::sleep(remaining.min(Duration::from_millis(25)));
            }
        },
        Message::PaletteFocusRejected { epoch },
    )
}

pub(crate) fn spawn_system_wait(
    context: &ComponentContext<WfdiagShell>,
    receiver: Arc<Mutex<mpsc::Receiver<SystemCompleted>>>,
) -> ComponentTask {
    // Invariant: this wait task is the ONLY consumer of `receiver`. The mutex
    // exists so the runtime can hand the same receiver back for re-arming, not
    // for concurrent reads — `recv_timeout` parks under the guard for up to
    // 100 ms, which would stall a second consumer for that long.
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::SystemWaitCancelled;
            }

            let received = match receiver.lock() {
                Ok(receiver) => receiver.recv_timeout(Duration::from_millis(100)),
                Err(_) => return Message::SystemWorkerStopped,
            };
            match received {
                Ok(completed) => {
                    return Message::SystemRuntimeCompleted(Box::new(completed));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Message::SystemWorkerStopped;
                }
            }
        },
        Message::SystemWaitRejected,
    )
}

pub(crate) fn spawn_issue_wait(
    context: &ComponentContext<WfdiagShell>,
    receiver: Arc<Mutex<mpsc::Receiver<IssueDetectionCompleted>>>,
) -> ComponentTask {
    // Single-consumer invariant: see `spawn_system_wait`.
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::IssueWaitCancelled;
            }

            let received = match receiver.lock() {
                Ok(receiver) => receiver.recv_timeout(Duration::from_millis(100)),
                Err(_) => return Message::IssueWorkerStopped,
            };
            match received {
                Ok(completed) => {
                    return Message::IssueRuntimeCompleted(Box::new(completed));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Message::IssueWorkerStopped;
                }
            }
        },
        Message::IssueWaitRejected,
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
                    return Message::WindowHookRetryRejected;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::WindowHookRetryReady;
                }
                std::thread::sleep(remaining.min(Duration::from_millis(100)));
            }
        },
        Message::WindowHookRetryRejected,
    )
}

pub(crate) fn spawn_instance_watch(
    context: &ComponentContext<WfdiagShell>,
    lifecycle_revision: u64,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::InstanceWaitCancelled;
            }
            if let Some(snapshot) = window::lifecycle_snapshot_if_changed(lifecycle_revision) {
                return Message::WindowLifecycleChanged(snapshot);
            }
            if instance::activation_requested() {
                return Message::InstanceActivated;
            }
            if let Some(shortcut) = window::take_global_shortcut() {
                return Message::GlobalShortcut(shortcut);
            }
            let command = window::take_tray_command();
            if command != window::TRAY_COMMAND_NONE {
                return Message::TrayCommand(command);
            }
            std::thread::sleep(WINDOW_COMMAND_POLL);
        },
        Message::InstanceWaitCancelled,
    )
}

/// `relaunch_self_elevated` blocks on the UAC prompt and COM; keep it off
/// the WinUI thread.
pub(crate) fn spawn_relaunch_as_admin(context: &ComponentContext<WfdiagShell>) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |_cancellation| {
            Message::RestartAsAdminFinished(
                wfdiag_native_remediation::elevation::relaunch_self_elevated_with_flag(
                    ELEVATED_RELAUNCH_FLAG,
                ),
            )
        },
        Message::RestartAsAdminFinished(Err(
            "The Reactor background queue rejected the elevation hand-off".to_string(),
        )),
    )
}

/// Resolve the implicit "latest different scan" report baseline without
/// touching DPAPI or the filesystem on the WinUI thread. History failures are
/// intentionally soft here: the shipping report path generates without a
/// comparison whenever its automatic baseline cannot be read.
pub(crate) fn spawn_report_generation_preparation(
    context: &ComponentContext<WfdiagShell>,
    request_id: u64,
    mut generation: ReportGeneration,
    mut reply: HistoryReply<Option<ComparisonResult>>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::ReportGenerationPreparationCancelled { request_id };
            }
            match reply.try_recv() {
                Ok(Ok(comparison)) => {
                    generation.comparison = comparison;
                    return Message::ReportGenerationPrepared {
                        request_id,
                        generation: Box::new(generation),
                    };
                }
                // An implicit history baseline is optional. A storage error or
                // a stopped history worker must not disable AI reporting.
                Ok(Err(_)) | Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::ReportGenerationPrepared {
                        request_id,
                        generation: Box::new(generation),
                    };
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        },
        Message::ReportGenerationPreparationRejected { request_id },
    )
}

pub(crate) fn spawn_export_wait(
    context: &ComponentContext<WfdiagShell>,
    receiver: Arc<Mutex<mpsc::Receiver<ExportCompleted>>>,
) -> ComponentTask {
    // Single-consumer invariant: see `spawn_system_wait`.
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::ExportWaitCancelled;
            }

            let received = match receiver.lock() {
                Ok(receiver) => receiver.recv_timeout(Duration::from_millis(100)),
                Err(_) => return Message::ExportWorkerStopped,
            };
            match received {
                Ok(completed) => {
                    return Message::ExportRuntimeCompleted(Box::new(completed));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Message::ExportWorkerStopped;
                }
            }
        },
        Message::ExportWaitRejected,
    )
}

pub(crate) fn spawn_export_file_write(
    context: &ComponentContext<WfdiagShell>,
    request_id: u64,
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
            Message::ExportFileSaved {
                request_id,
                result: Box::new(result),
            }
        },
        Message::ExportWaitRejected,
    )
}

pub(crate) fn spawn_support_package_write(
    context: &ComponentContext<WfdiagShell>,
    request_id: u64,
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
            Message::SupportPackageSaved {
                request_id,
                result: Box::new(write()),
            }
        },
        Message::ExportWaitRejected,
    )
}

pub(crate) fn spawn_issue_request_preparation(
    context: &ComponentContext<WfdiagShell>,
    pending: PendingIssueDetection,
    results: SharedScanEvidence,
) -> ComponentTask {
    let rejected = pending.clone();
    context.spawn_background_with_rejection(
        move |cancellation| {
            if cancellation.is_cancelled() {
                return Message::IssueRequestPreparationCancelled(pending);
            }

            // These two OS-dependent inputs stay off the Reactor UI thread;
            // diagnostic evidence itself is shared without another deep copy.
            let now = IssueTimestamp::now();
            let temp_file_count = std::fs::read_dir(std::env::temp_dir())
                .ok()
                .map(|entries| entries.count());
            let prepared = prepare_issue_detection(
                pending.request_id,
                pending.committed_epoch,
                pending.session_id.clone(),
                Arc::clone(&results),
                now,
                temp_file_count,
            );
            if cancellation.is_cancelled() {
                Message::IssueRequestPreparationCancelled(prepared.pending)
            } else {
                Message::IssueRequestPrepared(Box::new(prepared))
            }
        },
        Message::IssueRequestPreparationRejected(rejected),
    )
}

pub(crate) fn spawn_update_delay(context: &ComponentContext<WfdiagShell>) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| {
            if cancellation.is_cancelled() {
                return Message::UpdateDelayCancelled;
            }
            let throttle = reactor_update_throttle();
            if throttle
                .as_ref()
                .is_some_and(|throttle| !throttle.should_check_at(unix_time_millis()))
            {
                return Message::UpdateStartupSkipped;
            }

            let deadline = Instant::now() + START_DELAY;
            loop {
                if cancellation.is_cancelled() {
                    return Message::UpdateDelayCancelled;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::UpdateStartupDue { throttle };
                }
                std::thread::sleep(remaining.min(Duration::from_millis(100)));
            }
        },
        Message::UpdateDelayRejected,
    )
}

pub(crate) fn spawn_update_wait(
    context: &ComponentContext<WfdiagShell>,
    mut reply: UpdateReply,
    throttle: Option<UpdateThrottle>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |cancellation| loop {
            if cancellation.is_cancelled() {
                return Message::UpdateCheckCancelled;
            }
            match reply.try_recv() {
                Ok(outcome) => {
                    // Match the Store hook: a completed backend check consumes
                    // the daily attempt even when its deliberately silent
                    // result offers nothing. Persistence failure remains
                    // fail-open.
                    if let Some(throttle) = throttle.as_ref() {
                        let _ = throttle.record_at(unix_time_millis());
                    }
                    // A check that could not complete (offline, rate limited,
                    // unparseable) is reported as an error rather than folded
                    // into "no update"; the UI still shows nothing for it.
                    return Message::UpdateCheckFinished(match outcome {
                        UpdateOutcome::Failed(failure) => Err(failure.to_string()),
                        completed => Ok(completed.into_available()),
                    });
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Message::UpdateCheckFinished(Err(
                        "Native update worker stopped before replying".to_string(),
                    ));
                }
            }
        },
        Message::UpdateCheckRejected,
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
                    return Message::UpdateNoticeTimerCancelled {
                        epoch,
                        timer_generation,
                    };
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Message::UpdateNoticeExpired {
                        epoch,
                        timer_generation,
                    };
                }
                std::thread::sleep(remaining.min(Duration::from_millis(100)));
            }
        },
        Message::UpdateNoticeTimerRejected {
            epoch,
            timer_generation,
        },
    )
}

pub(crate) fn spawn_about_external_action(
    context: &ComponentContext<WfdiagShell>,
    epoch: u64,
    action: AboutExternalAction,
    update: Option<UpdateInfo>,
) -> ComponentTask {
    context.spawn_background_with_rejection(
        move |_| Message::AboutExternalFinished {
            epoch,
            result: launch_external_action(action, update.as_ref()),
        },
        Message::AboutExternalRejected { epoch },
    )
}
