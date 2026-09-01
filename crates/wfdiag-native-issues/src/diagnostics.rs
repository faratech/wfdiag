use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Completed diagnostic evidence consumed by issue detection.
///
/// This is the shipping `TaskResult` wire contract. The Tauri backend and the
/// native diagnostic coordinator re-export it under their existing names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// One diagnostic result shared by every in-process consumer.
///
/// Native collectors allocate the potentially large `output` string once.
/// Cloning this handle for UI delivery, issue detection, export, history, or
/// AI context assembly only increments the reference count.
pub type SharedTaskResult = Arc<TaskResult>;

/// Immutable-by-convention task evidence for one scan.
pub type ScanEvidence = HashMap<String, SharedTaskResult>;

/// Complete immutable evidence snapshot shared across application workers.
pub type SharedScanEvidence = Arc<ScanEvidence>;

/// Read-only result lookup accepted by the detector catalog.
///
/// The compatibility implementation for the shipping owned map lets Tauri
/// retain its existing state contract while native shells pass shared scan
/// evidence without materializing another map of large strings.
pub trait TaskResultLookup {
    fn get_task_result(&self, task_id: &str) -> Option<&TaskResult>;
}

impl TaskResultLookup for HashMap<String, TaskResult> {
    fn get_task_result(&self, task_id: &str) -> Option<&TaskResult> {
        self.get(task_id)
    }
}

impl TaskResultLookup for ScanEvidence {
    fn get_task_result(&self, task_id: &str) -> Option<&TaskResult> {
        self.get(task_id).map(AsRef::as_ref)
    }
}

/// Return shared task results in stable task-id order.
///
/// JSON object order is not semantically significant, but exports, cache
/// fingerprints, and diagnostic tests benefit from deterministic traversal.
#[must_use]
pub fn ordered_scan_evidence(evidence: &ScanEvidence) -> Vec<(&str, &SharedTaskResult)> {
    let mut entries = evidence
        .iter()
        .map(|(task_id, result)| (task_id.as_str(), result))
        .collect::<Vec<_>>();
    entries.sort_unstable_by_key(|(task_id, _)| *task_id);
    entries
}

/// Diagnostic metadata used by the catalog's cross-catalog invariant test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticTask {
    pub id: String,
}

/// Test adapter over the shipping task catalog source.
///
/// Parsing the source keeps the portable invariant test tied to the one
/// shipping catalog without maintaining a second list of task identifiers.
#[cfg(test)]
pub(crate) fn get_all_tasks() -> Vec<DiagnosticTask> {
    include_str!("../../../src-tauri/src/diagnostics.rs")
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("id: \"")
                .and_then(|rest| rest.split_once('"'))
                .map(|(id, _)| DiagnosticTask { id: id.to_string() })
        })
        .collect()
}
