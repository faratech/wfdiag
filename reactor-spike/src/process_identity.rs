//! Stable process selection across refreshed process-explorer pages.
//!
//! A PID alone is not an identity because Windows may reuse it after a process
//! exits. A positive process start time disambiguates that reuse. Some visual
//! fixtures and inaccessible system processes cannot provide a start time;
//! those observations use a documented PID-only fallback without discarding a
//! previously known start time.

#![deny(unsafe_code)]

/// Canonical identity for one process lifetime.
///
/// `start_time == 0` means that the source could not provide a trustworthy
/// creation time. Negative source values are canonicalized to the same unknown
/// sentinel so deterministic fixtures do not need fabricated timestamps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time: i64,
}

impl ProcessIdentity {
    pub const UNKNOWN_START_TIME: i64 = 0;

    #[must_use]
    pub const fn new(pid: u32, start_time: i64) -> Self {
        Self {
            pid,
            start_time: if start_time > 0 {
                start_time
            } else {
                Self::UNKNOWN_START_TIME
            },
        }
    }

    #[must_use]
    pub const fn has_known_start_time(self) -> bool {
        self.start_time > Self::UNKNOWN_START_TIME
    }

    /// Whether a refreshed observation may represent this process lifetime.
    ///
    /// Known creation times must agree. If either side is unknown, PID is the
    /// only available evidence and is used as a fixture-safe fallback.
    #[must_use]
    pub const fn matches_observation(self, observation: Self) -> bool {
        if self.pid != observation.pid {
            return false;
        }
        !self.has_known_start_time()
            || !observation.has_known_start_time()
            || self.start_time == observation.start_time
    }

    /// Reconcile a matching refresh while retaining the strongest identity.
    ///
    /// A newly available start time upgrades a PID-only selection. A transient
    /// unknown observation never erases a start time that was already known.
    #[must_use]
    pub const fn reconcile(self, observation: Self) -> Option<Self> {
        if !self.matches_observation(observation) {
            return None;
        }
        if self.has_known_start_time() {
            Some(self)
        } else if observation.has_known_start_time() {
            Some(observation)
        } else {
            Some(self)
        }
    }
}

/// Minimal identity projection implemented by native rows and test fixtures.
pub trait ProcessIdentitySource {
    fn process_pid(&self) -> u32;
    fn process_start_time(&self) -> i64;

    #[must_use]
    fn process_identity(&self) -> ProcessIdentity {
        ProcessIdentity::new(self.process_pid(), self.process_start_time())
    }
}

#[cfg(windows)]
impl ProcessIdentitySource for wfdiag_native_monitor::ProcessRow {
    fn process_pid(&self) -> u32 {
        self.pid
    }

    fn process_start_time(&self) -> i64 {
        self.start_time
    }
}

/// Build the stable identity stored by the process-selection state.
// Retained for library consumers and fixtures that do not own a native
// ProcessRow. The executable currently constructs its local row projection
// through the trait-equivalent `ProcessIdentity::new` path.
#[allow(dead_code)]
#[must_use]
pub fn process_identity(row: &(impl ProcessIdentitySource + ?Sized)) -> ProcessIdentity {
    row.process_identity()
}

fn match_quality(selection: ProcessIdentity, observation: ProcessIdentity) -> Option<u8> {
    if !selection.matches_observation(observation) {
        return None;
    }
    Some(
        match (
            selection.has_known_start_time(),
            observation.has_known_start_time(),
        ) {
            // An exact lifetime match is stronger than any PID-only fallback.
            (true, true) => 3,
            // Prefer upgrading a fixture/unknown selection when the refresh has a
            // trustworthy creation time.
            (false, true) => 2,
            (true, false) | (false, false) => 1,
        },
    )
}

/// Locate the selected row in a refreshed page.
///
/// The quality ordering only matters for malformed/fixture pages containing
/// duplicate PIDs: an exact known lifetime wins over an unknown fallback.
#[must_use]
pub fn selected_process_row<Row: ProcessIdentitySource>(
    selection: Option<ProcessIdentity>,
    rows: &[Row],
) -> Option<&Row> {
    let selection = selection?;
    rows.iter()
        .filter_map(|row| {
            match_quality(selection, row.process_identity()).map(|quality| (quality, row))
        })
        .max_by_key(|(quality, _)| *quality)
        .map(|(_, row)| row)
}

