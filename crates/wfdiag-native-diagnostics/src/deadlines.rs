//! How long a scan waits for one collector before moving on.
//!
//! A scan used to wait forever on a single task: a slow `defrag /A`, a hung
//! WMI provider or a stalled file read parked the whole run at N-1 of N with
//! no way out but cancelling. Every task now has a deadline; when it passes,
//! the task is reported as a collection error naming the deadline and the
//! scan completes. The collector thread itself cannot be interrupted and
//! finishes (or hits its own command timeout) in the background; its late
//! result is discarded.

use std::time::Duration;

/// The deadline for any task not listed below.
pub const DEFAULT_TASK_DEADLINE: Duration = Duration::from_secs(60);

/// Tasks that legitimately take longer: external tools with their own
/// 300 s command timeout, and collectors that walk large stores.
const DEADLINES: &[(&str, u64)] = &[
    // External tools (each command is capped at 300 s by the executor).
    ("dism_scan_health", 330),
    ("dism_health", 200),
    ("chkdsk", 200),
    ("disk_fragmentation", 240),
    ("dxdiag", 150),
    ("battery_report", 120),
    // Windows Update Agent COM history can be slow on old installations.
    ("windows_update", 120),
    // Event-log channels and large WMI classes.
    ("event_logs", 90),
    ("windows_update_events", 90),
    ("event_codes_critical", 90),
    ("installed_programs", 90),
    ("store_apps", 90),
    ("scheduled_tasks", 90),
    ("services", 90),
    ("drivers_list", 90),
    ("system_driver", 90),
    ("system_devices", 90),
    ("processes", 90),
    // Collectors with their own inner budgets.
    ("disk_usage", 30),
    ("minidump", 30),
    ("network_path", 15),
];

/// The deadline for `task_id`.
#[must_use]
pub fn task_deadline(task_id: &str) -> Duration {
    DEADLINES
        .iter()
        .find(|(id, _)| *id == task_id)
        .map_or(DEFAULT_TASK_DEADLINE, |(_, secs)| {
            Duration::from_secs(*secs)
        })
}

/// The collection error a timed-out task reports.
#[must_use]
pub fn deadline_error(task_name: &str, deadline: Duration) -> String {
    format!(
        "{task_name} did not finish within {} s. The scan continued without it; run this \
         check on its own from Diagnostics to retry, and if it keeps timing out the \
         underlying Windows component (WMI, the event log, or the tool it runs) is stalling.",
        deadline.as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_deadline_is_bounded_and_the_default_is_a_minute() {
        assert_eq!(DEFAULT_TASK_DEADLINE, Duration::from_secs(60));
        for (id, secs) in DEADLINES {
            assert!((15..=330).contains(secs), "{id}: {secs}");
        }
        let mut ids: Vec<&str> = DEADLINES.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), DEADLINES.len(), "duplicate task id");
        assert_eq!(task_deadline("minidump"), Duration::from_secs(30));
        assert_eq!(task_deadline("os_info"), DEFAULT_TASK_DEADLINE);
        assert!(
            deadline_error("BSOD Minidumps", task_deadline("minidump")).contains("within 30 s")
        );
    }
}
