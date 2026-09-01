//! Window lifecycle, navigation, palette, and native event pumping.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::{PROCESS_FILTER_DEBOUNCE, PROCESS_PAGE_SIZE};
use crate::app::message::{Message, PaletteFocusAction};
use crate::app::policy::{
    MonitoringLifecycleAction, effective_window_theme, global_shortcut_is_allowed,
    monitoring_lifecycle_action, next_process_request_id, take_matching_system_request,
    window_hook_retry_delay, window_is_usable, window_theme_setting,
};
use crate::app::state::{Page, PageTransition};
use crate::app::tasks::{
    drain_chat_events, drain_native_receiver, spawn_instance_watch, spawn_palette_focus_delay,
    spawn_system_wait, spawn_window_hook_retry,
};
use crate::dialogs::palette::{
    PALETTE_APP_TEMPLATES, PALETTE_NAVIGATION_TEMPLATES, PALETTE_REPORT_TEMPLATES,
    PALETTE_SCAN_TEMPLATES, PALETTE_STOP_SCAN_TEMPLATE, PaletteCommandSpec,
    diagnostic_palette_icon, palette_visible_matches,
};
use crate::platform::{focus, instance, ui_wake, window};
use crate::widgets::icons::FaIcon;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_monitor::{
    MonitorProfile, NativeMonitorRuntime, ProcessQuery, ProcessSortDirection, ProcessSortKey,
};
use wfdiag_native_system::{SystemCompleted, SystemPayload, SystemRequestKind};
use wfdiag_ui_core::{ChatEvent, UiEvent, UiWakeHandler};
use windows_reactor::*;

impl WfdiagShell {
    /// Drain the wake-driven native producers from the UI thread.
    /// Worker threads only enqueue typed data and post one coalesced WM_APP
    /// signal, so idle pages no longer retain a Reactor poll task per channel.
    /// The system/issue/export completion channels are the exception: each is
    /// owned by a dedicated wait task (see `spawn_system_wait` and siblings)
    /// that parks on the receiver instead.
    pub(crate) fn drain_native_messages(&self) -> Vec<Message> {
        let mut messages = Vec::new();
        let mut saturated = false;

        if let Some(snapshot) =
            window::lifecycle_snapshot_if_changed(self.window_lifecycle_revision)
        {
            messages.push(Message::WindowLifecycleChanged(snapshot));
        }
        if instance::activation_requested() {
            messages.push(Message::InstanceActivated);
        }
        while let Some(shortcut) = window::take_global_shortcut() {
            messages.push(Message::GlobalShortcut(shortcut));
        }
        let tray_command = window::take_tray_command();
        if tray_command != window::TRAY_COMMAND_NONE {
            messages.push(Message::TrayCommand(tray_command));
        }

        if let Some(receiver) = self.backend_receiver.as_ref() {
            let events = receiver.drain();
            let terminated = receiver.is_terminated();
            if !events.is_empty() || terminated {
                messages.push(Message::BackendBatch { events, terminated });
            }
        }
        if let Some(receiver) = self.diagnostic_receiver.as_ref() {
            let events = receiver.drain();
            let terminated = receiver.is_terminated();
            if !events.is_empty() || terminated {
                messages.push(Message::DiagnosticBatch { events, terminated });
            }
        }

        if let Some(receiver) = self.settings_receiver.as_ref() {
            saturated |= drain_native_receiver(
                receiver,
                &mut messages,
                |event| Message::SettingsRuntimeEvent(Box::new(event)),
                Message::SettingsWorkerStopped,
            );
        }

        if let Some(receiver) = self.provider_setup_receiver.as_ref() {
            saturated |= drain_native_receiver(
                receiver,
                &mut messages,
                |event| Message::ProviderSetupWorkerEventReceived(Box::new(event)),
                Message::ProviderSetupWorkerStopped,
            );
        }
        if let Some(receiver) = self.subscription_auth_receiver.as_ref() {
            saturated |= drain_native_receiver(
                receiver,
                &mut messages,
                |event| Message::SubscriptionAuthWorkerEventReceived(Box::new(event)),
                Message::SubscriptionAuthWorkerStopped,
            );
        }
        if let Some(receiver) = self.subscription_install_receiver.as_ref() {
            saturated |= drain_native_receiver(
                receiver,
                &mut messages,
                |event| Message::SubscriptionInstallWorkerEventReceived(Box::new(event)),
                Message::SubscriptionInstallWorkerStopped,
            );
        }
        if let Some(receiver) = self.chat_receiver.as_ref() {
            saturated |= drain_chat_events(receiver, &mut messages);
        }
        if let Some(receiver) = self.report_receiver.as_ref() {
            saturated |= drain_native_receiver(
                receiver,
                &mut messages,
                |event| Message::ReportWorkerEventReceived(Box::new(event)),
                Message::ReportWorkerStopped,
            );
        }
        if let Some(receiver) = self.analysis_receiver.as_ref() {
            saturated |= drain_native_receiver(
                receiver,
                &mut messages,
                |event| Message::AnalysisWorkerEventReceived(Box::new(event)),
                Message::AnalysisWorkerStopped,
            );
        }
        if let Some(receiver) = self.fix_plan_receiver.as_ref() {
            saturated |= drain_native_receiver(
                receiver,
                &mut messages,
                |event| Message::FixPlanWorkerEventReceived(Box::new(event)),
                Message::FixPlanWorkerStopped,
            );
        }
        if let Some(receiver) = self.action_receiver.as_ref() {
            saturated |= drain_native_receiver(
                receiver,
                &mut messages,
                |event| Message::ActionWorkerEventReceived(Box::new(event)),
                Message::ActionWorkerStopped,
            );
        }
        if let Some(receiver) = self.action_run_receiver.as_ref() {
            saturated |= drain_native_receiver(
                receiver,
                &mut messages,
                |event| Message::ActionRunEventReceived(Box::new(event)),
                Message::ActionRunStreamStopped,
            );
        }

        if saturated {
            ui_wake::notify();
        }
        messages
    }

