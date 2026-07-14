use crate::diagnostics::TaskResult;
use crate::encrypted_storage::EncryptedStorage;
use crate::error::DiagError;
use crate::timestamp::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const SUMMARY_INDEX_ID: &str = "_scan_summary_index";

// Simplified scan record for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanRecord {
    pub id: String,
    pub timestamp: Timestamp,
    pub computer_name: String,
    pub os_version: String,
    pub is_admin: bool,
    pub results: HashMap<String, TaskResult>,
    pub task_count: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub tags: Vec<String>,
}

// Simplified scan summary for listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSummary {
    pub id: String,
    pub timestamp: Timestamp,
    pub computer_name: String,
    pub task_count: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub tags: Vec<String>,
}

// Simplified comparison result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    pub current_scan: ScanSummary,
    pub previous_scan: ScanSummary,
    pub total_changes: usize,
    pub new_failures: Vec<TaskChange>,
    pub new_successes: Vec<TaskChange>,
    pub status_unchanged: Vec<TaskChange>,
}

/// Lightweight comparison contract for the History screen. Full diagnostic
/// outputs are fetched only when the user expands one task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonSummary {
    pub current_scan: ScanSummary,
    pub previous_scan: ScanSummary,
    pub total_changes: usize,
    pub new_failures: Vec<TaskChangeSummary>,
    pub new_successes: Vec<TaskChangeSummary>,
    pub status_unchanged: Vec<TaskChangeSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskChangeSummary {
    pub task_id: String,
    pub task_name: String,
    pub category: String,
    pub current_success: bool,
    pub previous_success: bool,
    pub output_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDiffDetail {
    pub task_id: String,
    pub current_output: String,
    pub previous_output: String,
}

impl From<TaskChange> for TaskChangeSummary {
    fn from(change: TaskChange) -> Self {
        Self {
            task_id: change.task_id,
            task_name: change.task_name,
            category: change.category,
            current_success: change.current_success,
            previous_success: change.previous_success,
            output_changed: change.output_changed,
        }
    }
}

// Failure trend for one task across recent scans
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTrend {
    pub task_id: String,
    /// Scans (of the considered window) in which the task failed
    pub failed: usize,
    /// Scans in which the task was present at all
    pub seen_in: usize,
    /// Size of the window actually loaded
    pub scans_considered: usize,
}

// Simple task change representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskChange {
    pub task_id: String,
    pub task_name: String,
    pub category: String,
    pub current_success: bool,
    pub previous_success: bool,
    pub current_output: String,
    pub previous_output: String,
    pub output_changed: bool,
}

pub struct ScanStorage {
    #[allow(dead_code)]
    storage_dir: PathBuf,
    encrypted_storage: EncryptedStorage,
}

impl ScanStorage {
    fn summary_for(scan: &ScanRecord) -> ScanSummary {
        ScanSummary {
            id: scan.id.clone(),
            timestamp: scan.timestamp,
            computer_name: scan.computer_name.clone(),
            task_count: scan.task_count,
            success_count: scan.success_count,
            failure_count: scan.failure_count,
            duration_ms: scan.duration_ms,
            label: scan.label.clone(),
            tags: scan.tags.clone(),
        }
    }

    pub fn new() -> Result<Self, String> {
        let storage_dir = Self::get_storage_directory()?;

        // Create directory if it doesn't exist
        if let Err(e) = fs::create_dir_all(&storage_dir) {
            return Err(DiagError::file(storage_dir.display().to_string(), e.to_string()).into());
        }

        // Initialize encrypted storage
        let encrypted_storage = EncryptedStorage::new(storage_dir.clone())
            .map_err(|e| DiagError::storage("init_encrypted_storage", e.to_string()))?;

        println!("Scan storage initialized at: {:?}", storage_dir);

        Ok(Self {
            storage_dir,
            encrypted_storage,
        })
    }

    fn get_storage_directory() -> Result<PathBuf, String> {
        let app_data = std::env::var("APPDATA")
            .map_err(|_| DiagError::internal("Failed to get APPDATA directory"))?;

        Ok(PathBuf::from(app_data).join("wfdiag-tauri").join("scans"))
    }

