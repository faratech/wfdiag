//! Window lifecycle, navigation, the palette, and the coalesced wake drain.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::WINDOW_COMMAND_POLL;
use crate::app::message::Message;
use crate::app::native_msg::NativeMsg;
use crate::app::policy::{
    MonitoringLifecycleAction, effective_window_theme, global_shortcut_is_allowed,
    monitoring_lifecycle_action, window_hook_retry_delay, window_is_usable, window_theme_setting,
};
use crate::app::state::{Page, PageTransition};
use crate::app::tasks::{spawn_instance_watch, spawn_palette_focus_delay, spawn_window_hook_retry};
use crate::dialogs::export::msg::ExportMsg;
use crate::dialogs::export::msg::ExportPickerKind;
use crate::dialogs::palette::msg::PaletteFocusAction;
use crate::dialogs::palette::view::{
    PALETTE_APP_TEMPLATES, PALETTE_NAVIGATION_TEMPLATES, PALETTE_REPORT_TEMPLATES,
    PALETTE_SCAN_TEMPLATES, PALETTE_STOP_SCAN_TEMPLATE, PaletteCommandSpec,
    diagnostic_palette_icon, palette_visible_matches,
};
use crate::fixtures::knobs::tray_enabled;
use crate::platform::{focus, instance, ui_wake, window};
use crate::widgets::icons::FaIcon;
use std::borrow::Cow;
use wfdiag_app::AppCommand;
use wfdiag_native_diagnostics::ScanKind;
use windows_reactor::*;

impl WfdiagShell {
    /// Drain everything the coalesced native wake stands for.
    ///
    /// There is exactly one engine channel left — the application service's own
    /// event queue — plus the Win32 signals the shell owns itself. The former
    /// per-worker receivers and their `spawn_*_wait` tasks are gone.
    pub(crate) fn drain_native_messages(&mut self) -> Vec<Message> {
        let mut messages = Vec::new();

        if let Some(snapshot) =
            window::lifecycle_snapshot_if_changed(self.shell.window_lifecycle_revision)
        {
            messages.push(Message::Native(NativeMsg::WindowLifecycleChanged(snapshot)));
        }
        if instance::activation_requested() {
            messages.push(Message::Native(NativeMsg::InstanceActivated));
        }
        while let Some(shortcut) = window::take_global_shortcut() {
            messages.push(Message::Native(NativeMsg::GlobalShortcut(shortcut)));
        }
        let tray_command = window::take_tray_command();
        if tray_command != window::TRAY_COMMAND_NONE {
            messages.push(Message::Native(NativeMsg::TrayCommand(tray_command)));
        }
        // The save picker runs on its own STA thread (#140) and answers
        // through the same coalesced wake every other producer uses.
        if let Some(completion) = crate::platform::save_picker::take_completed_picker() {
            messages.push(Message::Export(ExportMsg::PickerFinished {
                epoch: completion.epoch,
                kind: match completion.request {
                    crate::platform::save_picker::SavePickerRequest::Export(_) => {
                        ExportPickerKind::File
                    }
                    crate::platform::save_picker::SavePickerRequest::SupportPackage => {
                        ExportPickerKind::SupportPackage
                    }
                },
                outcome: Box::new(completion.reply),
            }));
        }

        // `AppService::drain` is the only reader of worker output and the only
        // writer of the snapshot, so this is the single point where engine
        // state enters the UI thread. It is pumped even when the batch is
        // empty, because a drain also advances reply deadlines.
        if self.app.is_some() {
            let events = self
                .app
                .as_mut()
                .map(wfdiag_app::AppService::drain)
                .unwrap_or_default();
            messages.push(Message::App(events));
        }
        messages
    }

    /// Re-arm the degraded-path instance/lifecycle watch.
    ///
    /// Only used when the kernel wait registration is unavailable. Dropping a
    /// `ComponentTask` does NOT cancel its closure (windows-reactor keeps the
    /// thread running), so re-arming without cancelling would accumulate live
    /// poll threads until the 64-slot background budget starts rejecting
    /// every other spawn in the app.
    pub(crate) fn arm_instance_watch(
        &mut self,
        context: &ComponentContext<Self>,
        lifecycle_revision: u64,
    ) {
        if instance::activation_wake_registered() {
            return;
        }
        if let Some(previous) = self.shell.instance_wait.take() {
            previous.cancel();
        }
        // #207: this fallback used to spin at 50 ms for the rest of the
        // session with nothing anywhere saying so. It now runs at
        // WINDOW_COMMAND_POLL (250 ms) and says it once.
        if !self.shell.degraded_instance_watch_reported {
            self.shell.degraded_instance_watch_reported = true;
            self.shell.status = format!(
                "Tray and single-instance events are polling every {} ms · Windows refused the \
                 event-driven registration",
                WINDOW_COMMAND_POLL.as_millis()
            );
        }
        self.shell.instance_wait = Some(spawn_instance_watch(context, lifecycle_revision));
    }