    /// Re-arm the degraded-path instance/lifecycle watch.
    ///
    /// Only used when the kernel wait registration is unavailable. Dropping a
    /// `ComponentTask` does NOT cancel its closure (windows-reactor keeps the
    /// thread running), so re-arming without cancelling would accumulate live
    /// 50 ms poll threads until the 64-slot background budget starts rejecting
    /// every other spawn in the app.
    pub(crate) fn arm_instance_watch(
        &mut self,
        context: &ComponentContext<Self>,
        lifecycle_revision: u64,
    ) {
        if instance::activation_wake_registered() {
            return;
        }
        if let Some(previous) = self.instance_wait.take() {
            previous.cancel();
        }
        self.instance_wait = Some(spawn_instance_watch(context, lifecycle_revision));
    }

    pub(crate) fn resume_system_wait(&mut self, context: &ComponentContext<Self>) {
        if self.system_info_request_id.is_none() && self.architecture_request_id.is_none() {
            self.system_wait = None;
            return;
        }
        // One consumer per receiver: a live wait already owns the shared
        // `Arc<Mutex<Receiver>>` (see the single-consumer note on
        // `spawn_system_wait`), so re-arming would park a second thread that
        // nothing ever cancels (#210).
        if self.system_wait.is_some() {
            return;
        }
        let Some(receiver) = self.system_receiver.as_ref().map(Arc::clone) else {
            self.system_wait = None;
            return;
        };
        self.system_wait = Some(spawn_system_wait(context, receiver));
    }

