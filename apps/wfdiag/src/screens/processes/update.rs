//! How the Processes screen answers its own messages and the engine's events.

#![deny(unsafe_code)]

use crate::app::consts::{
    PROCESS_FILTER_DEBOUNCE, PROCESS_LIVE_REFRESH_INTERVAL, PROCESS_PAGE_SIZE,
};
use crate::app::message::Message;
use crate::app::policy::rejection_text;
use crate::app::screen::ScreenCx;
use crate::app::state::Page;
use crate::screens::processes::state::{ProcessQueryOrigin, ProcessesMsg, ProcessesScreen};
use std::time::Instant;
use wfdiag_app::ports::monitor::{ProcessQuery, ProcessSortDirection, ProcessSortKey};
use wfdiag_app::{AppCommand, AppEvent, DispatchOutcome, MonitorEvent};
use wfdiag_native_projection::process_identity::{ProcessIdentity, reconcile_process_selection_by};

impl ProcessesScreen {
    pub(crate) fn update(&mut self, message: ProcessesMsg, cx: &mut ScreenCx<'_>) {
        match message {
            ProcessesMsg::FilterChanged(value) => {
                self.filter = value;
                self.offset = 0;
                self.selected = None;
                self.request_page(ProcessQueryOrigin::User, true, cx);
            }
            ProcessesMsg::Sort(sort_key) => self.set_sort(sort_key, cx),
            ProcessesMsg::Previous => {
                self.offset = self.offset.saturating_sub(PROCESS_PAGE_SIZE);
                self.selected = None;
                self.request_page(ProcessQueryOrigin::User, false, cx);
            }
            ProcessesMsg::Next => {
                if let Some(page) = self.page.as_ref()
                    && page.offset.saturating_add(page.items.len()) < page.total
                {
                    self.offset = page.offset.saturating_add(page.limit);
                    self.selected = None;
                    self.request_page(ProcessQueryOrigin::User, false, cx);
                }
            }
            ProcessesMsg::QueryDue { revision } => {
                if revision == self.debounce_revision {
                    self.debounce_task = None;
                    // A debounce only ever guards typing, which is by
                    // definition a user-initiated query change.
                    self.send_page_request(ProcessQueryOrigin::User, cx);
                }
            }
            ProcessesMsg::QueryDebounceEnded { revision } => {
                if revision == self.debounce_revision {
                    self.debounce_task = None;
                    self.loading = false;
                }
            }
            ProcessesMsg::Select(identity) => self.selected = identity,
        }
    }

    fn set_sort(&mut self, sort_key: ProcessSortKey, cx: &mut ScreenCx<'_>) {
        if self.sort_key == sort_key {
            self.sort_direction = match self.sort_direction {
                ProcessSortDirection::Asc => ProcessSortDirection::Desc,
                ProcessSortDirection::Desc => ProcessSortDirection::Asc,
            };
        } else {
            self.sort_key = sort_key;
            self.sort_direction = match sort_key {
                ProcessSortKey::Name | ProcessSortKey::Pid | ProcessSortKey::Status => {
                    ProcessSortDirection::Asc
                }
                _ => ProcessSortDirection::Desc,
            };
        }
        self.offset = 0;
        self.selected = None;
        self.request_page(ProcessQueryOrigin::User, false, cx);
    }