    /// Install the tray + close-to-tray hook once the WinUI window exists.
    /// Runs on the UI thread (subclassing requires the owning thread); the
    /// bool guard makes it a cheap no-op afterwards.
    pub(crate) fn ensure_window_hook(&mut self, context: &ComponentContext<Self>) {
        if self.shell.window_hook_installed
            || self.shell.window_hook_retry_task.is_some()
            || self.shell.deterministic_visual
        {
            return;
        }
        let Some(window) = instance::main_window_hwnd() else {
            // `create` runs before windows-reactor materializes its HWND. A
            // one-shot bootstrap reaches this path after the first delay; if
            // window creation is slower, continue with bounded backoff until
            // the exact process-owned window can be discovered.
            self.shell.window_hook_retry_failures =
                self.shell.window_hook_retry_failures.saturating_add(1);
            let delay = window_hook_retry_delay(self.shell.window_hook_retry_failures);
            self.shell.window_hook_retry_task = Some(spawn_window_hook_retry(context, delay));
            return;
        };
        // The validation switch omits only the notification-area icon. The
        // native wake/lifecycle subclass remains required for event delivery.
        let tray_disabled = !tray_enabled();
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
                self.shell.status = format!("System tray unavailable · {}", error.message());
                self.shell.window_hook_installed = true;
                self.shell.window_hook_retry_failures = 0;
                ui_wake::notify();
                return;
            }

