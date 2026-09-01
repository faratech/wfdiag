//! History policy: what a completed scan writes, and what retention applies.
//!
//! The engine never guesses these. Retention comes from the live settings
//! document (the history worker re-reads it on every save), the auto-save
//! decision is captured when a scan starts, and the record itself is built
//! from the committed snapshot plus the host's system identity.

use std::collections::HashMap;
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_history::{ScanRecord, Timestamp};
use wfdiag_native_settings::AppSettings;
use wfdiag_native_system::SystemInfo;
use wfdiag_ui_core::DiagnosticTaskResult;

/// The history tag written with a saved scan.
#[must_use]
pub const fn scan_kind_history_tag(scan_kind: ScanKind) -> &'static str {
    match scan_kind {
        ScanKind::Quick => "Quick Scan",
        ScanKind::Full => "Full Scan",
        ScanKind::Targeted => "Manual Diagnostic",
    }
}

/// The human label for a scan kind, used in status text.
#[must_use]
pub const fn scan_kind_label(scan_kind: ScanKind) -> &'static str {
    match scan_kind {
        ScanKind::Quick => "Quick Scan",
        ScanKind::Full => "Full Scan",
        ScanKind::Targeted => "Targeted Scan",
    }
}

/// Executor concurrency, clamping the "unset" zero to the shipping default.
#[must_use]
pub fn scan_concurrency(max_concurrent_tasks: u32) -> usize {
    if max_concurrent_tasks == 0 {
        5
    } else {
        usize::try_from(max_concurrent_tasks).unwrap_or(5)
    }
}

/// Whether a completed scan of this shape is written to history.
///
/// A one-row rerun updates the currently committed scan in place. Persisting
/// it as a separate one-row history entry would lose the scan context, so a
/// targeted rerun never auto-saves regardless of the setting.
#[must_use]
pub const fn auto_save_allowed(auto_save_setting: bool, targeted_rerun: bool) -> bool {
    auto_save_setting && !targeted_rerun
}

/// The retention policy the history worker reads before each save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Whether history is kept at all.
    pub retain_history: bool,
    /// How many scans are retained.
    pub history_limit: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            retain_history: true,
            history_limit: 30,
        }
    }
}

impl From<&AppSettings> for RetentionPolicy {
    fn from(settings: &AppSettings) -> Self {
        Self {
            retain_history: settings.retain_history,
            history_limit: settings.history_limit,
        }
    }
}

impl RetentionPolicy {
    /// The tuple shape the history runtime's retention callback returns.
    #[must_use]
    pub const fn as_tuple(self) -> (bool, u32) {
        (self.retain_history, self.history_limit)
    }
}

/// Build the persisted record for one committed scan.
#[must_use]
pub fn build_scan_record(
    session_id: String,
    system_info: &SystemInfo,
    results: &[DiagnosticTaskResult],
    duration_ms: u64,
    history_tag: String,
    now: Timestamp,
) -> ScanRecord {
    let results: HashMap<String, wfdiag_native_history::SharedTaskResult> = results
        .iter()
        .map(|result| {
            (
                result.task_id.clone(),
                std::sync::Arc::clone(&result.result),
            )
        })
        .collect();
    let success_count = results.values().filter(|result| result.success).count();
    let failure_count = results.len().saturating_sub(success_count);
    ScanRecord {
        id: session_id,
        timestamp: now,
        computer_name: system_info.computer_name.clone(),
        os_version: system_info.os_version.clone(),
        is_admin: system_info.is_admin,
        task_count: results.len(),
        success_count,
        failure_count,
        results,
        duration_ms,
        label: None,
        tags: vec![history_tag],
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RetentionPolicy, auto_save_allowed, build_scan_record, scan_concurrency,
        scan_kind_history_tag,
    };
    use std::sync::Arc;
    use wfdiag_native_diagnostics::ScanKind;
    use wfdiag_native_history::Timestamp;
    use wfdiag_native_issues::TaskResult;
    use wfdiag_native_settings::AppSettings;
    use wfdiag_native_system::SystemInfo;
    use wfdiag_ui_core::DiagnosticTaskResult;

    #[test]
    fn a_targeted_rerun_never_auto_saves() {
        assert!(auto_save_allowed(true, false));
        assert!(!auto_save_allowed(true, true));
        assert!(!auto_save_allowed(false, false));
    }

    #[test]
    fn concurrency_and_tags_match_the_shipping_policy() {
        assert_eq!(scan_concurrency(0), 5);
        assert_eq!(scan_concurrency(3), 3);
        assert_eq!(
            scan_kind_history_tag(ScanKind::Targeted),
            "Manual Diagnostic"
        );
        assert_eq!(scan_kind_history_tag(ScanKind::Quick), "Quick Scan");
    }

    #[test]
    fn retention_follows_the_settings_document() {
        let settings = AppSettings {
            retain_history: false,
            history_limit: 7,
            ..AppSettings::default()
        };
        assert_eq!(
            RetentionPolicy::from(&settings).as_tuple(),
            (false, 7),
            "the worker reads the live policy before each save"
        );
        assert_eq!(RetentionPolicy::default().as_tuple(), (true, 30));
    }

    #[test]
    fn a_record_counts_successes_and_failures_from_the_committed_rows() {
        let rows = vec![
            DiagnosticTaskResult::new(
                "scan_1",
                "os_info",
                Arc::new(TaskResult {
                    success: true,
                    output: "ok".to_string(),
                    error: None,
                    duration_ms: 1,
                }),
            ),
            DiagnosticTaskResult::new(
                "scan_1",
                "processor",
                Arc::new(TaskResult {
                    success: false,
                    output: String::new(),
                    error: Some("failed".to_string()),
                    duration_ms: 2,
                }),
            ),
        ];
        let record = build_scan_record(
            "scan_1".to_string(),
            &SystemInfo {
                computer_name: "TEST-PC".to_string(),
                os_version: "Windows 11".to_string(),
                is_admin: true,
            },
            &rows,
            42,
            "Quick Scan".to_string(),
            Timestamp::from_secs(1_700_000_000),
        );
        assert_eq!(record.task_count, 2);
        assert_eq!(record.success_count, 1);
        assert_eq!(record.failure_count, 1);
        assert_eq!(record.duration_ms, 42);
        assert_eq!(record.tags, ["Quick Scan"]);
        assert!(record.label.is_none());
    }
}
