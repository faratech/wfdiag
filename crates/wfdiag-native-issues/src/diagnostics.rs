use serde::{Deserialize, Serialize};

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