    pub fn save_scan(&self, scan: &ScanRecord) -> Result<(), String> {
        println!("Saving encrypted scan {}", scan.id);

        let (retain_history, _) = crate::commands::settings::history_retention();
        if !retain_history {
            println!("Scan history retention is disabled; skipping persistent save");
            return Ok(());
        }

        // Validate scan data
        if scan.id.is_empty() {
            return Err(DiagError::storage("save_scan", "Scan ID cannot be empty").into());
        }

        if scan.results.is_empty() {
            return Err(
                DiagError::storage("save_scan", "Scan must have at least one result").into(),
            );
        }

        // Store using encrypted storage
        self.encrypted_storage
            .store(&scan.id, scan)
            .map_err(|e| DiagError::storage("save_encrypted_scan", e.to_string()))?;
        self.upsert_summary(Self::summary_for(scan))?;

        println!("Successfully saved encrypted scan {}", scan.id);

        // Clean up old scans
        self.cleanup_old_scans()?;

        Ok(())
    }

    pub fn load_scan(&self, scan_id: &str) -> Result<ScanRecord, String> {
        if !self.encrypted_storage.exists(scan_id) {
            return Err(DiagError::storage(
                "load_scan",
                format!("Encrypted scan '{}' not found", scan_id),
            )
            .into());
        }

        println!("Loading encrypted scan {}", scan_id);

        let scan: ScanRecord = self
            .encrypted_storage
            .load(scan_id)
            .map_err(|e| DiagError::storage("load_encrypted_scan", e.to_string()))?;

        // Validate loaded scan
        if scan.id != scan_id {
            return Err(DiagError::SessionMismatch {
                expected: scan_id.to_string(),
                actual: scan.id.clone(),
            }
            .into());
        }

        println!(
            "Successfully loaded encrypted scan {} with {} results",
            scan.id,
            scan.results.len()
        );

        Ok(scan)
    }

    pub fn list_scans(&self) -> Result<Vec<ScanSummary>, String> {
        println!("Listing encrypted scans");

        // Listing encrypted filenames is cheap and lets us detect an index
        // write that failed after the full scan was committed. Checking only
        // that indexed entries still existed missed orphaned scan files and
        // made otherwise valid history permanently invisible.
        let stored_scan_ids: std::collections::HashSet<String> = self
            .encrypted_storage
            .list_files()
            .map_err(|e| DiagError::storage("list_scans", e.to_string()))?
            .into_iter()
            .filter(|id| id != SUMMARY_INDEX_ID)
            .collect();

        if self.encrypted_storage.exists(SUMMARY_INDEX_ID)
            && let Ok(mut summaries) = self
                .encrypted_storage
                .load::<Vec<ScanSummary>>(SUMMARY_INDEX_ID)
            && summaries.len() == stored_scan_ids.len()
            && summaries
                .iter()
                .all(|summary| stored_scan_ids.contains(&summary.id))
        {
            summaries.sort_by_key(|summary| std::cmp::Reverse(summary.timestamp));
            return Ok(summaries);
        }

        self.rebuild_summary_index()
    }

    fn rebuild_summary_index(&self) -> Result<Vec<ScanSummary>, String> {
        let mut summaries = Vec::new();

        let scan_ids = self
            .encrypted_storage
            .list_files()
            .map_err(|e| DiagError::storage("list_scans", e.to_string()))?;

        for scan_id in scan_ids.into_iter().filter(|id| id != SUMMARY_INDEX_ID) {
            println!("Found encrypted scan: {}", scan_id);

            match self.load_scan(&scan_id) {
                Ok(scan) => {
                    summaries.push(Self::summary_for(&scan));
                }
                Err(e) => {
                    println!("Warning: Failed to load scan '{}': {}", scan_id, e);
                    // Continue loading other scans instead of failing completely
                }
            }
        }

        // Sort by timestamp (newest first)
        summaries.sort_by_key(|b| std::cmp::Reverse(b.timestamp));

        println!("Successfully loaded {} scan summaries", summaries.len());

        self.encrypted_storage
            .store(SUMMARY_INDEX_ID, &summaries)
            .map_err(|error| DiagError::storage("write_summary_index", error.to_string()))?;

        Ok(summaries)
    }

