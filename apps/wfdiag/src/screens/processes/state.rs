//! The Processes screen's own view state and message alphabet.

#![deny(unsafe_code)]

use crate::screens::processes::view::ProcessViewRow;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use wfdiag_app::ports::monitor::{ProcessPage, ProcessSortDirection, ProcessSortKey};
use wfdiag_native_projection::process_identity::ProcessIdentity;
use windows_reactor::*;

/// Why a process page was asked for.
///
/// #194: only a query the **user** changed — filter, sort, page, or the
/// Refresh button — may show `Refreshing…` and disable the paging buttons.
/// The two-second live tick re-queries the same CPU-sorted page and must
/// update the rows in place, otherwise the summary and the pager flicker
/// twice a second while nothing the user did has changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessQueryOrigin {
    /// The user changed the query, or asked for a refresh.
    User,
    /// The live telemetry tick, refreshing the same query.
    Tick,
}

impl ProcessQueryOrigin {
    /// Whether this query may occupy the page's progress affordances.
    pub(crate) const fn shows_progress(self) -> bool {
        matches!(self, Self::User)
    }
}

/// Everything the Processes page renders.
pub(crate) struct ProcessesScreen {
    pub(crate) filter: String,
    pub(crate) page: Option<ProcessPage>,
    /// The visible rows, rebuilt whenever the engine publishes a page.
    ///
    /// A row whose contents did not change keeps its existing `Arc`, so
    /// [`crate::screens::processes::view::ProcessRowInput`]'s `PartialEq`
    /// short-circuits on a pointer comparison and Reactor leaves that realized
    /// row alone (#194).
    pub(crate) rows: Vec<Arc<ProcessViewRow>>,
    pub(crate) sort_key: ProcessSortKey,
    pub(crate) sort_direction: ProcessSortDirection,
    pub(crate) offset: usize,
    /// Whether a **user-initiated** query is in flight. Live ticks never set
    /// this (#194).
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) selected: Option<ProcessIdentity>,
    /// The process-filter debounce. This is the one remaining
    /// `spawn_background` on the data path: it is pure typing latency, not a
    /// worker wait.
    pub(crate) debounce_revision: u64,
    pub(crate) debounce_task: Option<ComponentTask>,
    pub(crate) last_refresh_started_at: Option<Instant>,
}

impl Default for ProcessesScreen {
    fn default() -> Self {
        Self {
            filter: String::new(),
            page: None,
            rows: Vec::new(),
            sort_key: ProcessSortKey::CpuPercent,
            sort_direction: ProcessSortDirection::Desc,
            offset: 0,
            loading: false,
            error: None,
            selected: None,
            debounce_revision: 0,
            debounce_task: None,
            last_refresh_started_at: None,
        }
    }
}

impl ProcessesScreen {
    /// Record that a query was accepted.
    ///
    /// #194: this is the whole rule. `loading` — which prints `Refreshing…`
    /// in the summary and disables the pager — is set **only** for a query the
    /// user changed. The two-second live tick refreshes the same rows in
    /// place and leaves both affordances alone.
    pub(crate) fn accept_query(&mut self, origin: ProcessQueryOrigin) {
        if origin.shows_progress() {
            self.loading = true;
            self.error = None;
        }
    }

    /// Adopt the engine's current page, reusing the `Arc` of every row whose
    /// contents are unchanged so a live tick re-renders nothing (#194).
    pub(crate) fn set_page(&mut self, page: Option<&ProcessPage>) {
        let Some(page) = page else {
            self.page = None;
            self.rows.clear();
            return;
        };
        let previous: HashMap<ProcessIdentity, Arc<ProcessViewRow>> = self
            .rows
            .drain(..)
            .map(|row| (row.identity(), row))
            .collect();
        self.rows = page
            .items
            .iter()
            .map(|item| {
                let row = ProcessViewRow::from(item);
                match previous.get(&row.identity()) {
                    Some(existing) if **existing == row => Arc::clone(existing),
                    _ => Arc::new(row),
                }
            })
            .collect();
        self.page = Some(page.clone());
    }
}