    /// Install the tray + close-to-tray hook once the WinUI window exists.
    /// Runs on the UI thread (subclassing requires the owning thread); the
    /// bool guard makes it a cheap no-op afterwards.
    pub(crate) fn ensure_window_hook(&mut self, context: &ComponentContext<Self>) {
        if self.window_hook_installed
            || self.window_hook_retry_task.is_some()
            || self.deterministic_visual
        {
            return;
        }
        let Some(window) = instance::main_window_hwnd() else {
            // `create` runs before windows-reactor materializes its HWND. A
            // one-shot bootstrap reaches this path after the first delay; if
            // window creation is slower, continue with bounded backoff until
            // the exact process-owned window can be discovered.
            self.window_hook_retry_failures = self.window_hook_retry_failures.saturating_add(1);
            let delay = window_hook_retry_delay(self.window_hook_retry_failures);
            self.window_hook_retry_task = Some(spawn_window_hook_retry(context, delay));
            return;
        };
        // The validation switch omits only the notification-area icon. The
        // native wake/lifecycle subclass remains required for event delivery.
        let tray_disabled = std::env::var_os("WFDIAG_NO_TRAY").is_some();
        let installed = if tray_disabled {
            window::install_without_tray(window)
        } else {
            window::install(window, "WindowsForum Diagnostics")
        };
        if let Err(error) = installed {
            if error.core_installed() {
                // Explorer can reject a notification-area icon while the
                // lifecycle/shortcut/wake subclass itself is healthy. Keep
                // event-driven delivery and explicitly disable close-to-tray
                // so the user can never hide a window that has no restore
                // affordance.
                window::set_close_to_tray(false);
                self.status = format!("System tray unavailable · {}", error.message());
                self.window_hook_installed = true;
                self.window_hook_retry_failures = 0;
                ui_wake::notify();
                return;
            }

            self.window_hook_retry_failures = self.window_hook_retry_failures.saturating_add(1);
            let delay = window_hook_retry_delay(self.window_hook_retry_failures);
            self.status = format!(
                "Native window integration is retrying in {} ms · {}",
                delay.as_millis(),
                error.message()
            );
            self.window_hook_retry_task = Some(spawn_window_hook_retry(context, delay));
            return;
        }
        window::set_close_to_tray(!tray_disabled && self.settings_snapshot.close_to_tray);
        self.window_hook_installed = true;
        self.window_hook_retry_failures = 0;
        // Producers may have queued data before the HWND became available.
        ui_wake::notify();
    }

    pub(crate) fn apply_window_lifecycle(
        &mut self,
        snapshot: window::WindowLifecycleSnapshot,
        context: &ComponentContext<Self>,
    ) {
        self.window_lifecycle_revision = snapshot.revision;
        self.window_usable = window_is_usable(snapshot);
        if !matches!(self.page, Page::Monitor | Page::Processes) {
            return;
        }
        match monitoring_lifecycle_action(
            snapshot,
            self.monitoring_paused,
            self.monitoring_paused_by_lifecycle,
        ) {
            MonitoringLifecycleAction::Pause => {
                let accepted = self
                    .native_monitor
                    .as_ref()
                    .is_some_and(|runtime| runtime.pause());
                if accepted {
                    self.monitoring_paused = true;
                    self.monitoring_paused_by_lifecycle = true;
                    self.invalidate_process_page_request();
                    if matches!(self.page, Page::Monitor | Page::Processes) {
                        self.status =
                            "Live monitoring paused while the window is inactive".to_string();
                    }
                }
            }
            MonitoringLifecycleAction::ResumeAndRefresh => {
                let accepted = self
                    .native_monitor
                    .as_ref()
                    .is_some_and(|runtime| runtime.resume());
                if accepted {
                    self.monitoring_paused = false;
                    self.monitoring_paused_by_lifecycle = false;
                    if let Some(runtime) = self.native_monitor.as_ref() {
                        let _ = runtime.refresh();
                    }
                    if self.page == Page::Processes {
                        self.request_process_page(context, false);
                    }
                    if matches!(self.page, Page::Monitor | Page::Processes) {
                        self.status = "Live monitoring resumed and refreshed".to_string();
                    }
                }
            }
            MonitoringLifecycleAction::None => {}
        }
    }

    pub(crate) fn blocking_overlay_open(&self) -> bool {
        self.settings_open
            || self.about_open
            || self.palette_open
            || self.shortcut_help_open
            || self.full_scan_consent.is_some()
            || self.cloud_fallback_consent.is_some()
            || self.cloud_fallback_policy_update.is_some()
            || self.action_review.is_some()
            || self.repair_confirm.is_some()
            || self.subscription_install_prompt.is_some()
            || self.history_clear_confirm
    }