    fn upsert_summary(&self, summary: ScanSummary) -> Result<(), String> {
        let mut summaries = if self.encrypted_storage.exists(SUMMARY_INDEX_ID) {
            match self
                .encrypted_storage
                .load::<Vec<ScanSummary>>(SUMMARY_INDEX_ID)
            {
                Ok(summaries) => summaries,
                Err(_) => self.rebuild_summary_index()?,
            }
        } else {
            self.rebuild_summary_index()?
        };
        summaries.retain(|existing| existing.id != summary.id);
        summaries.push(summary);
        summaries.sort_by_key(|item| std::cmp::Reverse(item.timestamp));
        self.encrypted_storage
            .store(SUMMARY_INDEX_ID, &summaries)
            .map_err(|error| DiagError::storage("write_summary_index", error.to_string()).into())
    }

    pub fn compare_scans(
        &self,
        current_id: &str,
        previous_id: &str,
    ) -> Result<ComparisonResult, String> {
        println!("Comparing scans: '{}' vs '{}'", current_id, previous_id);

        if current_id == previous_id {
            return Err(
                DiagError::storage("compare_scans", "Cannot compare scan with itself").into(),
            );
        }

        // Load both scans
        let current = self.load_scan(current_id)?;
        let previous = self.load_scan(previous_id)?;

        println!("Loaded current scan: {} results", current.results.len());
        println!("Loaded previous scan: {} results", previous.results.len());

        Ok(Self::compute_comparison(current, previous))
    }

    pub fn compare_scans_summary(
        &self,
        current_id: &str,
        previous_id: &str,
    ) -> Result<ComparisonSummary, String> {
        let comparison = self.compare_scans(current_id, previous_id)?;
        Ok(ComparisonSummary {
            current_scan: comparison.current_scan,
            previous_scan: comparison.previous_scan,
            total_changes: comparison.total_changes,
            new_failures: comparison
                .new_failures
                .into_iter()
                .map(TaskChangeSummary::from)
                .collect(),
            new_successes: comparison
                .new_successes
                .into_iter()
                .map(TaskChangeSummary::from)
                .collect(),
            status_unchanged: comparison
                .status_unchanged
                .into_iter()
                .map(TaskChangeSummary::from)
                .collect(),
        })
    }

    pub fn scan_task_diff(
        &self,
        current_id: &str,
        previous_id: &str,
        task_id: &str,
    ) -> Result<TaskDiffDetail, String> {
        if current_id == previous_id {
            return Err(
                DiagError::storage("scan_task_diff", "Cannot compare scan with itself").into(),
            );
        }
        let current = self.load_scan(current_id)?;
        let previous = self.load_scan(previous_id)?;
        let current_result = current.results.get(task_id).ok_or_else(|| {
            DiagError::storage(
                "scan_task_diff",
                format!("Task '{}' is not present in the current scan", task_id),
            )
            .to_string()
        })?;
        let previous_result = previous.results.get(task_id).ok_or_else(|| {
            DiagError::storage(
                "scan_task_diff",
                format!("Task '{}' is not present in the previous scan", task_id),
            )
            .to_string()
        })?;
        Ok(TaskDiffDetail {
            task_id: task_id.to_string(),
            current_output: current_result.output.clone(),
            previous_output: previous_result.output.clone(),
        })
    }