/// Keep, upgrade, or clear selection after a refreshed `ProcessRow` page.
///
/// Returning `None` means the selected process disappeared from the current
/// page or its PID was reused by a process with a different known start time.
#[must_use]
pub fn reconcile_process_selection<Row: ProcessIdentitySource>(
    selection: Option<ProcessIdentity>,
    rows: &[Row],
) -> Option<ProcessIdentity> {
    let selection = selection?;
    let observation = selected_process_row(Some(selection), rows)?.process_identity();
    selection.reconcile(observation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct FixtureRow {
        pid: u32,
        start_time: i64,
        sample: &'static str,
    }

    impl FixtureRow {
        const fn new(pid: u32, start_time: i64, sample: &'static str) -> Self {
            Self {
                pid,
                start_time,
                sample,
            }
        }
    }

    impl ProcessIdentitySource for FixtureRow {
        fn process_pid(&self) -> u32 {
            self.pid
        }

        fn process_start_time(&self) -> i64 {
            self.start_time
        }
    }

    #[test]
    fn pid_reuse_with_a_different_known_start_time_clears_selection() {
        let selected = ProcessIdentity::new(4242, 100);
        let refreshed = [FixtureRow::new(4242, 200, "reused")];

        assert!(selected_process_row(Some(selected), &refreshed).is_none());
        assert_eq!(
            reconcile_process_selection(Some(selected), &refreshed),
            None
        );
    }

    #[test]
    fn disappearance_from_the_refreshed_page_clears_selection() {
        let selected = ProcessIdentity::new(4242, 100);
        let refreshed = [
            FixtureRow::new(7, 10, "other"),
            FixtureRow::new(8, 11, "other-2"),
        ];

        assert_eq!(
            reconcile_process_selection(Some(selected), &refreshed),
            None
        );
    }

    #[test]
    fn unchanged_identity_selects_the_newest_row_data() {
        let old = FixtureRow::new(4242, 100, "old sample");
        let refreshed = [FixtureRow::new(4242, 100, "new sample")];
        let selected = process_identity(&old);

        assert_eq!(
            reconcile_process_selection(Some(selected), &refreshed),
            Some(selected)
        );
        assert_eq!(
            selected_process_row(Some(selected), &refreshed).map(|row| row.sample),
            Some("new sample")
        );
    }

    #[test]
    fn unknown_fixture_start_times_use_a_canonical_pid_fallback() {
        let selected = ProcessIdentity::new(4242, -1);
        let refreshed = [FixtureRow::new(4242, 0, "fixture refresh")];

        assert_eq!(selected.start_time, ProcessIdentity::UNKNOWN_START_TIME);
        assert_eq!(
            reconcile_process_selection(Some(selected), &refreshed),
            Some(ProcessIdentity::new(4242, 0))
        );
        assert!(selected_process_row(Some(selected), &refreshed).is_some());
    }

    #[test]
    fn unknown_selection_upgrades_when_a_known_start_time_arrives() {
        let selected = ProcessIdentity::new(4242, 0);
        let refreshed = [FixtureRow::new(4242, 777, "native sample")];

        assert_eq!(
            reconcile_process_selection(Some(selected), &refreshed),
            Some(ProcessIdentity::new(4242, 777))
        );
    }

    #[test]
    fn transient_unknown_refresh_does_not_erase_a_known_start_time() {
        let selected = ProcessIdentity::new(4242, 777);
        let refreshed = [FixtureRow::new(4242, 0, "restricted sample")];

        assert_eq!(
            reconcile_process_selection(Some(selected), &refreshed),
            Some(selected)
        );
    }

    #[test]
    fn exact_known_match_wins_over_duplicate_unknown_fixture_row() {
        let selected = ProcessIdentity::new(4242, 777);
        let refreshed = [
            FixtureRow::new(4242, 0, "unknown duplicate"),
            FixtureRow::new(4242, 777, "exact"),
        ];

        assert_eq!(
            selected_process_row(Some(selected), &refreshed).map(|row| row.sample),
            Some("exact")
        );
        assert_eq!(
            reconcile_process_selection(Some(selected), &refreshed),
            Some(selected)
        );
    }

    #[test]
    fn absent_selection_stays_absent() {
        let refreshed = [FixtureRow::new(4242, 100, "sample")];

        assert!(selected_process_row(None, &refreshed).is_none());
        assert_eq!(reconcile_process_selection(None, &refreshed), None);
    }
}