    pub(crate) fn set_palette_visibility(&mut self, open: bool, context: &ComponentContext<Self>) {
        if self.palette_open == open {
            return;
        }
        if let Some(task) = self.palette_focus_task.take() {
            task.cancel();
        }
        if open {
            focus::capture_pre_palette_focus();
        }
        self.palette_open = open;
        window::set_palette_open(open);
        self.palette_query.clear();
        self.palette_active_index = 0;
        self.palette_dialog_epoch = self.palette_dialog_epoch.wrapping_add(1);
        let action = if open {
            PaletteFocusAction::FocusQuery
        } else {
            PaletteFocusAction::RestorePrevious
        };
        self.palette_focus_task = Some(spawn_palette_focus_delay(
            context,
            self.palette_dialog_epoch,
            action,
        ));
    }

    /// Apply a Win32-captured chord only after revalidating the current UI
    /// state on Reactor's component thread. This mirrors the shipping hook's
    /// overlay, editable-target, and active-scan gates without allowing the
    /// raw window procedure to mutate component state.
    pub(crate) fn handle_global_shortcut(
        &mut self,
        mut event: window::GlobalShortcutEvent,
        context: &ComponentContext<Self>,
    ) {
        // WinUI 3 controls are windowless. Replace the Win32 HWND hint with
        // the authoritative focused XAML object while we are on the UI thread.
        event.editable_focused |= focus::editable_control_focused();
        if !global_shortcut_is_allowed(
            event,
            self.blocking_overlay_open(),
            self.palette_open,
            self.diagnostics_busy(),
        ) {
            return;
        }
        match event.command {
            window::GlobalShortcutCommand::TogglePalette => {
                self.set_palette_visibility(!self.palette_open, context);
            }
            window::GlobalShortcutCommand::PalettePrevious => {
                self.palette_active_index = self.palette_active_index.saturating_sub(1);
            }
            window::GlobalShortcutCommand::PaletteNext => {
                let match_count =
                    palette_visible_matches(self.palette_command_specs(), &self.palette_query)
                        .len();
                self.palette_active_index = self
                    .palette_active_index
                    .saturating_add(1)
                    .min(match_count.saturating_sub(1));
            }
            window::GlobalShortcutCommand::PaletteExecute => {
                let matches =
                    palette_visible_matches(self.palette_command_specs(), &self.palette_query);
                if let Some(matched) = matches.get(self.palette_active_index) {
                    let tag = matched.command.tag.to_string();
                    if matched.command.enabled {
                        self.set_palette_visibility(false, context);
                        self.handle_palette_command(tag, context);
                    }
                }
            }
            window::GlobalShortcutCommand::PaletteClose => {
                self.set_palette_visibility(false, context);
            }
            window::GlobalShortcutCommand::Navigate(index) => {
                let Some(page) = index
                    .checked_sub(1)
                    .and_then(|index| Page::ALL.get(usize::from(index)))
                    .copied()
                else {
                    return;
                };
                self.handle_palette_command(page.tag().to_string(), context);
            }
            window::GlobalShortcutCommand::ShowHelp => {
                self.shortcut_help_open = true;
            }
            window::GlobalShortcutCommand::QuickScan => {
                self.begin_diagnostic_scan(ScanKind::Quick, context);
            }
            window::GlobalShortcutCommand::FullScan => {
                self.begin_diagnostic_scan(ScanKind::Full, context);
            }
        }
    }

