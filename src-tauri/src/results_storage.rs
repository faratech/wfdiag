use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use crate::diagnostics::TaskResult;
use crate::encrypted_storage::EncryptedStorage;
use crate::error::DiagError;
use crate::timestamp::Timestamp;

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
    max_scans: usize,
    encrypted_storage: EncryptedStorage,
}

impl ScanStorage {
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
            max_scans: 50,
            encrypted_storage,
        })
    }
    
    fn get_storage_directory() -> Result<PathBuf, String> {
        let app_data = std::env::var("APPDATA")
            .map_err(|_| DiagError::internal("Failed to get APPDATA directory"))?;

        Ok(PathBuf::from(app_data)
            .join("wfdiag-tauri")
            .join("scans"))
    }
    
    pub fn save_scan(&self, scan: &ScanRecord) -> Result<(), String> {
        println!("Saving encrypted scan {}", scan.id);

        // Validate scan data
        if scan.id.is_empty() {
            return Err(DiagError::storage("save_scan", "Scan ID cannot be empty").into());
        }

        if scan.results.is_empty() {
            return Err(DiagError::storage("save_scan", "Scan must have at least one result").into());
        }

        // Store using encrypted storage
        self.encrypted_storage.store(&scan.id, scan)
            .map_err(|e| DiagError::storage("save_encrypted_scan", e.to_string()))?;
        
        println!("Successfully saved encrypted scan {}", scan.id);
        
        // Clean up old scans
        self.cleanup_old_scans()?;
        
        Ok(())
    }
    
    pub fn load_scan(&self, scan_id: &str) -> Result<ScanRecord, String> {
        if !self.encrypted_storage.exists(scan_id) {
            return Err(DiagError::storage("load_scan", format!("Encrypted scan '{}' not found", scan_id)).into());
        }

        println!("Loading encrypted scan {}", scan_id);

        let scan: ScanRecord = self.encrypted_storage.load(scan_id)
            .map_err(|e| DiagError::storage("load_encrypted_scan", e.to_string()))?;

        // Validate loaded scan
        if scan.id != scan_id {
            return Err(DiagError::SessionMismatch {
                expected: scan_id.to_string(),
                actual: scan.id.clone(),
            }
            .into());
        }
        
        println!("Successfully loaded encrypted scan {} with {} results", scan.id, scan.results.len());
        
        Ok(scan)
    }
    
    pub fn list_scans(&self) -> Result<Vec<ScanSummary>, String> {
        let mut summaries = Vec::new();

        println!("Listing encrypted scans");

        let scan_ids = self.encrypted_storage.list_files()
            .map_err(|e| DiagError::storage("list_scans", e.to_string()))?;
        
        for scan_id in scan_ids {
            println!("Found encrypted scan: {}", scan_id);
            
            match self.load_scan(&scan_id) {
                Ok(scan) => {
                    summaries.push(ScanSummary {
                        id: scan.id,
                        timestamp: scan.timestamp,
                        computer_name: scan.computer_name,
                        task_count: scan.task_count,
                        success_count: scan.success_count,
                        failure_count: scan.failure_count,
                        duration_ms: scan.duration_ms,
                        tags: scan.tags,
                    });
                }
                Err(e) => {
                    println!("Warning: Failed to load scan '{}': {}", scan_id, e);
                    // Continue loading other scans instead of failing completely
                }
            }
        }
        
        // Sort by timestamp (newest first)
        summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        
        println!("Successfully loaded {} scan summaries", summaries.len());
        
        Ok(summaries)
    }
    
    pub fn compare_scans(&self, current_id: &str, previous_id: &str) -> Result<ComparisonResult, String> {
        println!("Comparing scans: '{}' vs '{}'", current_id, previous_id);

        if current_id == previous_id {
            return Err(DiagError::storage("compare_scans", "Cannot compare scan with itself").into());
        }
        
        // Load both scans
        let current = self.load_scan(current_id)?;
        let previous = self.load_scan(previous_id)?;
        
        println!("Loaded current scan: {} results", current.results.len());
        println!("Loaded previous scan: {} results", previous.results.len());
        
        // Get task metadata for proper names and categories
        let all_tasks = crate::diagnostics::get_all_tasks();
        let task_map: HashMap<String, &crate::diagnostics::DiagnosticTask> = all_tasks
            .iter()
            .map(|t| (t.id.clone(), t))
            .collect();
        
        let mut new_failures = Vec::new();
        let mut new_successes = Vec::new();
        let mut status_unchanged = Vec::new();
        
        // Find all tasks that exist in either scan
        let mut all_task_ids: std::collections::HashSet<String> = current.results.keys().cloned().collect();
        all_task_ids.extend(previous.results.keys().cloned());
        
        for task_id in all_task_ids {
            let current_result = current.results.get(&task_id);
            let previous_result = previous.results.get(&task_id);
            
            let task_info = task_map.get(&task_id);
            let task_name = task_info.map(|t| t.name.clone()).unwrap_or_else(|| task_id.clone());
            let category = task_info.map(|t| t.category.clone()).unwrap_or_else(|| "Unknown".to_string());
            
            match (current_result, previous_result) {
                (Some(current), Some(previous)) => {
                    // Task exists in both scans
                    // For JSON outputs, compare parsed data instead of raw strings
                    let output_changed = if current.success && previous.success {
                        !Self::outputs_are_equivalent(&current.output, &previous.output)
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
        
        let total_changes = new_failures.len() + new_successes.len() + 
                           status_unchanged.iter().filter(|c| c.output_changed).count();
        
        println!("Comparison complete: {} total changes, {} new failures, {} new successes", 
                total_changes, new_failures.len(), new_successes.len());
        
        Ok(ComparisonResult {
            current_scan: ScanSummary {
                id: current.id,
                timestamp: current.timestamp,
                computer_name: current.computer_name,
                task_count: current.task_count,
                success_count: current.success_count,
                failure_count: current.failure_count,
                duration_ms: current.duration_ms,
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
                tags: previous.tags,
            },
            total_changes,
            new_failures,
            new_successes,
            status_unchanged,
        })
    }
    
    /// Compare two outputs intelligently, parsing JSON when possible
    fn outputs_are_equivalent(output1: &str, output2: &str) -> bool {
        // First, try quick string comparison
        if output1 == output2 {
            return true;
        }
        
        // Try to parse both as JSON for deep comparison
        if let (Ok(json1), Ok(json2)) = (
            serde_json::from_str::<serde_json::Value>(output1),
            serde_json::from_str::<serde_json::Value>(output2)
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
    
    /// Deep comparison of JSON values, ignoring formatting and key order
    fn json_values_equal(val1: &serde_json::Value, val2: &serde_json::Value) -> bool {
        use serde_json::Value;
        
        match (val1, val2) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => {
                // Compare numbers as floats to handle precision differences
                a.as_f64() == b.as_f64()
            }
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                // Compare array elements
                for (item1, item2) in a.iter().zip(b.iter()) {
                    if !Self::json_values_equal(item1, item2) {
                        return false;
                    }
                }
                true
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
    
    fn cleanup_old_scans(&self) -> Result<(), String> {
        let summaries = self.list_scans()?;
        
        if summaries.len() > self.max_scans {
            println!("Cleaning up old encrypted scans: {} total, keeping {}", summaries.len(), self.max_scans);
            
            // Remove oldest scans (summaries are already sorted newest first)
            for summary in summaries.iter().skip(self.max_scans) {
                if let Err(e) = self.encrypted_storage.delete(&summary.id) {
                    println!("Warning: Failed to remove old encrypted scan '{}': {}", summary.id, e);
                } else {
                    println!("Removed old encrypted scan: {}", summary.id);
                }
            }
        }
        
        Ok(())
    }

    pub fn clear_history(&self) -> Result<(), String> {
        println!("Clearing all scan history...");

        // Get list of all scan files (returns filenames without .enc extension)
        let scan_files = self.encrypted_storage.list_files()
            .map_err(|e| format!("Failed to list scan files: {}", e))?;

        let mut deleted_count = 0;
        let mut failed_count = 0;

        // Delete each scan - list_files returns the base filename (without .enc)
        for scan_file in scan_files {
            // Skip if not a scan file (we only want to delete scan_* files)
            if !scan_file.starts_with("scan_") {
                continue;
            }

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

        println!("Clear history completed: {} deleted, {} failed", deleted_count, failed_count);

        if failed_count > 0 {
            Err(DiagError::storage(
                "clear_history",
                format!("Cleared {} scans, but {} failed to delete", deleted_count, failed_count),
            )
            .into())
        } else {
            Ok(())
        }
    }
}