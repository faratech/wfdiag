use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixResult {
    pub success: bool,
    pub message: String,
    pub actions_taken: Vec<String>,
    pub requires_restart: bool,
}

pub struct IssueFixer;

/// Issue IDs that have a real automated fix (must mirror the `fix_issue` match arms).
/// Exposed so the UI can show the "Fix Issue" button ONLY for issues we can actually
/// fix, instead of offering it for every detected issue and silently no-op'ing. Note
/// the detector emits `pending_windows_updates` (not in this list) while the fixer
/// handles `windows_update_service`; gating on this list correctly hides the button for
/// the former until that id mismatch is reconciled.
pub fn fixable_issue_ids() -> &'static [&'static str] {
    &[
        "disk_fragmentation",
        "temp_files",
        "windows_update_service",
        "dns_cache",
        "icon_cache",
        "prefetch_files",
        "recycle_bin",
        "stopped_services",
        "high_cpu_usage",
        "high_memory_usage",
    ]
}

impl IssueFixer {
    pub fn new() -> Self {
        Self
    }

    pub async fn fix_issue(&self, issue_id: &str) -> Result<FixResult> {
        match issue_id {
            "disk_fragmentation" => self.fix_disk_fragmentation().await,
            "temp_files" => self.fix_temp_files().await,
            "windows_update_service" => self.fix_windows_update_service().await,
            "dns_cache" => self.fix_dns_cache().await,
            "icon_cache" => self.fix_icon_cache().await,
            "prefetch_files" => self.fix_prefetch_files().await,
            "recycle_bin" => self.empty_recycle_bin().await,
            "stopped_services" => self.fix_stopped_services().await,
            "high_cpu_usage" => self.fix_high_cpu_usage().await,
            "high_memory_usage" => self.fix_high_memory_usage().await,
            _ => Ok(FixResult {
                success: false,
                message: format!("No automated fix available for issue: {}", issue_id),
                actions_taken: vec![],
                requires_restart: false,
            }),
        }
    }

    async fn fix_disk_fragmentation(&self) -> Result<FixResult> {
        let mut actions = Vec::new();

        // Launch Windows Defragment and Optimize Drives utility
        let output = Command::new("dfrgui.exe").spawn();

        match output {
            Ok(_) => {
                actions.push("Launched Windows Defragment and Optimize Drives utility".to_string());
                actions
                    .push("Please select the drive and click 'Optimize' to defragment".to_string());

                Ok(FixResult {
                    success: true,
                    message: "Windows Defragment and Optimize Drives utility has been launched. Administrator privileges are required to defragment drives.".to_string(),
                    actions_taken: actions,
                    requires_restart: false,
                })
            }
            Err(_) => {
                // Try alternative approach
                let alt_output = Command::new("cmd").args(["/c", "start", "dfrgui"]).spawn();

                if alt_output.is_ok() {
                    actions.push("Launched Windows Defragment utility".to_string());
                    Ok(FixResult {
                        success: true,
                        message: "Defragment utility has been launched. Administrator privileges required.".to_string(),
                        actions_taken: actions,
                        requires_restart: false,
                    })
                } else {
                    Ok(FixResult {
                        success: false,
                        message: "Failed to launch Defragment utility. Please run 'dfrgui' manually as Administrator.".to_string(),
                        actions_taken: vec![],
                        requires_restart: false,
                    })
                }
            }
        }
    }

    async fn fix_temp_files(&self) -> Result<FixResult> {
        let mut actions = Vec::new();

        // Launch Windows Disk Cleanup utility
        // This will open the disk cleanup tool for the user to run with proper permissions
        let output = Command::new("cleanmgr").args(["/sagerun:1"]).spawn();

        match output {
            Ok(_) => {
                actions.push("Launched Windows Disk Cleanup utility".to_string());
                actions.push("Please select the items you want to clean in the dialog".to_string());

                Ok(FixResult {
                    success: true,
                    message: "Windows Disk Cleanup has been launched. Please follow the prompts to clean temporary files. Note: Administrator privileges may be required for some cleanup operations.".to_string(),
                    actions_taken: actions,
                    requires_restart: false,
                })
            }
            Err(_) => {
                // Try alternative approach - open disk cleanup with default settings
                let alt_output = Command::new("cmd")
                    .args(["/c", "start", "cleanmgr"])
                    .spawn();

                if alt_output.is_ok() {
                    actions.push("Launched Windows Disk Cleanup utility".to_string());
                    Ok(FixResult {
                        success: true,
                        message: "Windows Disk Cleanup has been launched. Administrator privileges may be required for full cleanup.".to_string(),
                        actions_taken: actions,
                        requires_restart: false,
                    })
                } else {
                    Ok(FixResult {
                        success: false,
                        message: "Failed to launch Disk Cleanup. Please run 'cleanmgr' manually as Administrator.".to_string(),
                        actions_taken: vec![],
                        requires_restart: false,
                    })
                }
            }
        }
    }

