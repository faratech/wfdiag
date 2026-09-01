//! Encrypted scan history shared by `WFDiag`'s Tauri and native `WinUI` shells.
//!
//! The persistence implementation deliberately retains the shipping v2 file
//! envelope and current-user DPAPI protection. The native command runtime owns
//! all blocking file/DPAPI work on a dedicated Tokio worker, so callers may
//! enqueue requests directly from a `WinUI` thread and await replies elsewhere.

#![deny(unsafe_code)]

mod runtime;

#[allow(unsafe_code)]
mod encrypted_storage;

// Shared primitives compile once in `wfdiag-native-core`. `results_storage.rs`
// and `encrypted_storage.rs` still spell them `crate::error`,
// `crate::timestamp` and `crate::fs_atomic`.
#[allow(unused_imports)]
mod error {
    pub use wfdiag_native_core::error::*;
}
#[allow(unused_imports)]
mod fs_atomic {
    pub use wfdiag_native_core::fs_atomic::*;
}
#[allow(unused_imports)]
mod timestamp {
    pub use wfdiag_native_core::timestamp::*;
}

/// Minimal diagnostic contracts needed by the history format and comparison
/// engine. They intentionally serialize identically to the shipping backend's
/// contracts; the two UI shells can supply their own live task catalog.
pub mod diagnostics {
    use serde::{Deserialize, Serialize};
    use std::sync::{OnceLock, RwLock};
    pub type TaskResult = wfdiag_native_issues::SharedTaskResult;

    #[cfg(test)]
    #[must_use]
    pub fn task_result(success: bool, output: String, duration_ms: u64) -> TaskResult {
        std::sync::Arc::new(wfdiag_native_issues::TaskResult {
            success,
            output,
            error: None,
            duration_ms,
        })
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct DiagnosticTask {
        pub id: String,
        pub name: String,
        pub description: String,
        pub category: String,
        pub admin_required: bool,
    }

    fn catalog() -> &'static RwLock<Vec<DiagnosticTask>> {
        static CATALOG: OnceLock<RwLock<Vec<DiagnosticTask>>> = OnceLock::new();
        CATALOG.get_or_init(|| RwLock::new(Vec::new()))
    }

    /// Replace the fallback metadata used by the compatibility `ScanStorage`
    /// constructor. Production runtimes inject their catalog per instance.
    pub fn set_default_task_catalog(tasks: Vec<DiagnosticTask>) {
        if let Ok(mut current) = catalog().write() {
            *current = tasks;
        }
    }

    #[must_use]
    pub fn get_all_tasks() -> Vec<DiagnosticTask> {
        catalog()
            .read()
            .map(|tasks| tasks.clone())
            .unwrap_or_default()
    }
}

/// Compatibility settings shim used only by `ScanStorage::new`. Native code
/// should prefer `ScanStorage::new_in` or `NativeHistoryRuntime::start` and
/// inject the current settings-backed policy.
pub mod commands {
    pub mod settings {
        #[must_use]
        pub fn history_retention() -> (bool, u32) {
            (true, 30)
        }
    }
}

#[allow(dead_code, clippy::all, clippy::pedantic)]
#[path = "../../../src-tauri/src/results_storage.rs"]
mod storage;

pub use diagnostics::{DiagnosticTask, set_default_task_catalog};
pub use encrypted_storage::EncryptedStorage;
pub use runtime::{HistoryReply, HistoryRuntimeConfig, HistoryRuntimeError, NativeHistoryRuntime};
pub use storage::{
    ComparisonResult, ComparisonSummary, ScanRecord, ScanStorage, ScanSummary, TaskChange,
    TaskChangeSummary, TaskDiffDetail, TaskTrend,
};
pub use timestamp::Timestamp;
pub use wfdiag_native_issues::{
    ScanEvidence, SharedScanEvidence, SharedTaskResult, TaskResult, ordered_scan_evidence,
};

#[cfg(test)]
mod contract_tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn task_result_matches_the_shipping_json_contract() {
        let value = serde_json::to_value(TaskResult {
            success: false,
            output: "output".to_string(),
            error: Some("error".to_string()),
            duration_ms: 42,
        })
        .expect("serialize task result");
        assert_eq!(
            value,
            serde_json::json!({
                "success": false,
                "output": "output",
                "error": "error",
                "duration_ms": 42
            })
        );
    }

    #[test]
    fn scan_record_matches_the_shipping_json_contract() {
        let shared_result = std::sync::Arc::new(TaskResult {
            success: true,
            output: "ok".to_string(),
            error: None,
            duration_ms: 5,
        });
        let record = ScanRecord {
            id: "scan_contract".to_string(),
            timestamp: Timestamp::from_iso_string("2026-08-30T12:00:00Z").expect("timestamp"),
            computer_name: "TEST-PC".to_string(),
            os_version: "Windows 11".to_string(),
            is_admin: true,
            results: HashMap::from([(
                "os_info".to_string(),
                std::sync::Arc::clone(&shared_result),
            )]),
            task_count: 1,
            success_count: 1,
            failure_count: 0,
            duration_ms: 5,
            label: Some("Baseline".to_string()),
            tags: vec!["stable".to_string()],
        };
        let value = serde_json::to_value(&record).expect("serialize scan record");
        assert_eq!(value["timestamp"], "2026-08-30T12:00:00Z");
        assert_eq!(value["results"]["os_info"]["duration_ms"], 5);
        assert_eq!(value["label"], "Baseline");
        assert_eq!(value["tags"], serde_json::json!(["stable"]));
        assert!(value.get("task_count").is_some());
        assert!(value.get("success_count").is_some());
        assert!(value.get("failure_count").is_some());
        let cloned = record.clone();
        assert!(std::sync::Arc::ptr_eq(
            &cloned.results["os_info"],
            &shared_result
        ));
    }
}