/// Everything the Processes page can ask for.
#[derive(Clone)]
pub(crate) enum ProcessesMsg {
    FilterChanged(String),
    Sort(ProcessSortKey),
    Previous,
    Next,
    /// The process-filter debounce elapsed. The engine is only asked for a
    /// page once the user stops typing.
    QueryDue {
        revision: u64,
    },
    QueryDebounceEnded {
        revision: u64,
    },
    Select(Option<ProcessIdentity>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use wfdiag_app::ports::monitor::ProcessRow;

    fn row(pid: u32, start_time: i64, cpu: f32) -> ProcessRow {
        ProcessRow {
            pid,
            parent_pid: 1,
            name: format!("proc{pid}"),
            cpu_percent: cpu,
            memory_percent: 1.0,
            memory_mb: 100.0,
            virtual_memory_mb: 200.0,
            gpu_percent: None,
            gpu_memory_mb: None,
            npu_percent: None,
            npu_memory_mb: None,
            cpu_time_secs: 7,
            start_time,
            status: "Running".to_string(),
            thread_count: 4,
            handle_count: 40,
            priority: 8,
            io_read_bytes: 1,
            io_write_bytes: 2,
        }
    }

    fn page(items: Vec<ProcessRow>) -> ProcessPage {
        let total = items.len();
        ProcessPage {
            items,
            total,
            ..ProcessPage::default()
        }
    }

    #[test]
    fn only_a_user_query_shows_progress() {
        assert!(ProcessQueryOrigin::User.shows_progress());
        assert!(!ProcessQueryOrigin::Tick.shows_progress());
    }

    #[test]
    fn live_tick_never_sets_the_loading_flag() {
        let mut screen = ProcessesScreen::default();
        screen.accept_query(ProcessQueryOrigin::Tick);
        assert!(
            !screen.loading,
            "#194: a periodic refresh must not print Refreshing… or disable the pager"
        );
    }

    #[test]
    fn user_query_sets_the_loading_flag_and_clears_the_error() {
        let mut screen = ProcessesScreen {
            error: Some("stale".to_string()),
            ..ProcessesScreen::default()
        };
        screen.accept_query(ProcessQueryOrigin::User);
        assert!(screen.loading);
        assert_eq!(screen.error, None);
    }

    #[test]
    fn live_tick_leaves_a_stale_error_banner_in_place() {
        let mut screen = ProcessesScreen {
            error: Some("stale".to_string()),
            ..ProcessesScreen::default()
        };
        screen.accept_query(ProcessQueryOrigin::Tick);
        assert_eq!(screen.error.as_deref(), Some("stale"));
    }

    #[test]
    fn unchanged_rows_keep_their_arc_across_a_refresh() {
        let mut screen = ProcessesScreen::default();
        screen.set_page(Some(&page(vec![row(10, 5, 1.0), row(11, 6, 2.0)])));
        let first = screen.rows.clone();
        screen.set_page(Some(&page(vec![row(10, 5, 1.0), row(11, 6, 2.0)])));
        assert!(Arc::ptr_eq(&first[0], &screen.rows[0]));
        assert!(Arc::ptr_eq(&first[1], &screen.rows[1]));
    }

    #[test]
    fn a_changed_row_gets_a_new_arc_and_its_neighbours_do_not() {
        let mut screen = ProcessesScreen::default();
        screen.set_page(Some(&page(vec![row(10, 5, 1.0), row(11, 6, 2.0)])));
        let first = screen.rows.clone();
        screen.set_page(Some(&page(vec![row(10, 5, 1.0), row(11, 6, 90.0)])));
        assert!(Arc::ptr_eq(&first[0], &screen.rows[0]));
        assert!(!Arc::ptr_eq(&first[1], &screen.rows[1]));
    }

    #[test]
    fn a_reordered_page_reuses_every_arc() {
        let mut screen = ProcessesScreen::default();
        screen.set_page(Some(&page(vec![row(10, 5, 1.0), row(11, 6, 2.0)])));
        let first = screen.rows.clone();
        // The CPU-sorted page swaps places every couple of seconds; identity
        // keying means both rows are still the same rows (#194).
        screen.set_page(Some(&page(vec![row(11, 6, 2.0), row(10, 5, 1.0)])));
        assert!(Arc::ptr_eq(&first[1], &screen.rows[0]));
        assert!(Arc::ptr_eq(&first[0], &screen.rows[1]));
    }

    #[test]
    fn row_keys_follow_identity_not_position() {
        let mut screen = ProcessesScreen::default();
        screen.set_page(Some(&page(vec![row(10, 5, 1.0), row(10, 9, 2.0)])));
        let keys: Vec<String> = screen.rows.iter().map(|row| row.row_key()).collect();
        assert_eq!(keys[0], "process:10:5");
        assert_ne!(keys[0], keys[1], "a reused PID is not the same row");
    }

    #[test]
    fn clearing_the_page_clears_the_rows() {
        let mut screen = ProcessesScreen::default();
        screen.set_page(Some(&page(vec![row(10, 5, 1.0)])));
        screen.set_page(None);
        assert!(screen.page.is_none());
        assert!(screen.rows.is_empty());
    }
}