    async fn fix_windows_update_service(&self) -> Result<FixResult> {
        let mut actions = Vec::new();

        // Stop Windows Update service
        let _ = Command::new("net").args(["stop", "wuauserv"]).output()?;
        actions.push("Stopped Windows Update service".to_string());

        // Clear update cache
        let update_cache = "C:\\Windows\\SoftwareDistribution\\Download";
        if std::path::Path::new(update_cache).exists() {
            let _ = std::fs::remove_dir_all(update_cache);
            let _ = std::fs::create_dir_all(update_cache);
            actions.push("Cleared Windows Update cache".to_string());
        }

        // Restart Windows Update service
        let output = Command::new("net").args(["start", "wuauserv"]).output()?;

        if output.status.success() {
            actions.push("Restarted Windows Update service".to_string());
            Ok(FixResult {
                success: true,
                message: "Windows Update service has been reset.".to_string(),
                actions_taken: actions,
                requires_restart: false,
            })
        } else {
            Ok(FixResult {
                success: false,
                message: "Failed to restart Windows Update service.".to_string(),
                actions_taken: actions,
                requires_restart: false,
            })
        }
    }

    async fn fix_dns_cache(&self) -> Result<FixResult> {
        let mut actions = Vec::new();

        let output = Command::new("ipconfig").args(["/flushdns"]).output()?;

        if output.status.success() {
            actions.push("Flushed DNS cache".to_string());
            Ok(FixResult {
                success: true,
                message: "DNS cache has been cleared successfully.".to_string(),
                actions_taken: actions,
                requires_restart: false,
            })
        } else {
            Ok(FixResult {
                success: false,
                message: "Failed to flush DNS cache.".to_string(),
                actions_taken: actions,
                requires_restart: false,
            })
        }
    }