    /// Pure diff of two loaded scans (storage-free, so it is unit-testable).
    pub(crate) fn compute_comparison(
        current: ScanRecord,
        previous: ScanRecord,
    ) -> ComparisonResult {
        // Get task metadata for proper names and categories
        let all_tasks = crate::diagnostics::get_all_tasks();
        let task_map: HashMap<String, &crate::diagnostics::DiagnosticTask> =
            all_tasks.iter().map(|t| (t.id.clone(), t)).collect();

        let mut new_failures = Vec::new();
        let mut new_successes = Vec::new();
        let mut status_unchanged = Vec::new();

        // Find all tasks that exist in either scan
        let mut all_task_ids: std::collections::HashSet<String> =
            current.results.keys().cloned().collect();
        all_task_ids.extend(previous.results.keys().cloned());

        for task_id in all_task_ids {
            let current_result = current.results.get(&task_id);
            let previous_result = previous.results.get(&task_id);

            let task_info = task_map.get(&task_id);
            let task_name = task_info
                .map(|t| t.name.clone())
                .unwrap_or_else(|| task_id.clone());
            let category = task_info
                .map(|t| t.category.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            match (current_result, previous_result) {
                (Some(current), Some(previous)) => {
                    // Task exists in both scans
                    // For JSON outputs, compare parsed data instead of raw strings
                    let output_changed = if current.success && previous.success {
                        !Self::outputs_are_equivalent_for_task(
                            &task_id,
                            &current.output,
                            &previous.output,
                        )
                    } else {
                        current.output != previous.output
                    };

                    let change = TaskChange {
                        task_id: task_id.clone(),
                        task_name,
                        category,
                        current_success: current.success,
                        previous_success: previous.success,
                        current_output: current.output.clone(),
                        previous_output: previous.output.clone(),
                        output_changed,
                    };

                    if current.success != previous.success {
                        if current.success && !previous.success {
                            new_successes.push(change);
                        } else {
                            new_failures.push(change);
                        }
                    } else {
                        status_unchanged.push(change);
                    }
                }
                (Some(_), None) | (None, Some(_)) => {
                    // Task present in only ONE scan (different task selections between
                    // runs — e.g. a full scan vs a partial scan). There is no before/after
                    // to compare, so this is not a status transition. Previously the
                    // (None, Some) case pushed EVERY not-re-run task into `new_failures`
                    // with current_success=false, raising a false "these just broke" alarm
                    // and inflating total_changes. Skip non-intersection tasks entirely.
                    let _ = (task_name, category);
                    continue;
                }
                (None, None) => {
                    // This shouldn't happen due to how we build the set
                    continue;
                }
            }
        }

        let total_changes = new_failures.len()
            + new_successes.len()
            + status_unchanged.iter().filter(|c| c.output_changed).count();

        println!(
            "Comparison complete: {} total changes, {} new failures, {} new successes",
            total_changes,
            new_failures.len(),
            new_successes.len()
        );

        ComparisonResult {
            current_scan: ScanSummary {
                id: current.id,
                timestamp: current.timestamp,
                computer_name: current.computer_name,
                task_count: current.task_count,
                success_count: current.success_count,
                failure_count: current.failure_count,
                duration_ms: current.duration_ms,
                label: current.label,
                tags: current.tags,
            },
            previous_scan: ScanSummary {
                id: previous.id,
                timestamp: previous.timestamp,
                computer_name: previous.computer_name,
                task_count: previous.task_count,
                success_count: previous.success_count,
                failure_count: previous.failure_count,
                duration_ms: previous.duration_ms,
                label: previous.label,
                tags: previous.tags,
            },
            total_changes,
            new_failures,
            new_successes,
            status_unchanged,
        }
    }

    /// Compare two outputs intelligently, parsing JSON when possible
    #[cfg(test)]
    fn outputs_are_equivalent(output1: &str, output2: &str) -> bool {
        Self::outputs_are_equivalent_for_task("", output1, output2)
    }

    fn outputs_are_equivalent_for_task(task_id: &str, output1: &str, output2: &str) -> bool {
        // First, try quick string comparison
        if output1 == output2 {
            return true;
        }

        // These snapshots are intentionally volatile and do not represent
        // configuration drift. Their pass/fail transition remains visible,
        // but comparing every sampled value produces noise rather than signal.
        if matches!(task_id, "processes" | "performance") {
            return true;
        }

        // Deferred-file-operation counts are volatile housekeeping, not a
        // restart-state transition. Track only the authoritative requirement
        // and its recognized Windows source markers.
        if task_id == "pending_reboot"
            && let (Some(current), Some(previous)) = (
                Self::pending_reboot_comparison_key(output1),
                Self::pending_reboot_comparison_key(output2),
            )
        {
            return current == previous;
        }

        // Try to parse both as JSON for deep comparison
        if let (Ok(json1), Ok(json2)) = (
            serde_json::from_str::<serde_json::Value>(output1),
            serde_json::from_str::<serde_json::Value>(output2),
        ) {
            // Compare parsed JSON objects
            Self::json_values_equal(&json1, &json2)
        } else {
            // Not JSON or parsing failed, fall back to string comparison
            // Normalize whitespace and compare
            let normalized1 = output1.trim().replace("\r\n", "\n");
            let normalized2 = output2.trim().replace("\r\n", "\n");
            normalized1 == normalized2
        }
    }

    fn pending_reboot_comparison_key(output: &str) -> Option<(bool, Vec<String>)> {
        let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
        let object = value.as_object()?;
        let explicit = match object.get("restart_required") {
            Some(value) => Some(value.as_bool()?),
            None => None,
        };
        let pending = match object.get("pending") {
            Some(value) => Some(value.as_bool()?),
            None => None,
        };
        if explicit.is_none() && pending.is_none() {
            return None;
        }
        let mut reasons = Vec::new();
        for reason in object.get("reasons")?.as_array()? {
            let reason = reason.as_str()?;
            match reason {
                "component_based_servicing" | "windows_update" => {
                    reasons.push(reason.to_string());
                }
                "pending_file_rename" | "pending_file_operations" => {}
                _ => return None,
            }
        }
        reasons.sort_unstable();
        reasons.dedup();
        let required = explicit.unwrap_or_else(|| pending == Some(true) && !reasons.is_empty());
        Some((required, reasons))
    }

    /// Deep comparison of JSON values, ignoring formatting and key order
    fn json_values_equal(val1: &serde_json::Value, val2: &serde_json::Value) -> bool {
        use serde_json::Value;

        match (val1, val2) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => Self::json_numbers_equal(a, b),
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    return false;
                }

                // WMI does not guarantee row order. When rows expose a stable
                // identity, compare them as a keyed set instead of positionally.
                let keyed = |items: &[Value]| {
                    let mut map = std::collections::BTreeMap::new();
                    for item in items {
                        let identity = Self::json_identity(item)?;
                        if map.insert(identity, item.clone()).is_some() {
                            return None;
                        }
                    }
                    Some(map)
                };
                if let (Some(left), Some(right)) = (keyed(a), keyed(b)) {
                    if left.keys().ne(right.keys()) {
                        return false;
                    }
                    return left.iter().all(|(identity, left_value)| {
                        right.get(identity).is_some_and(|right_value| {
                            Self::json_values_equal(left_value, right_value)
                        })
                    });
                }

                a.iter()
                    .zip(b.iter())
                    .all(|(left, right)| Self::json_values_equal(left, right))
            }
            (Value::Object(a), Value::Object(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                // Compare all keys and values
                for (key, val1) in a {
                    match b.get(key) {
                        Some(val2) => {
                            if !Self::json_values_equal(val1, val2) {
                                return false;
                            }
                        }
                        None => return false,
                    }
                }
                true
            }
            _ => false, // Different types
        }
    }

    fn json_identity(value: &serde_json::Value) -> Option<String> {
        let object = value.as_object()?;
        const IDENTITY_KEYS: &[&str] = &[
            "id",
            "deviceid",
            "device_id",
            "pnpdeviceid",
            "guid",
            "name",
            "displayname",
            "display_name",
            "caption",
            "driveletter",
            "mount_point",
            "pid",
        ];

        for wanted in IDENTITY_KEYS {
            if let Some((key, value)) = object
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(wanted))
            {
                let rendered = match value {
                    serde_json::Value::String(value) => value.clone(),
                    serde_json::Value::Number(value) => value.to_string(),
                    _ => continue,
                };
                if !rendered.is_empty() {
                    return Some(format!("{}={}", key.to_ascii_lowercase(), rendered));
                }
            }
        }
        None
    }

    fn json_numbers_equal(a: &serde_json::Number, b: &serde_json::Number) -> bool {
        if a == b {
            return true;
        }

        let a_is_integer = a.as_i64().is_some() || a.as_u64().is_some();
        let b_is_integer = b.as_i64().is_some() || b.as_u64().is_some();

        // Distinct JSON integers must stay distinct even when they are larger
        // than f64 can represent exactly.
        if a_is_integer && b_is_integer {
            return false;
        }

        const MAX_SAFE_F64_INTEGER: u64 = 9_007_199_254_740_991;
        let integer_is_safe = |n: &serde_json::Number| {
            n.as_i64()
                .map(|i| i.unsigned_abs() <= MAX_SAFE_F64_INTEGER)
                .or_else(|| n.as_u64().map(|u| u <= MAX_SAFE_F64_INTEGER))
                .unwrap_or(true)
        };

        if !integer_is_safe(a) || !integer_is_safe(b) {
            return false;
        }

        match (a.as_f64(), b.as_f64()) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
    }

    /// Replace metadata tags on a stored scan. Kept for compatibility; user
    /// labels are stored independently by `update_label`.
    pub fn update_tags(&self, scan_id: &str, tags: Vec<String>) -> Result<(), String> {
        let mut scan = self.load_scan(scan_id)?;
        scan.tags = tags;
        self.encrypted_storage
            .store(&scan.id, &scan)
            .map_err(|e| DiagError::storage("update_tags", e.to_string()))?;
        self.upsert_summary(Self::summary_for(&scan))
    }

    pub fn update_label(&self, scan_id: &str, label: Option<String>) -> Result<(), String> {
        let mut scan = self.load_scan(scan_id)?;
        scan.label = label
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.encrypted_storage
            .store(&scan.id, &scan)
            .map_err(|e| DiagError::storage("update_label", e.to_string()))?;
        self.upsert_summary(Self::summary_for(&scan))
    }

    /// Per-task failure counts over the newest `limit` scans, for trend
    /// badges ("failed in N of last M scans"). Only tasks that failed at
    /// least once are returned.
    pub fn task_failure_trends(&self, limit: usize) -> Result<Vec<TaskTrend>, String> {
        let summaries = self.list_scans()?; // newest first
        let mut failed: HashMap<String, usize> = HashMap::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut scans_used = 0usize;

        for summary in summaries.iter().take(limit) {
            let Ok(scan) = self.load_scan(&summary.id) else {
                continue; // corrupt/unreadable scans don't poison the trend
            };
            scans_used += 1;
            for (task_id, result) in &scan.results {
                *seen.entry(task_id.clone()).or_default() += 1;
                if !result.success {
                    *failed.entry(task_id.clone()).or_default() += 1;
                }
            }
        }

        let mut trends: Vec<TaskTrend> = failed
            .into_iter()
            .map(|(task_id, fail_count)| TaskTrend {
                seen_in: *seen.get(&task_id).unwrap_or(&fail_count),
                scans_considered: scans_used,
                failed: fail_count,
                task_id,
            })
            .collect();
        trends.sort_by(|a, b| b.failed.cmp(&a.failed).then(a.task_id.cmp(&b.task_id)));
        Ok(trends)
    }

    fn cleanup_old_scans(&self) -> Result<(), String> {
        let summaries = self.list_scans()?;
        let (_, configured_limit) = crate::commands::settings::history_retention();
        let max_scans = configured_limit.clamp(1, 500) as usize;

        if summaries.len() > max_scans {
            println!(
                "Cleaning up old encrypted scans: {} total, keeping {}",
                summaries.len(),
                max_scans
            );

            // Remove oldest scans (summaries are already sorted newest first)
            let mut failed_deletions = Vec::new();
            for summary in summaries.iter().skip(max_scans) {
                if let Err(e) = self.encrypted_storage.delete(&summary.id) {
                    println!(
                        "Warning: Failed to remove old encrypted scan '{}': {}",
                        summary.id, e
                    );
                    // Keep the summary visible so a later cleanup can retry;
                    // dropping it from the index would strand an encrypted
                    // file that users could no longer see or clear normally.
                    failed_deletions.push(summary.clone());
                } else {
                    println!("Removed old encrypted scan: {}", summary.id);
                }
            }

            let mut retained: Vec<ScanSummary> = summaries.into_iter().take(max_scans).collect();
            retained.extend(failed_deletions);
            self.encrypted_storage
                .store(SUMMARY_INDEX_ID, &retained)
                .map_err(|error| DiagError::storage("trim_summary_index", error.to_string()))?;
        }

        Ok(())
    }

    pub fn clear_history(&self) -> Result<(), String> {
        println!("Clearing all scan history...");

        // Get list of all scan files (returns filenames without .enc extension)
        let scan_files = self
            .encrypted_storage
            .list_files()
            .map_err(|e| format!("Failed to list scan files: {}", e))?;

        let mut deleted_count = 0;
        let mut failed_count = 0;

        // Delete each scan - list_files returns the base filename (without .enc)
        for scan_file in scan_files {
            match self.encrypted_storage.delete(&scan_file) {
                Ok(_) => {
                    deleted_count += 1;
                    println!("Deleted scan: {}", scan_file);
                }
                Err(e) => {
                    failed_count += 1;
                    println!("Warning: Failed to delete scan '{}': {}", scan_file, e);
                }
            }
        }

        println!(
            "Clear history completed: {} deleted, {} failed",
            deleted_count, failed_count
        );

        if failed_count > 0 {
            Err(DiagError::storage(
                "clear_history",
                format!(
                    "Cleared {} scans, but {} failed to delete",
                    deleted_count, failed_count
                ),
            )
            .into())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(success: bool, output: &str) -> TaskResult {
        TaskResult {
            success,
            output: output.to_string(),
            error: None,
            duration_ms: 5,
        }
    }

    fn scan(id: &str, results: Vec<(&str, TaskResult)>) -> ScanRecord {
        let results: HashMap<String, TaskResult> = results
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let success_count = results.values().filter(|r| r.success).count();
        ScanRecord {
            id: id.to_string(),
            timestamp: Timestamp::from_iso_string("2026-06-12T10:00:00Z").unwrap(),
            computer_name: "TEST-PC".to_string(),
            os_version: "Windows 11".to_string(),
            is_admin: false,
            task_count: results.len(),
            success_count,
            failure_count: results.len() - success_count,
            duration_ms: 1000,
            label: None,
            tags: vec![],
            results,
        }
    }

    #[test]
    fn regression_lands_in_new_failures() {
        let current = scan("cur", vec![("os_info", result(false, "boom"))]);
        let previous = scan("prev", vec![("os_info", result(true, "fine"))]);
        let cmp = ScanStorage::compute_comparison(current, previous);
        assert_eq!(cmp.new_failures.len(), 1);
        assert_eq!(cmp.new_failures[0].task_id, "os_info");
        assert!(cmp.new_successes.is_empty());
        assert_eq!(cmp.total_changes, 1);
    }

    #[test]
    fn recovery_lands_in_new_successes() {
        let current = scan("cur", vec![("os_info", result(true, "fine"))]);
        let previous = scan("prev", vec![("os_info", result(false, "boom"))]);
        let cmp = ScanStorage::compute_comparison(current, previous);
        assert_eq!(cmp.new_successes.len(), 1);
        assert!(cmp.new_failures.is_empty());
        assert_eq!(cmp.total_changes, 1);
    }

    #[test]
    fn json_key_order_is_not_a_change() {
        let current = scan("cur", vec![("disk", result(true, r#"{"a": 1, "b": 2}"#))]);
        let previous = scan("prev", vec![("disk", result(true, r#"{"b": 2, "a": 1}"#))]);
        let cmp = ScanStorage::compute_comparison(current, previous);
        assert_eq!(cmp.total_changes, 0);
        assert_eq!(cmp.status_unchanged.len(), 1);
        assert!(!cmp.status_unchanged[0].output_changed);
    }

    #[test]
    fn real_output_change_is_counted() {
        let current = scan("cur", vec![("disk", result(true, r#"{"free": 10}"#))]);
        let previous = scan("prev", vec![("disk", result(true, r#"{"free": 90}"#))]);
        let cmp = ScanStorage::compute_comparison(current, previous);
        assert_eq!(cmp.total_changes, 1);
        assert!(cmp.status_unchanged[0].output_changed);
    }

    #[test]
    fn wmi_style_arrays_compare_by_stable_identity() {
        let current = scan(
            "cur",
            vec![(
                "services",
                result(
                    true,
                    r#"[{"Name":"Beta","State":"Running"},{"Name":"Alpha","State":"Stopped"}]"#,
                ),
            )],
        );
        let previous = scan(
            "prev",
            vec![(
                "services",
                result(
                    true,
                    r#"[{"Name":"Alpha","State":"Stopped"},{"Name":"Beta","State":"Running"}]"#,
                ),
            )],
        );
        let cmp = ScanStorage::compute_comparison(current, previous);
        assert_eq!(cmp.total_changes, 0);
    }

    #[test]
    fn volatile_performance_samples_do_not_create_drift() {
        let current = scan("cur", vec![("performance", result(true, r#"{"cpu":91}"#))]);
        let previous = scan("prev", vec![("performance", result(true, r#"{"cpu":2}"#))]);
        let cmp = ScanStorage::compute_comparison(current, previous);
        assert_eq!(cmp.total_changes, 0);
    }

    #[test]
    fn deferred_file_operation_churn_does_not_create_restart_drift() {
        let previous = r#"{"pending":false,"restart_required":false,"reasons":[],"summary":"2 deferred operations","deferred_file_operations":{"pending":true,"operation_count":2}}"#;
        let current = r#"{"pending":false,"restart_required":false,"reasons":[],"summary":"48 deferred operations","deferred_file_operations":{"pending":true,"operation_count":48}}"#;
        assert!(ScanStorage::outputs_are_equivalent_for_task(
            "pending_reboot",
            previous,
            current
        ));
    }

    #[test]
    fn authoritative_restart_source_changes_remain_visible() {
        let clear = r#"{"pending":false,"restart_required":false,"reasons":[]}"#;
        let update = r#"{"pending":true,"restart_required":true,"reasons":["windows_update"]}"#;
        let servicing =
            r#"{"pending":true,"restart_required":true,"reasons":["component_based_servicing"]}"#;
        assert!(!ScanStorage::outputs_are_equivalent_for_task(
            "pending_reboot",
            clear,
            update
        ));
        assert!(!ScanStorage::outputs_are_equivalent_for_task(
            "pending_reboot",
            update,
            servicing
        ));
    }

    #[test]
    fn non_intersecting_tasks_are_skipped() {
        // A task present in only one scan is a selection difference, not a
        // regression — it must not inflate new_failures/total_changes
        let current = scan("cur", vec![("only_in_current", result(true, "x"))]);
        let previous = scan("prev", vec![("only_in_previous", result(true, "y"))]);
        let cmp = ScanStorage::compute_comparison(current, previous);
        assert!(cmp.new_failures.is_empty());
        assert!(cmp.new_successes.is_empty());
        assert!(cmp.status_unchanged.is_empty());
        assert_eq!(cmp.total_changes, 0);
    }

    #[test]
    fn outputs_equivalence_rules() {
        // CRLF / surrounding whitespace normalize away for non-JSON
        assert!(ScanStorage::outputs_are_equivalent("a\r\nb", "a\nb"));
        assert!(ScanStorage::outputs_are_equivalent(" x ", "x"));
        // Numeric representation differences inside JSON are equal as floats
        assert!(ScanStorage::outputs_are_equivalent(
            r#"{"v": 1.0}"#,
            r#"{"v": 1}"#
        ));
        // Genuinely different values are a change
        assert!(!ScanStorage::outputs_are_equivalent(
            r#"{"v": 1}"#,
            r#"{"v": 2}"#
        ));
        assert!(!ScanStorage::outputs_are_equivalent(
            r#"{"bytes": 9007199254740992}"#,
            r#"{"bytes": 9007199254740993}"#
        ));
        assert!(!ScanStorage::outputs_are_equivalent("abc", "abd"));
    }

    #[test]
    fn scan_record_roundtrips_through_serde() {
        let original = scan(
            "rt",
            vec![
                ("os_info", result(true, "ok")),
                ("disk", result(false, "err")),
            ],
        );
        let json = serde_json::to_string(&original).unwrap();
        let restored: ScanRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, original.id);
        assert_eq!(restored.task_count, original.task_count);
        assert_eq!(restored.success_count, original.success_count);
        assert_eq!(restored.results.len(), original.results.len());
        assert!(!restored.results["disk"].success);
        assert_eq!(restored.results["os_info"].output, "ok");
    }
}
