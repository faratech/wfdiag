//! How the Monitor screen answers its own messages and the engine's events.

#![deny(unsafe_code)]

use crate::app::screen::ScreenCx;
use crate::screens::monitor::state::{MonitorMsg, MonitorScreen};
use wfdiag_app::{AppCommand, AppEvent, DispatchOutcome, MonitorEvent};

impl MonitorScreen {
    pub(crate) fn update(&mut self, message: MonitorMsg, cx: &mut ScreenCx<'_>) {
        match message {
            MonitorMsg::ToggleMonitoring => self.toggle(cx),
            MonitorMsg::RequestNetworkConnections => {
                if cx.shell.deterministic_visual || self.network_loading {
                    return;
                }
                // #198: the request goes through the engine, which drops a
                // reply that a newer request has already superseded.
                match cx.dispatch(AppCommand::RequestNetworkConnections) {
                    DispatchOutcome::Accepted { .. } => self.network_loading = true,
                    outcome => cx.report_rejection(&outcome),
                }
            }
        }
    }

    /// Pause or resume live sampling from the page's own button.
    pub(crate) fn toggle(&mut self, cx: &mut ScreenCx<'_>) {
        let pause = !self.paused;
        if !pause && !cx.shell.window_usable {
            // Preserve the user's resume intent without waking the monitor
            // while the app is hidden, minimized, or inactive.
            self.paused_by_lifecycle = true;
            cx.status("Live monitoring will resume when the window is active");
            return;
        }
        if cx
            .dispatch(AppCommand::SetMonitorPaused { paused: pause })
            .is_accepted()
        {
            self.paused = pause;
            self.paused_by_lifecycle = false;
            if !pause {
                let _ = cx.dispatch(AppCommand::MonitorRefresh);
            }
            cx.status(if pause {
                "Live monitoring paused"
            } else {
                "Live monitoring resumed"
            });
        } else {
            cx.status("Native monitoring control is unavailable");
        }
    }

    pub(crate) fn on_app_event(&mut self, event: &AppEvent, cx: &mut ScreenCx<'_>) {
        let AppEvent::Monitor(event) = event else {
            return;
        };
        match event {
            MonitorEvent::Stats(stats) => {
                self.error = None;
                if !self.paused && cx.shell.page.consumes_live_telemetry() {
                    cx.status(format!(
                        "Live sample · CPU {:.0}% · memory {:.0}%",
                        stats.cpu_utilization, stats.memory_utilization
                    ));
                }
                self.history.push_stats(stats);
                self.stats = Some(stats.as_ref().clone());
            }
            MonitorEvent::NetworkConnections(_) => self.network_loading = false,
            MonitorEvent::Unavailable { reason } => {
                self.network_loading = false;
                self.error = Some(reason.clone());
                cx.status(reason.clone());
            }
            _ => {}
        }
    }
}