    async fn fix_icon_cache(&self) -> Result<FixResult> {
        let mut actions = Vec::new();

        // Delete icon cache files
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        let icon_cache_path = format!("{}\\AppData\\Local\\IconCache.db", home);

        if std::path::Path::new(&icon_cache_path).exists() {
            let _ = std::fs::remove_file(&icon_cache_path);
            actions.push("Deleted icon cache".to_string());
        }

        // Delete thumbnail cache
        let thumb_cache_path = format!("{}\\AppData\\Local\\Microsoft\\Windows\\Explorer", home);
        if let Ok(entries) = std::fs::read_dir(&thumb_cache_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("thumbcache"))
                {
                    let _ = std::fs::remove_file(&path);
                }
            }
            actions.push("Cleared thumbnail cache".to_string());
        }

        Ok(FixResult {
            success: true,
            message: "Icon and thumbnail caches have been cleared. Explorer will rebuild them automatically.".to_string(),
            actions_taken: actions,
            requires_restart: true,
        })
    }

    async fn fix_prefetch_files(&self) -> Result<FixResult> {
        let mut actions = Vec::new();

        let prefetch_path = "C:\\Windows\\Prefetch";
        if let Ok(entries) = std::fs::read_dir(prefetch_path) {
            let mut count = 0;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "pf") {
                    let _ = std::fs::remove_file(&path);
                    count += 1;
                }
            }
            actions.push(format!("Deleted {} prefetch files", count));
        }

        Ok(FixResult {
            success: true,
            message: "Prefetch files have been cleaned.".to_string(),
            actions_taken: actions,
            requires_restart: false,
        })
    }

    async fn empty_recycle_bin(&self) -> Result<FixResult> {
        let mut actions = Vec::new();

        // Empty recycle bin using PowerShell
        let output = Command::new("powershell")
            .args([
                "-Command",
                "Clear-RecycleBin -Force -ErrorAction SilentlyContinue",
            ])
            .output()?;

        if output.status.success() {
            actions.push("Emptied Recycle Bin".to_string());
            Ok(FixResult {
                success: true,
                message: "Recycle Bin has been emptied.".to_string(),
                actions_taken: actions,
                requires_restart: false,
            })
        } else {
            Ok(FixResult {
                success: false,
                message: "Failed to empty Recycle Bin.".to_string(),
                actions_taken: actions,
                requires_restart: false,
            })
        }
    }

    async fn fix_stopped_services(&self) -> Result<FixResult> {
        let mut actions = Vec::new();
        let critical_services = vec![
            "wuauserv", // Windows Update
            "BITS",     // Background Intelligent Transfer
            "spooler",  // Print Spooler
            "Themes",   // Windows Themes
            "AudioSrv", // Windows Audio
        ];

        for service in critical_services {
            let output = Command::new("sc").args(["start", service]).output()?;

            if output.status.success() {
                actions.push(format!("Started {} service", service));
            }
        }

        Ok(FixResult {
            success: !actions.is_empty(),
            message: if actions.is_empty() {
                "No stopped critical services found.".to_string()
            } else {
                "Critical services have been restarted.".to_string()
            },
            actions_taken: actions,
            requires_restart: false,
        })
    }

    async fn fix_high_cpu_usage(&self) -> Result<FixResult> {
        let mut actions = Vec::new();

        // Launch Task Manager to let user identify and end high CPU processes
        let output = Command::new("taskmgr.exe").spawn();

        match output {
            Ok(_) => {
                actions.push("Launched Windows Task Manager".to_string());
                actions.push("Please identify and end processes with high CPU usage".to_string());

                Ok(FixResult {
                    success: true,
                    message: "Task Manager has been launched. Sort by CPU usage and end unnecessary high-usage processes.".to_string(),
                    actions_taken: actions,
                    requires_restart: false,
                })
            }
            Err(_) => {
                // Try alternative approach
                let alt_output = Command::new("cmd").args(["/c", "start", "taskmgr"]).spawn();

                if alt_output.is_ok() {
                    actions.push("Launched Task Manager".to_string());
                    Ok(FixResult {
                        success: true,
                        message: "Task Manager launched. Check CPU column to identify high-usage processes.".to_string(),
                        actions_taken: actions,
                        requires_restart: false,
                    })
                } else {
                    Ok(FixResult {
                        success: false,
                        message: "Failed to launch Task Manager. Press Ctrl+Shift+Esc to open it manually.".to_string(),
                        actions_taken: vec![],
                        requires_restart: false,
                    })
                }
            }
        }
    }

    async fn fix_high_memory_usage(&self) -> Result<FixResult> {
        // NOTE: Previously this ran `[System.GC]::Collect()` in a throwaway PowerShell
        // process (which only collects that process's own heap, freeing nothing in the
        // high-usage processes) and `echo off | clip` (which destructively wiped the
        // user's clipboard). Both were removed: they freed no memory and lost user data.
        // High memory usage requires the user to identify and close offending processes,
        // so we launch Task Manager for manual triage, mirroring fix_high_cpu_usage.
        let mut actions = Vec::new();
        let output = Command::new("taskmgr.exe").spawn();

        if output.is_ok() {
            actions.push("Launched Task Manager".to_string());
            Ok(FixResult {
                success: true,
                message: "Task Manager launched. Check the Memory column to identify and close high-usage processes.".to_string(),
                actions_taken: actions,
                requires_restart: false,
            })
        } else {
            Ok(FixResult {
                success: false,
                message: "Failed to launch Task Manager. Press Ctrl+Shift+Esc to open it manually and check memory usage.".to_string(),
                actions_taken: vec![],
                requires_restart: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixable_ids_have_no_duplicates() {
        let ids = fixable_issue_ids();
        let mut sorted = ids.to_vec();
        sorted.sort_unstable();
        let len_before = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), len_before);
    }

    #[tokio::test]
    async fn unknown_issue_id_is_a_safe_no_op() {
        // Must not run any command, must report failure honestly
        let result = IssueFixer::new()
            .fix_issue("definitely_not_real")
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.actions_taken.is_empty());
        assert!(result.message.contains("No automated fix"));
    }

    /// The "Fix" button only appears when the detector emits an id that is in
    /// fixable_issue_ids(). This pins the intersection so renames on either
    /// side fail loudly instead of silently orphaning a fix or a detection.
    #[test]
    fn fixer_and_detector_ids_stay_in_sync() {
        let detector_ids: Vec<String> = crate::issue_detector::IssueDetector::new()
            .detect_issues(&std::collections::HashMap::new())
            .into_iter()
            .map(|i| i.id)
            .collect();

        // Fixable ids the detector can actually surface
        let expected_active = [
            "disk_fragmentation",
            "temp_files",
            "dns_cache",
            "stopped_services",
            "high_cpu_usage",
            "high_memory_usage",
        ];
        // Fixable ids with NO corresponding detector today (manual-only fixes,
        // reachable only if a future detector emits them). windows_update_service
        // is the known id mismatch: the detector emits pending_windows_updates.
        let known_orphans = [
            "windows_update_service",
            "icon_cache",
            "prefetch_files",
            "recycle_bin",
        ];

        for id in fixable_issue_ids() {
            let detectable = detector_ids.iter().any(|d| d == id);
            if expected_active.contains(id) {
                assert!(
                    detectable,
                    "fixable id '{}' no longer emitted by the detector",
                    id
                );
            } else {
                assert!(
                    known_orphans.contains(id),
                    "new fixable id '{}' — classify it here",
                    id
                );
                assert!(
                    !detectable,
                    "'{}' is now detectable — move it to expected_active",
                    id
                );
            }
        }
    }
}