    pub(crate) fn palette_command_specs(&self) -> Vec<PaletteCommandSpec> {
        let mut commands = Vec::with_capacity(
            PALETTE_NAVIGATION_TEMPLATES.len()
                + PALETTE_SCAN_TEMPLATES.len()
                + PALETTE_REPORT_TEMPLATES.len()
                + PALETTE_APP_TEMPLATES.len()
                + self.diagnostic_results.len()
                + self.diagnostic_catalog.len()
                + 3,
        );
        commands.extend(
            PALETTE_NAVIGATION_TEMPLATES
                .into_iter()
                .map(|template| template.command(true)),
        );
        let scan_idle = !self.diagnostics_busy();
        commands.extend(
            PALETTE_SCAN_TEMPLATES
                .into_iter()
                .map(|template| template.command(scan_idle)),
        );
        if !scan_idle {
            commands.push(PALETTE_STOP_SCAN_TEMPLATE.command(!self.diagnostic_cancel_requested));
        }

        let report_ready = !self.diagnostic_results.is_empty() && self.export_pending.is_none();
        commands.extend(
            PALETTE_REPORT_TEMPLATES
                .into_iter()
                .map(|template| template.command(report_ready)),
        );

        let dark =
            effective_window_theme(self.theme, self.effective_color_scheme) == WindowTheme::Dark;
        commands.push(PaletteCommandSpec {
            section: "App",
            label: Cow::Borrowed(if dark {
                "Switch to Light Theme"
            } else {
                "Switch to Dark Theme"
            }),
            tag: Cow::Borrowed("toggle-theme"),
            keywords: Cow::Borrowed("theme dark light appearance"),
            enabled: !self.settings_saving,
            icon: if dark { FaIcon::Sun } else { FaIcon::Moon },
            shortcut: None,
        });
        commands.extend(
            PALETTE_APP_TEMPLATES
                .into_iter()
                .map(|template| template.command(true)),
        );
        commands.push(PaletteCommandSpec {
            section: "App",
            label: Cow::Borrowed(if self.pane_open {
                "Collapse Navigation Rail"
            } else {
                "Expand Navigation Rail"
            }),
            tag: Cow::Borrowed("toggle-pane"),
            keywords: Cow::Borrowed("sidebar navigation rail collapse expand"),
            enabled: true,
            icon: if self.pane_open {
                FaIcon::AnglesLeft
            } else {
                FaIcon::AnglesRight
            },
            shortcut: None,
        });

        for result in &self.diagnostic_results {
            let name = self
                .diagnostic_catalog
                .iter()
                .find(|task| task.id == result.task_id)
                .map_or(result.task_id.as_str(), |task| task.name.as_str());
            commands.push(PaletteCommandSpec {
                section: "Diagnostics",
                label: Cow::Owned(format!("View Result: {name}")),
                tag: Cow::Owned(format!("view:{}", result.task_id)),
                keywords: Cow::Owned(format!("result diagnostic {}", result.task_id)),
                enabled: true,
                icon: self
                    .diagnostic_catalog
                    .iter()
                    .find(|task| task.id == result.task_id)
                    .map_or(FaIcon::Diagnostics, |task| {
                        diagnostic_palette_icon(&task.category)
                    }),
                shortcut: None,
            });
        }
        for task in &self.diagnostic_catalog {
            commands.push(PaletteCommandSpec {
                section: "Diagnostics",
                label: Cow::Owned(format!("Run: {}", task.name)),
                tag: Cow::Owned(format!("run:{}", task.id)),
                keywords: Cow::Owned(format!("task diagnostic {} {}", task.id, task.category)),
                enabled: scan_idle && (self.is_admin || !task.admin_required),
                icon: diagnostic_palette_icon(&task.category),
                shortcut: None,
            });
        }
        commands
    }

    pub(crate) fn toggle_navigation_rail(&mut self, context: &ComponentContext<Self>) {
        self.pane_open = !self.pane_open;
        let mut submitted = self.settings_snapshot.clone();
        submitted.nav_rail_collapsed = !self.pane_open;
        if self.persist_shell_settings(submitted, context) {
            self.status = if self.pane_open {
                "Navigation expanded"
            } else {
                "Navigation collapsed"
            }
            .to_string();
        }
    }

    /// Invalidate process-page work before changing surfaces. Cancelling the
    /// component task is not sufficient once the native request has reached
    /// its blocking receive, so advance the generation as well; any completion
    /// already in flight is then rejected by the normal request-id guard.
    pub(crate) fn invalidate_process_page_request(&mut self) {
        if let Some(task) = self.process_request_task.take() {
            task.cancel();
        }
        self.process_request_id = next_process_request_id(self.process_request_id);
        self.process_loading = false;
        self.process_last_refresh_started_at = None;
    }