            self.shell.window_hook_retry_failures =
                self.shell.window_hook_retry_failures.saturating_add(1);
            let delay = window_hook_retry_delay(self.shell.window_hook_retry_failures);
            self.shell.status = format!(
                "Native window integration is retrying in {} ms · {}",
                delay.as_millis(),
                error.message()
            );
            self.shell.window_hook_retry_task = Some(spawn_window_hook_retry(context, delay));
            return;
        }
        window::set_close_to_tray(!tray_disabled && self.shell.settings.close_to_tray);
        self.shell.window_hook_installed = true;
        self.shell.window_hook_retry_failures = 0;
        // Producers may have queued data before the HWND became available.
        ui_wake::notify();
    }

    pub(crate) fn apply_window_lifecycle(
        &mut self,
        snapshot: window::WindowLifecycleSnapshot,
        context: &ComponentContext<Self>,
    ) {
        self.shell.window_lifecycle_revision = snapshot.revision;
        self.shell.window_usable = window_is_usable(snapshot);
        // Hiding the window is the single biggest idle cost the shell has, so
        // the engine is told about visibility regardless of the current page.
        self.dispatch(AppCommand::WindowVisibility {
            visible: self.shell.window_usable,
        });
        if !self.shell.page.consumes_live_telemetry() {
            return;
        }
        match monitoring_lifecycle_action(
            snapshot,
            self.monitor.paused,
            self.monitor.paused_by_lifecycle,
        ) {
            MonitoringLifecycleAction::Pause => {
                if self
                    .dispatch(AppCommand::SetMonitorPaused { paused: true })
                    .is_accepted()
                {
                    self.monitor.paused = true;
                    self.monitor.paused_by_lifecycle = true;
                    self.processes.loading = false;
                    self.shell.status =
                        "Live monitoring paused while the window is inactive".to_string();
                }
            }
            MonitoringLifecycleAction::ResumeAndRefresh => {
                if self
                    .dispatch(AppCommand::SetMonitorPaused { paused: false })
                    .is_accepted()
                {
                    self.monitor.paused = false;
                    self.monitor.paused_by_lifecycle = false;
                    let _ = self.dispatch(AppCommand::MonitorRefresh);
                    if self.shell.page == Page::Processes {
                        self.request_process_page(context, false);
                    }
                    self.shell.status = "Live monitoring resumed and refreshed".to_string();
                }
            }
            MonitoringLifecycleAction::None => {}
        }
    }

    pub(crate) fn blocking_overlay_open(&self) -> bool {
        self.settings.open
            || self.about.open
            || self.palette.open
            || self.shortcuts.open
            || self.ai.full_scan_consent.is_some()
            || self.ai.cloud_fallback_consent.is_some()
            || self.action_review.open()
            || self.settings.subscription_install_prompt.is_some()
            || self.history.clear_confirm
    }

    pub(crate) fn set_palette_visibility(&mut self, open: bool, context: &ComponentContext<Self>) {
        if self.palette.open == open {
            return;
        }
        if let Some(task) = self.palette.focus_task.take() {
            task.cancel();
        }
        if open {
            focus::capture_pre_palette_focus();
        }
        self.palette.open = open;
        window::set_palette_open(open);
        self.palette.query.clear();
        self.palette.active_index = 0;
        self.palette.epoch = self.palette.epoch.wrapping_add(1);
        let action = if open {
            PaletteFocusAction::FocusQuery
        } else {
            PaletteFocusAction::RestorePrevious
        };
        self.palette.focus_task = Some(spawn_palette_focus_delay(
            context,
            self.palette.epoch,
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
            self.palette.open,
            self.diagnostics.busy(),
        ) {
            return;
        }
        match event.command {
            window::GlobalShortcutCommand::TogglePalette => {
                self.set_palette_visibility(!self.palette.open, context);
            }
            window::GlobalShortcutCommand::PalettePrevious => {
                self.palette.active_index = self.palette.active_index.saturating_sub(1);
            }
            window::GlobalShortcutCommand::PaletteNext => {
                let match_count =
                    palette_visible_matches(self.palette_command_specs(), &self.palette.query)
                        .len();
                self.palette.active_index = self
                    .palette
                    .active_index
                    .saturating_add(1)
                    .min(match_count.saturating_sub(1));
            }
            window::GlobalShortcutCommand::PaletteExecute => {
                let matches =
                    palette_visible_matches(self.palette_command_specs(), &self.palette.query);
                if let Some(matched) = matches.get(self.palette.active_index) {
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
                self.shortcuts.open = true;
            }
            window::GlobalShortcutCommand::QuickScan => {
                self.begin_diagnostic_scan(ScanKind::Quick);
            }
            window::GlobalShortcutCommand::FullScan => {
                self.begin_diagnostic_scan(ScanKind::Full);
            }
        }
    }

    pub(crate) fn palette_command_specs(&self) -> Vec<PaletteCommandSpec> {
        let mut commands = Vec::with_capacity(
            PALETTE_NAVIGATION_TEMPLATES.len()
                + PALETTE_SCAN_TEMPLATES.len()
                + PALETTE_REPORT_TEMPLATES.len()
                + PALETTE_APP_TEMPLATES.len()
                + self.diagnostics.results.len()
                + self.diagnostics.catalog.len()
                + 3,
        );
        commands.extend(
            PALETTE_NAVIGATION_TEMPLATES
                .into_iter()
                .map(|template| template.command(true)),
        );
        let scan_idle = !self.diagnostics.busy();
        commands.extend(
            PALETTE_SCAN_TEMPLATES
                .into_iter()
                .map(|template| template.command(scan_idle)),
        );
        if !scan_idle {
            commands.push(PALETTE_STOP_SCAN_TEMPLATE.command(!self.diagnostics.cancelling()));
        }

        let report_ready = !self.diagnostics.results.is_empty() && self.export.pending.is_none();
        commands.extend(
            PALETTE_REPORT_TEMPLATES
                .into_iter()
                .map(|template| template.command(report_ready)),
        );

        let dark = effective_window_theme(self.shell.theme, self.shell.effective_color_scheme)
            == WindowTheme::Dark;
        commands.push(PaletteCommandSpec {
            section: "App",
            label: Cow::Borrowed(if dark {
                "Switch to Light Theme"
            } else {
                "Switch to Dark Theme"
            }),
            tag: Cow::Borrowed("toggle-theme"),
            keywords: Cow::Borrowed("theme dark light appearance"),
            enabled: !self.settings.saving,
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
            label: Cow::Borrowed(if self.shell.pane_open {
                "Collapse Navigation Rail"
            } else {
                "Expand Navigation Rail"
            }),
            tag: Cow::Borrowed("toggle-pane"),
            keywords: Cow::Borrowed("sidebar navigation rail collapse expand"),
            enabled: true,
            icon: if self.shell.pane_open {
                FaIcon::AnglesLeft
            } else {
                FaIcon::AnglesRight
            },
            shortcut: None,
        });

        for result in &self.diagnostics.results {
            let name = self
                .diagnostics
                .catalog
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
                    .diagnostics
                    .catalog
                    .iter()
                    .find(|task| task.id == result.task_id)
                    .map_or(FaIcon::Diagnostics, |task| {
                        diagnostic_palette_icon(&task.category)
                    }),
                shortcut: None,
            });
        }
        for task in &self.diagnostics.catalog {
            commands.push(PaletteCommandSpec {
                section: "Diagnostics",
                label: Cow::Owned(format!("Run: {}", task.name)),
                tag: Cow::Owned(format!("run:{}", task.id)),
                keywords: Cow::Owned(format!("task diagnostic {} {}", task.id, task.category)),
                enabled: scan_idle && (self.shell.is_admin || !task.admin_required),
                icon: diagnostic_palette_icon(&task.category),
                shortcut: None,
            });
        }
        commands
    }

    pub(crate) fn toggle_navigation_rail(&mut self) {
        self.shell.pane_open = !self.shell.pane_open;
        let mut submitted = self.shell.settings.clone();
        submitted.nav_rail_collapsed = !self.shell.pane_open;
        if self.persist_shell_settings(submitted) {
            self.shell.status = if self.shell.pane_open {
                "Navigation expanded"
            } else {
                "Navigation collapsed"
            }
            .to_string();
        }
    }

    /// The only path that mutates the active page. Keeping the exit lifecycle
    /// here prevents direct actions (AI, scans, tray), navigation, and command
    /// palette commands from leaving a process query alive behind another page.
    pub(crate) fn transition_to_page(&mut self, next: Page) -> bool {
        let Some(transition) = PageTransition::between(self.shell.page, next) else {
            return false;
        };
        if transition.leaves_processes() {
            self.processes.invalidate_request();
        }
        if self.shell.page.consumes_live_telemetry()
            && !transition.next.consumes_live_telemetry()
            && !self.monitor.paused
        {
            // Keep the worker and its small system snapshot warm, but stop all
            // periodic collection while no live surface consumes it.
            if self
                .dispatch(AppCommand::SetMonitorPaused { paused: true })
                .is_accepted()
            {
                self.monitor.paused = true;
            }
        }
        self.shell.page = transition.next;
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
            Page::Processes | Page::Monitor => {
                self.resume_live_monitoring();
                if page == Page::Processes {
                    self.processes.offset = 0;
                    self.processes.selected = None;
                    self.request_process_page(context, false);
                }
            }
            Page::History => self.request_history_list(context),
            Page::Ai => {
                let _ = self.dispatch(AppCommand::RequestProviderStatus);
            }
            Page::Diagnostics | Page::Issues => {}
        }
    }

    /// Resume one-second sampling for a live surface.
    ///
    /// A pause the *lifecycle* imposed is only lifted once the window is
    /// usable again, so entering the page while minimized keeps the collector
    /// idle and records the intent instead.
    fn resume_live_monitoring(&mut self) {
        let _ = self.dispatch(AppCommand::MonitorRefresh);
        if !self.monitor.paused || !self.shell.window_usable {
            return;
        }
        if self
            .dispatch(AppCommand::SetMonitorPaused { paused: false })
            .is_accepted()
        {
            self.monitor.paused = false;
            self.monitor.paused_by_lifecycle = false;
            let _ = self.dispatch(AppCommand::MonitorRefresh);
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
                .diagnostics
                .results
                .iter()
                .any(|result| result.task_id == task_id)
            {
                self.diagnostics.selected_task_id = Some(task_id.to_string());
                self.transition_to_page(Page::Diagnostics);
                self.shell.status = format!("Selected diagnostic: {task_id}");
            }
            return;
        }
        if let Some(task_id) = tag.strip_prefix("run:").map(str::to_string) {
            self.transition_to_page(Page::Diagnostics);
            self.begin_targeted_diagnostic_scan(&task_id);
            return;
        }
        match tag.as_str() {
            "quick-scan" => {
                self.transition_to_page(Page::Diagnostics);
                self.begin_diagnostic_scan(ScanKind::Quick);
            }
            "full-scan" => {
                self.transition_to_page(Page::Diagnostics);
                self.begin_diagnostic_scan(ScanKind::Full);
            }
            "stop-scan" => self.request_diagnostic_cancel(),
            "export" => self.request_export_to_file(),
            "copy-diagnostic-report" => self.request_copy_diagnostic_report(),
            "support-package" => self.request_support_package(),
            "share" => self.request_share_to_windowsforum(),
            "email" => self.request_email_report(),
            "toggle-pane" => self.toggle_navigation_rail(),
            "settings" => self.open_settings(),
            "about" => self.open_about(),
            "shortcut-help" => self.shortcuts.open = true,
            "toggle-theme" => {
                let next_theme = match effective_window_theme(
                    self.shell.theme,
                    self.shell.effective_color_scheme,
                ) {
                    WindowTheme::Dark => WindowTheme::Light,
                    _ => WindowTheme::Dark,
                };
                let mut submitted = self.shell.settings.clone();
                submitted.theme = window_theme_setting(next_theme).to_string();
                if self.persist_shell_settings(submitted) {
                    self.shell.theme = next_theme;
                    self.shell.status =
                        format!("{} theme selected", window_theme_setting(next_theme));
                }
            }
            _ => (),
        }
    }

    // ---- live monitoring surfaces ------------------------------------------

    pub(crate) fn refresh_current_page(&mut self, context: &ComponentContext<Self>) {
        match self.shell.page {
            Page::Ai => {
                let accepted = self
                    .dispatch(AppCommand::RequestProviderStatus)
                    .is_accepted();
                self.shell.status = if accepted {
                    "Checking AI providers…".to_string()
                } else {
                    self.ai.status_error.clone().unwrap_or_else(|| {
                        "Native AI provider discovery is unavailable".to_string()
                    })
                };
            }
            Page::Issues => {
                self.shell.status = if self.shell.deterministic_visual {
                    "Visual fixture mode · live issue refresh disabled".to_string()
                } else if self.diagnostics.results.is_empty() {
                    "Run a completed scan before refreshing issues".to_string()
                } else if self.request_issue_refresh(context) {
                    "Refreshing issues from the latest completed scan…".to_string()
                } else {
                    self.issues
                        .error
                        .clone()
                        .unwrap_or_else(|| "Native issue detection is unavailable".to_string())
                };
            }
            _ => {
                let accepted = self.dispatch(AppCommand::MonitorRefresh).is_accepted();
                if self.shell.page == Page::Processes && accepted {
                    self.request_process_page(context, false);
                }
                self.shell.status = if accepted {
                    format!("{} refresh requested", self.shell.page.nav_label())
                } else {
                    "Native monitoring refresh is unavailable".to_string()
                };
            }
        }
    }

    // ---- tray ---------------------------------------------------------------

    pub(crate) fn handle_tray_command(&mut self, command: u8, context: &ComponentContext<Self>) {
        match command {
            window::TRAY_COMMAND_SHOW => match instance::main_window_hwnd() {
                Some(target) => {
                    // Honor the intent captured when the user opened the menu
                    // (or left-clicked), not live visibility at drain time.
                    if window::take_tray_menu_intent() == window::TRAY_MENU_INTENT_HIDE {
                        window::hide(target);
                    } else {
                        window::restore(target);
                    }
                }
                None => instance::activate_main_window(),
            },
            window::TRAY_COMMAND_QUICK_SCAN => {
                self.transition_to_page(Page::Diagnostics);
                self.begin_diagnostic_scan(ScanKind::Quick);
            }
            window::TRAY_COMMAND_EXIT => {
                // Close for real, not to tray. The engine is stopped in the
                // same order a normal shutdown uses.
                self.begin_engine_shutdown();
                window::request_forced_close();
                if !context.window().request_close() {
                    // Reactor declined the close (never seen in practice).
                    // Disarm the forced close so the next ordinary title-bar
                    // close still honours close-to-tray (#197), and drop the
                    // tray icon.
                    window::cancel_forced_close();
                    if let Some(target) = instance::main_window_hwnd() {
                        window::remove_tray_icon(target);
                    }
                }
            }
            _ => (),
        }
    }

    /// Stop the engine and reap its workers inside the shutdown budget.
    pub(crate) fn begin_engine_shutdown(&mut self) {
        let _ = self.dispatch(AppCommand::Shutdown);
        if let Some(app) = self.app.take() {
            self.app_events = None;
            let report = app.shutdown(crate::app::consts::ENGINE_SHUTDOWN_BUDGET);
            if !report.is_clean() {
                self.shell.status =
                    "Some background work did not stop inside its budget".to_string();
            }
        }
    }
}