    /// Ask for a process page, optionally after the typing debounce.
    pub(crate) fn request_page(
        &mut self,
        origin: ProcessQueryOrigin,
        debounce_filter: bool,
        cx: &mut ScreenCx<'_>,
    ) {
        if cx.shell.deterministic_visual || cx.shell.page != Page::Processes {
            return;
        }
        if let Some(task) = self.debounce_task.take() {
            task.cancel();
        }
        self.debounce_revision = self.debounce_revision.wrapping_add(1);
        if !debounce_filter {
            self.send_page_request(origin, cx);
            return;
        }
        // The engine has no reason to coalesce keystrokes — that is typing
        // latency, which is the shell's problem.
        let revision = self.debounce_revision;
        self.accept_query(origin);
        self.debounce_task = Some(cx.context.spawn_background_with_rejection(
            move |cancellation| {
                let started = Instant::now();
                while started.elapsed() < PROCESS_FILTER_DEBOUNCE {
                    if cancellation.is_cancelled() {
                        return Message::Processes(ProcessesMsg::QueryDebounceEnded { revision });
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Message::Processes(ProcessesMsg::QueryDue { revision })
            },
            Message::Processes(ProcessesMsg::QueryDebounceEnded { revision }),
        ));
    }

    /// Send the process page the current controls describe.
    pub(crate) fn send_page_request(&mut self, origin: ProcessQueryOrigin, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual || cx.shell.page != Page::Processes {
            return;
        }
        let query = ProcessQuery {
            search: self.filter.clone(),
            sort_by: self.sort_key,
            sort_direction: self.sort_direction,
            offset: self.offset,
            limit: PROCESS_PAGE_SIZE,
        };
        self.last_refresh_started_at = Some(Instant::now());
        match cx.dispatch(AppCommand::RequestProcessPage(query)) {
            DispatchOutcome::Accepted { .. } => self.accept_query(origin),
            outcome => {
                self.loading = false;
                if let Some(reason) = outcome.rejection() {
                    self.error = Some(rejection_text(reason));
                }
            }
        }
    }

    /// Drop any pending process work before changing surfaces. The engine
    /// discards a superseded page on its own; this only stops the debounce.
    pub(crate) fn invalidate_request(&mut self) {
        if let Some(task) = self.debounce_task.take() {
            task.cancel();
        }
        self.debounce_revision = self.debounce_revision.wrapping_add(1);
        self.loading = false;
        self.last_refresh_started_at = None;
    }

    pub(crate) fn on_app_event(&mut self, event: &AppEvent, cx: &mut ScreenCx<'_>) {
        let AppEvent::Monitor(event) = event else {
            return;
        };
        match event {
            MonitorEvent::Stats(_) => {
                // The process table rides the telemetry tick rather than
                // running a timer of its own, at half the sample rate.
                if self.live_refresh_due(cx) {
                    self.request_page(ProcessQueryOrigin::Tick, false, cx);
                }
            }
            MonitorEvent::ProcessPage(page) => {
                self.loading = false;
                self.error = None;
                self.offset = page.offset;
                self.selected = reconcile_process_selection_by(self.selected, &page.items, |row| {
                    ProcessIdentity::new(row.pid, row.start_time)
                });
                cx.status(format!(
                    "Process inventory · {} of {} shown",
                    page.items.len(),
                    page.total
                ));
            }
            MonitorEvent::ProcessPageSuperseded => self.loading = false,
            MonitorEvent::Unavailable { reason } => {
                self.loading = false;
                self.error = Some(reason.clone());
            }
            MonitorEvent::NetworkConnections(_) | MonitorEvent::PausedChanged { .. } => {}
        }
    }

    /// Whether the live tick may re-query the visible page.
    ///
    /// A user-initiated query already in flight wins: `loading` is only ever
    /// set by one of those (#194).
    fn live_refresh_due(&self, cx: &ScreenCx<'_>) -> bool {
        let elapsed = self.last_refresh_started_at.is_none_or(|last| {
            Instant::now().duration_since(last) >= PROCESS_LIVE_REFRESH_INTERVAL
        });
        live_tick_refreshes(
            elapsed,
            cx.live_paused,
            self.loading,
            cx.shell.page == Page::Processes,
        )
    }
}

/// The live tick's admission rule, as a pure function (#194).
///
/// The tick never displaces a user-initiated query: `user_query_in_flight` is
/// the only thing `loading` ever means.
pub(crate) const fn live_tick_refreshes(
    interval_elapsed: bool,
    live_paused: bool,
    user_query_in_flight: bool,
    on_processes_page: bool,
) -> bool {
    interval_elapsed && !live_paused && !user_query_in_flight && on_processes_page
}

#[cfg(test)]
mod tests {
    use super::live_tick_refreshes;

    #[test]
    fn the_live_tick_refreshes_only_on_the_processes_page_when_the_interval_elapsed() {
        assert!(live_tick_refreshes(true, false, false, true));
        assert!(!live_tick_refreshes(false, false, false, true));
        assert!(!live_tick_refreshes(true, true, false, true));
        assert!(!live_tick_refreshes(true, false, false, false));
    }

    #[test]
    fn a_user_query_in_flight_wins_over_the_live_tick() {
        // #194: `loading` only ever means "the user changed the query", so a
        // tick must never displace it — and, symmetrically, a tick can never
        // set it and block the next one.
        assert!(!live_tick_refreshes(true, false, true, true));
    }
}