    /// The only path that mutates the active page. Keeping the exit lifecycle
    /// here prevents direct actions (AI, scans, tray), navigation, and command
    /// palette commands from leaving a process query alive behind another page.
    pub(crate) fn transition_to_page(&mut self, next: Page) -> bool {
        let Some(transition) = PageTransition::between(self.page, next) else {
            return false;
        };
        if transition.leaves_processes() {
            self.invalidate_process_page_request();
        }
        if matches!(self.page, Page::Monitor | Page::Processes)
            && !matches!(transition.next, Page::Monitor | Page::Processes)
            && !self.monitoring_paused
        {
            // Keep the worker and its small system snapshot warm, but stop all
            // periodic collection while no live surface consumes it.
            let _ = self
                .native_monitor
                .as_ref()
                .is_some_and(|runtime| runtime.pause());
        }
        self.page = transition.next;
        true
    }

    /// Navigation additionally performs the destination's normal entry work.
    /// Direct workflow transitions use `transition_to_page` so they retain
    /// their existing provider/scan sequencing while sharing the exit guard.
    pub(crate) fn navigate_to_page(&mut self, page: Page, context: &ComponentContext<Self>) {
        if !self.transition_to_page(page) {
            return;
        }
        match page {
            Page::Processes => {
                let _ = self.request_monitor_refresh(context);
                if !self.monitoring_paused || self.monitoring_paused_by_lifecycle {
                    let resumed = self
                        .native_monitor
                        .as_ref()
                        .is_some_and(|runtime| runtime.resume());
                    if resumed {
                        self.monitoring_paused = false;
                        self.monitoring_paused_by_lifecycle = false;
                        if let Some(runtime) = self.native_monitor.as_ref() {
                            let _ = runtime.refresh();
                        }
                    }
                }
                self.process_offset = 0;
                self.selected_process = None;
                self.request_process_page(context, false);
            }
            Page::Monitor => {
                let _ = self.request_monitor_refresh(context);
                if !self.monitoring_paused || self.monitoring_paused_by_lifecycle {
                    let resumed = self
                        .native_monitor
                        .as_ref()
                        .is_some_and(|runtime| runtime.resume());
                    if resumed {
                        self.monitoring_paused = false;
                        self.monitoring_paused_by_lifecycle = false;
                        if let Some(runtime) = self.native_monitor.as_ref() {
                            let _ = runtime.refresh();
                        }
                    }
                }
            }
            Page::History => self.request_history_list(context),
            Page::Ai => self.request_ai_provider_status(context),
            Page::Diagnostics | Page::Issues => {}
        }
    }

    /// Execute one command-palette entry. Page tags reuse the navigation
    /// path; action tags mirror their titlebar/nav equivalents.
    pub(crate) fn handle_palette_command(&mut self, tag: String, context: &ComponentContext<Self>) {
        if let Some(page) = Page::from_tag(&tag) {
            self.navigate_to_page(page, context);
            return;
        }
        if let Some(task_id) = tag.strip_prefix("view:") {
            if self
                .diagnostic_results
                .iter()
                .any(|result| result.task_id == task_id)
            {
                self.selected_result_task_id = Some(task_id.to_string());
                self.transition_to_page(Page::Diagnostics);
                self.status = format!("Selected diagnostic: {task_id}");
            }
            return;
        }
        if let Some(task_id) = tag.strip_prefix("run:").map(str::to_string) {
            self.transition_to_page(Page::Diagnostics);
            self.begin_targeted_diagnostic_scan(&task_id, context);
            return;
        }
        match tag.as_str() {
            "quick-scan" => {
                self.transition_to_page(Page::Diagnostics);
                self.begin_diagnostic_scan(ScanKind::Quick, context);
            }
            "full-scan" => {
                self.transition_to_page(Page::Diagnostics);
                self.begin_diagnostic_scan(ScanKind::Full, context);
            }
            "stop-scan" => self.request_diagnostic_cancel(context),
            "export" => self.request_export_to_file(context),
            "copy-diagnostic-report" => self.request_copy_diagnostic_report(context),
            "support-package" => self.request_support_package(context),
            "share" => self.request_share_to_windowsforum(context),
            "email" => self.request_email_report(context),
            "toggle-pane" => self.toggle_navigation_rail(context),
            "settings" => self.open_settings(context),
            "about" => self.open_about(),
            "shortcut-help" => self.shortcut_help_open = true,
            "toggle-theme" => {
                let next_theme =
                    match effective_window_theme(self.theme, self.effective_color_scheme) {
                        WindowTheme::Dark => WindowTheme::Light,
                        _ => WindowTheme::Dark,
                    };
                let mut submitted = self.settings_snapshot.clone();
                submitted.theme = window_theme_setting(next_theme).to_string();
                if self.persist_shell_settings(submitted, context) {
                    self.theme = next_theme;
                    self.status = format!("{} theme selected", window_theme_setting(next_theme));
                }
            }
            _ => (),
        }
    }

    pub(crate) fn apply_system_completion(
        &mut self,
        completion: SystemCompleted,
        context: &ComponentContext<Self>,
    ) {
        self.system_wait = None;
        let Some(request_kind) = take_matching_system_request(
            &mut self.system_info_request_id,
            &mut self.architecture_request_id,
            completion.request_id,
        ) else {
            // A completion from a superseded startup query must never replace
            // newer shell identity. Keep waiting for the current request ids.
            self.resume_system_wait(context);
            return;
        };

        match completion.result {
            Ok(SystemPayload::SystemInfo(info))
                if request_kind == SystemRequestKind::SystemInfo =>
            {
                self.is_admin = info.is_admin;
                self.system_info = info;
            }
            Ok(SystemPayload::Architecture(architecture))
                if request_kind == SystemRequestKind::Architecture =>
            {
                self.architecture = Some(architecture);
            }
            Ok(_) => {
                let error = "Native system worker returned the wrong payload".to_string();
                self.system_error = Some(error.clone());
                self.status = error;
            }
            Err(error) => {
                let error = error.to_string();
                self.system_error = Some(error.clone());
                self.status = format!("Could not read native system identity · {error}");
            }
        }

        self.resume_system_wait(context);
        self.maybe_begin_startup_scan(context);
    }

    pub(crate) fn stop_system_delivery(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.system_wait = None;
        self.system_info_request_id = None;
        self.architecture_request_id = None;
        self.system_error = Some(reason.clone());
        self.system_receiver = None;
        self.system_runtime = None;
        self.status = reason;
    }

    pub(crate) fn request_monitor_refresh(&mut self, _context: &ComponentContext<Self>) -> bool {
        if let Some(runtime) = self.native_monitor.as_ref() {
            let accepted = runtime.refresh();
            if accepted {
                self.monitor_error = None;
            }
            return accepted;
        }
        if self.deterministic_visual {
            return false;
        }

        match NativeMonitorRuntime::start_with_profile(MonitorProfile::SystemOnly) {
            Ok((runtime, receiver)) => {
                if let Some(wait) = self.backend_wait.take() {
                    wait.cancel();
                }
                if let Some(previous) = self.backend_receiver.take() {
                    previous.close();
                }
                let receiver = Arc::new(receiver);
                receiver.set_wake_handler(UiWakeHandler::new(ui_wake::notify));
                self.backend_wait = None;
                self.backend_receiver = Some(receiver);
                self.native_monitor = Some(Arc::new(runtime));
                self.monitoring_paused = false;
                self.monitoring_paused_by_lifecycle = false;
                self.monitor_error = None;
                self.process_error = None;
                true
            }
            Err(error) => {
                self.monitor_error = Some(format!("Native monitoring could not start: {error}"));
                false
            }
        }
    }

    pub(crate) fn request_process_page(
        &mut self,
        context: &ComponentContext<Self>,
        debounce_filter: bool,
    ) {
        if self.deterministic_visual || self.page != Page::Processes {
            return;
        }

        let Some(runtime) = self.native_monitor.as_ref().map(Arc::clone) else {
            self.process_loading = false;
            self.process_error = Some("Native process inventory is unavailable".to_string());
            return;
        };

        self.process_request_id = next_process_request_id(self.process_request_id);
        let request_id = self.process_request_id;
        let query = ProcessQuery {
            search: self.process_filter.clone(),
            sort_by: self.process_sort_key,
            sort_direction: self.process_sort_direction,
            offset: self.process_offset,
            limit: PROCESS_PAGE_SIZE,
        };

        // Replacing the task cancels a previous debounce. Completions from a
        // query already running on the monitor worker are still harmless: the
        // monotonically increasing request id rejects stale results below.
        if let Some(task) = self.process_request_task.take() {
            task.cancel();
        }
        self.process_last_refresh_started_at = Some(Instant::now());
        self.process_loading = true;
        self.process_error = None;
        self.process_request_task = Some(context.spawn_background_with_rejection(
            move |cancellation| {
                if debounce_filter {
                    let started = Instant::now();
                    while started.elapsed() < PROCESS_FILTER_DEBOUNCE {
                        if cancellation.is_cancelled() {
                            return Message::ProcessQueryDiscarded { request_id };
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                if cancellation.is_cancelled() {
                    return Message::ProcessQueryDiscarded { request_id };
                }

                let result = runtime
                    .request_processes(query)
                    .map_err(|error| error.to_string())
                    .and_then(|receiver| match receiver.blocking_recv() {
                        Ok(wfdiag_native_monitor::ProcessQueryOutcome::Page(page)) => Ok(page),
                        Ok(wfdiag_native_monitor::ProcessQueryOutcome::Superseded) => {
                            Err("process query was superseded by a newer one".to_string())
                        }
                        Err(_) => Err("native process worker closed the query".to_string()),
                    });
                Message::ProcessQueryFinished { request_id, result }
            },
            Message::ProcessQueryRejected { request_id },
        ));
    }

    pub(crate) fn set_process_sort(
        &mut self,
        sort_key: ProcessSortKey,
        context: &ComponentContext<Self>,
    ) {
        if self.process_sort_key == sort_key {
            self.process_sort_direction = match self.process_sort_direction {
                ProcessSortDirection::Asc => ProcessSortDirection::Desc,
                ProcessSortDirection::Desc => ProcessSortDirection::Asc,
            };
        } else {
            self.process_sort_key = sort_key;
            self.process_sort_direction = match sort_key {
                ProcessSortKey::Name | ProcessSortKey::Pid | ProcessSortKey::Status => {
                    ProcessSortDirection::Asc
                }
                _ => ProcessSortDirection::Desc,
            };
        }
        self.process_offset = 0;
        self.selected_process = None;
        self.request_process_page(context, false);
    }

    pub(crate) fn apply_backend_event(&mut self, event: UiEvent) {
        match event {
            diagnostic @ (UiEvent::TaskProgress(_) | UiEvent::DiagnosticResult(_)) => {
                self.apply_diagnostic_event(diagnostic);
            }
            UiEvent::SystemStats(stats) => {
                self.monitor_error = None;
                if !self.monitoring_paused && matches!(self.page, Page::Monitor | Page::Processes) {
                    self.status = format!(
                        "Live sample · CPU {:.0}% · memory {:.0}%",
                        stats.cpu_utilization, stats.memory_utilization
                    );
                }
                self.monitor_history.push_stats(&stats);
                self.latest_system_stats = Some(stats);
            }
            UiEvent::Chat(ChatEvent::Delta(delta)) => self
                .chat_answer
                .get_or_insert_with(String::new)
                .push_str(&delta.text),
            UiEvent::Chat(ChatEvent::Done(done)) => {
                self.status = format!("AI response complete · {}", done.provider);
            }
            UiEvent::Chat(ChatEvent::Error(error)) => {
                self.status = format!("AI error · {}", error.message);
            }
            UiEvent::Chat(_) => self.status = "AI activity received".to_string(),
            UiEvent::Report(_) => self.status = "AI report activity received".to_string(),
            UiEvent::ActionStatus(_) => {
                self.status = "Remediation status received".to_string();
            }
            UiEvent::QuickScan(_) => self.status = "Quick scan requested".to_string(),
        }
    }
}
