//! Remediation engine: the vetted, tiered catalog of everything wfdiag can
//! run to fix a detected issue.
//!
//! Tiers:
//! - **OpenTool** — launches a Windows GUI (Task Manager, Disk Cleanup, …).
//! - **AutoSafe** — non-destructive commands/cleanups, one click.
//! - **Repair**  — admin and/or system-altering (DISM, SFC, network reset,
//!   restart). The backend REFUSES to run these without `confirmed = true`;
//!   the UI shows a confirmation modal first. This gate is enforced here, not
//!   in the frontend.
//!
//! SAFETY: every command is a compile-time constant — no user or model input
//! ever reaches an argv. The AI fix-plan path only ever yields catalog IDs,
//! which are validated against this table before the UI offers a Run button.

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixResult {
    pub success: bool,
    pub message: String,
    pub actions_taken: Vec<String>,
    pub requires_restart: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationTier {
    OpenTool,
    AutoSafe,
    Repair,
}

/// Wire form sent to the frontend (embedded in Issue + the Maintenance list)
/// and included in AI prompts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemediationSummary {
    pub id: String,
    pub label: String,
    pub description: String,
    pub tier: RemediationTier,
    pub admin_required: bool,
    pub requires_restart: bool,
    pub long_running: bool,
    pub maintenance: bool,
}

/// Outcome of run_remediation: either the confirmation gate fired, or the
/// remediation ran to completion (successfully or not).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FixOutcome {
    NeedsConfirmation { remediation: RemediationSummary },
    Completed { result: FixResult },
}

pub struct CmdStep {
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// Continue the sequence even if this step fails (e.g. stopping a service
    /// that is already stopped)
    pub ignore_failure: bool,
    pub action_label: &'static str,
}

pub enum RunKind {
    /// Fire-and-forget GUI launch
    Spawn {
        program: &'static str,
        args: &'static [&'static str],
    },
    /// Sequential commands, awaited with a timeout
    Steps {
        steps: &'static [CmdStep],
        timeout_secs: u64,
        success_msg: &'static str,
    },
    /// Filesystem-based cleanups that don't shell out
    Custom {
        f: fn() -> anyhow::Result<FixResult>,
    },
}

pub struct RemediationSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// EXACTLY what runs — shown verbatim in the confirm modal and AI prompts
    pub description: &'static str,
    pub tier: RemediationTier,
    pub admin_required: bool,
    pub requires_restart: bool,
    pub long_running: bool,
    /// Listed in the always-available Maintenance section
    pub maintenance: bool,
    pub run: RunKind,
}

impl RemediationSpec {
    pub fn summary(&self) -> RemediationSummary {
        RemediationSummary {
            id: self.id.to_string(),
            label: self.label.to_string(),
            description: self.description.to_string(),
            tier: self.tier,
            admin_required: self.admin_required,
            requires_restart: self.requires_restart,
            long_running: self.long_running,
            maintenance: self.maintenance,
        }
    }
}

// ============================================================================
// Command execution (injectable for tests)
// ============================================================================

pub struct CmdOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

type RunFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<CmdOutput>> + Send + 'a>>;

/// Runs commands. A trait so the confirm-gate and step-sequence tests can
/// assert exactly which commands would run without touching the system.
pub trait CommandRunner: Send + Sync {
    fn spawn(&self, program: &str, args: &[&str]) -> anyhow::Result<()>;
    fn run<'a>(&'a self, program: &'a str, args: &'a [&'a str], timeout: Duration)
    -> RunFuture<'a>;
}

/// Real runner: tokio process with hidden console windows and timeouts.
pub struct RealRunner;

impl CommandRunner for RealRunner {
    fn spawn(&self, program: &str, args: &[&str]) -> anyhow::Result<()> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            // GUI tools create their own windows; this only suppresses a
            // transient console host for console-subsystem launchers
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.spawn()?;
        Ok(())
    }

    fn run<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        timeout: Duration,
    ) -> RunFuture<'a> {
        Box::pin(async move {
            let mut cmd = tokio::process::Command::new(program);
            cmd.args(args);
            #[cfg(windows)]
            {
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            let output = tokio::time::timeout(timeout, cmd.output())
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "'{}' timed out after {} minute(s) — it may still be running",
                        program,
                        timeout.as_secs() / 60
                    )
                })??;
            #[cfg(windows)]
            let (stdout, stderr) = (
                crate::security::decode_windows_output(&output.stdout),
                crate::security::decode_windows_output(&output.stderr),
            );
            #[cfg(not(windows))]
            let (stdout, stderr) = (
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            );
            Ok(CmdOutput {
                success: output.status.success(),
                stdout,
                stderr,
            })
        })
    }
}

// ============================================================================
// The catalog
// ============================================================================

pub fn remediations() -> &'static [RemediationSpec] {
    &[
        // ---- OpenTool ----
        RemediationSpec {
            id: "open_defrag",
            label: "Open Optimize Drives",
            description: "Opens the Windows Optimize Drives tool (dfrgui.exe).",
            tier: RemediationTier::OpenTool,
            admin_required: false,
            requires_restart: false,
            long_running: false,
            maintenance: false,
            run: RunKind::Spawn {
                program: "dfrgui.exe",
                args: &[],
            },
        },
        RemediationSpec {
            id: "open_disk_cleanup",
            label: "Open Disk Cleanup",
            description: "Opens Windows Disk Cleanup (cleanmgr.exe) to pick what to remove.",
            tier: RemediationTier::OpenTool,
            admin_required: false,
            requires_restart: false,
            long_running: false,
            maintenance: false,
            run: RunKind::Spawn {
                program: "cleanmgr.exe",
                args: &[],
            },
        },
        RemediationSpec {
            id: "open_task_manager",
            label: "Open Task Manager",
            description: "Opens Task Manager (taskmgr.exe) to inspect processes and startup apps.",
            tier: RemediationTier::OpenTool,
            admin_required: false,
            requires_restart: false,
            long_running: false,
            maintenance: false,
            run: RunKind::Spawn {
                program: "taskmgr.exe",
                args: &[],
            },
        },
        RemediationSpec {
            id: "open_windows_update",
            label: "Open Windows Update",
            description: "Opens Settings > Windows Update (ms-settings:windowsupdate).",
            tier: RemediationTier::OpenTool,
            admin_required: false,
            requires_restart: false,
            long_running: false,
            maintenance: false,
            run: RunKind::Spawn {
                program: "explorer.exe",
                args: &["ms-settings:windowsupdate"],
            },
        },
        RemediationSpec {
            id: "open_security_center",
            label: "Open Windows Security",
            description: "Opens the Windows Security app (windowsdefender://).",
            tier: RemediationTier::OpenTool,
            admin_required: false,
            requires_restart: false,
            long_running: false,
            maintenance: false,
            run: RunKind::Spawn {
                program: "explorer.exe",
                args: &["windowsdefender://"],
            },
        },
        RemediationSpec {
            id: "open_device_manager",
            label: "Open Device Manager",
            description: "Opens Device Manager (devmgmt.msc) to inspect flagged devices.",
            tier: RemediationTier::OpenTool,
            admin_required: false,
            requires_restart: false,
            long_running: false,
            maintenance: false,
            run: RunKind::Spawn {
                program: "mmc.exe",
                args: &["devmgmt.msc"],
            },
        },
        RemediationSpec {
            id: "open_system_protection",
            label: "Open System Protection",
            description: "Opens System Protection settings (SystemPropertiesProtection.exe) to \
                          manage restore points.",
            tier: RemediationTier::OpenTool,
            admin_required: false,
            requires_restart: false,
            long_running: false,
            maintenance: false,
            run: RunKind::Spawn {
                program: "SystemPropertiesProtection.exe",
                args: &[],
            },
        },
        // ---- AutoSafe ----
        RemediationSpec {
            id: "flush_dns",
            label: "Flush DNS cache",
            description: "Runs 'ipconfig /flushdns' to clear cached DNS lookups.",
            tier: RemediationTier::AutoSafe,
            admin_required: false,
            requires_restart: false,
            long_running: false,
            maintenance: true,
            run: RunKind::Steps {
                steps: &[CmdStep {
                    program: "ipconfig",
                    args: &["/flushdns"],
                    ignore_failure: false,
                    action_label: "Flushed the DNS resolver cache",
                }],
                timeout_secs: 30,
                success_msg: "DNS cache flushed.",
            },
        },
        RemediationSpec {
            id: "clear_icon_cache",
            label: "Rebuild icon & thumbnail cache",
            description: "Deletes IconCache.db and Explorer thumbnail caches; they rebuild on \
                          next sign-in.",
            tier: RemediationTier::AutoSafe,
            admin_required: false,
            requires_restart: true,
            long_running: false,
            maintenance: true,
            run: RunKind::Custom {
                f: clear_icon_cache,
            },
        },
        RemediationSpec {
            id: "clear_prefetch",
            label: "Clear prefetch files",
            description: "Deletes .pf files from C:\\Windows\\Prefetch (Windows rebuilds them).",
            tier: RemediationTier::AutoSafe,
            admin_required: true,
            requires_restart: false,
            long_running: false,
            maintenance: true,
            run: RunKind::Custom { f: clear_prefetch },
        },
        RemediationSpec {
            id: "empty_recycle_bin",
            label: "Empty Recycle Bin",
            description: "Runs PowerShell Clear-RecycleBin -Force.",
            tier: RemediationTier::AutoSafe,
            admin_required: false,
            requires_restart: false,
            long_running: false,
            maintenance: true,
            run: RunKind::Steps {
                steps: &[CmdStep {
                    program: "powershell",
                    args: &[
                        "-NoProfile",
                        "-Command",
                        "Clear-RecycleBin -Force -ErrorAction SilentlyContinue",
                    ],
                    ignore_failure: false,
                    action_label: "Emptied the Recycle Bin",
                }],
                timeout_secs: 120,
                success_msg: "Recycle Bin emptied.",
            },
        },
        RemediationSpec {
            id: "clear_temp_files",
            label: "Clean temp files",
            description: "Best-effort delete of files in the user temp folder (locked files are \
                          skipped).",
            tier: RemediationTier::AutoSafe,
            admin_required: false,
            requires_restart: false,
            long_running: false,
            maintenance: true,
            run: RunKind::Custom {
                f: clear_temp_files,
            },
        },
        RemediationSpec {
            id: "start_critical_services",
            label: "Start stopped core services",
            description: "Runs 'sc start' for wuauserv, BITS, Spooler, Themes and AudioSrv.",
            tier: RemediationTier::AutoSafe,
            admin_required: true,
            requires_restart: false,
            long_running: false,
            maintenance: false,
            run: RunKind::Steps {
                steps: &[
                    CmdStep {
                        program: "sc",
                        args: &["start", "wuauserv"],
                        ignore_failure: true,
                        action_label: "Started wuauserv",
                    },
                    CmdStep {
                        program: "sc",
                        args: &["start", "BITS"],
                        ignore_failure: true,
                        action_label: "Started BITS",
                    },
                    CmdStep {
                        program: "sc",
                        args: &["start", "Spooler"],
                        ignore_failure: true,
                        action_label: "Started Spooler",
                    },
                    CmdStep {
                        program: "sc",
                        args: &["start", "Themes"],
                        ignore_failure: true,
                        action_label: "Started Themes",
                    },
                    CmdStep {
                        program: "sc",
                        args: &["start", "AudioSrv"],
                        ignore_failure: true,
                        action_label: "Started AudioSrv",
                    },
                ],
                timeout_secs: 120,
                success_msg: "Attempted to start the core services (already-running services \
                              are skipped).",
            },
        },
        // ---- Repair (confirm-gated) ----
        RemediationSpec {
            id: "windows_update_reset",
            label: "Reset Windows Update",
            description: "Stops the Windows Update service, clears the SoftwareDistribution \
                          download cache, and restarts the service.",
            tier: RemediationTier::Repair,
            admin_required: true,
            requires_restart: false,
            long_running: false,
            maintenance: true,
            run: RunKind::Custom {
                f: reset_windows_update,
            },
        },
        RemediationSpec {
            id: "dism_restorehealth",
            label: "Repair Windows image (DISM)",
            description: "Runs 'DISM /Online /Cleanup-Image /RestoreHealth' to repair the \
                          Windows component store. Can take 10-30 minutes.",
            tier: RemediationTier::Repair,
            admin_required: true,
            requires_restart: false,
            long_running: true,
            maintenance: true,
            run: RunKind::Steps {
                steps: &[CmdStep {
                    program: "dism",
                    args: &["/online", "/cleanup-image", "/restorehealth"],
                    ignore_failure: false,
                    action_label: "Ran DISM RestoreHealth",
                }],
                timeout_secs: 2700,
                success_msg: "DISM repair completed. Consider running System File Checker next.",
            },
        },
        RemediationSpec {
            id: "sfc_scannow",
            label: "System File Checker",
            description: "Runs 'sfc /scannow' to verify and repair protected system files. Can \
                          take 5-15 minutes.",
            tier: RemediationTier::Repair,
            admin_required: true,
            requires_restart: false,
            long_running: true,
            maintenance: true,
            run: RunKind::Steps {
                steps: &[CmdStep {
                    program: "sfc",
                    args: &["/scannow"],
                    ignore_failure: false,
                    action_label: "Ran System File Checker",
                }],
                timeout_secs: 1800,
                success_msg: "System File Checker completed.",
            },
        },
        RemediationSpec {
            id: "network_reset",
            label: "Reset network stack",
            description: "Runs 'netsh winsock reset' and 'netsh int ip reset'. Requires a \
                          restart to take effect.",
            tier: RemediationTier::Repair,
            admin_required: true,
            requires_restart: true,
            long_running: false,
            maintenance: true,
            run: RunKind::Steps {
                steps: &[
                    CmdStep {
                        program: "netsh",
                        args: &["winsock", "reset"],
                        ignore_failure: false,
                        action_label: "Reset Winsock catalog",
                    },
                    CmdStep {
                        program: "netsh",
                        args: &["int", "ip", "reset"],
                        ignore_failure: true, // partial resets still log success lines
                        action_label: "Reset TCP/IP stack",
                    },
                ],
                timeout_secs: 120,
                success_msg: "Network stack reset. Restart Windows to apply.",
            },
        },
        RemediationSpec {
            id: "restart_system",
            label: "Restart Windows (60s)",
            description: "Schedules a restart in 60 seconds via 'shutdown /r /t 60'. Cancel with \
                          'shutdown /a'.",
            tier: RemediationTier::Repair,
            admin_required: false,
            requires_restart: true,
            long_running: false,
            maintenance: false,
            run: RunKind::Steps {
                steps: &[CmdStep {
                    program: "shutdown",
                    args: &["/r", "/t", "60"],
                    ignore_failure: false,
                    action_label: "Scheduled a restart in 60 seconds",
                }],
                timeout_secs: 30,
                success_msg: "Restart scheduled for 60 seconds from now. Run 'shutdown /a' to \
                              cancel.",
            },
        },
    ]
}

pub fn find(remediation_id: &str) -> Option<&'static RemediationSpec> {
    remediations().iter().find(|r| r.id == remediation_id)
}

/// Wire summary for an id (used by issue_catalog to embed in Issues).
pub fn summary(remediation_id: &str) -> Option<RemediationSummary> {
    find(remediation_id).map(|spec| spec.summary())
}

// ============================================================================
// Engine
// ============================================================================

/// Execute a remediation. The Repair confirmation gate lives HERE: a Repair
/// tier id without `confirmed` returns NeedsConfirmation before any command
/// is constructed or run.
pub async fn execute(
    remediation_id: &str,
    confirmed: bool,
    runner: &dyn CommandRunner,
) -> Result<FixOutcome, String> {
    let spec =
        find(remediation_id).ok_or_else(|| format!("Unknown remediation '{}'", remediation_id))?;

    if spec.tier == RemediationTier::Repair && !confirmed {
        return Ok(FixOutcome::NeedsConfirmation {
            remediation: spec.summary(),
        });
    }

    let result = match &spec.run {
        RunKind::Spawn { program, args } => match runner.spawn(program, args) {
            Ok(()) => FixResult {
                success: true,
                message: format!("{} opened.", spec.label),
                actions_taken: vec![format!("Launched {}", program)],
                requires_restart: false,
            },
            Err(e) => FixResult {
                success: false,
                message: format!("Could not launch {}: {}", program, e),
                actions_taken: vec![],
                requires_restart: false,
            },
        },
        RunKind::Steps {
            steps,
            timeout_secs,
            success_msg,
        } => {
            let mut actions_taken = Vec::new();
            let timeout = Duration::from_secs(*timeout_secs);
            let mut failure: Option<String> = None;
            for step in *steps {
                match runner.run(step.program, step.args, timeout).await {
                    Ok(output) if output.success => {
                        actions_taken.push(step.action_label.to_string());
                    }
                    Ok(output) => {
                        if step.ignore_failure {
                            continue;
                        }
                        let detail = if output.stderr.trim().is_empty() {
                            output.stdout
                        } else {
                            output.stderr
                        };
                        failure = Some(format!(
                            "'{} {}' failed: {}",
                            step.program,
                            step.args.join(" "),
                            detail.trim().chars().take(400).collect::<String>()
                        ));
                        break;
                    }
                    Err(e) => {
                        if step.ignore_failure {
                            continue;
                        }
                        failure = Some(e.to_string());
                        break;
                    }
                }
            }
            match failure {
                Some(message) => FixResult {
                    success: false,
                    message,
                    actions_taken,
                    requires_restart: false,
                },
                None => FixResult {
                    success: true,
                    message: success_msg.to_string(),
                    actions_taken,
                    requires_restart: spec.requires_restart,
                },
            }
        }
        RunKind::Custom { f } => match f() {
            Ok(result) => result,
            Err(e) => FixResult {
                success: false,
                message: e.to_string(),
                actions_taken: vec![],
                requires_restart: false,
            },
        },
    };

    Ok(FixOutcome::Completed { result })
}

// ============================================================================
// Custom (filesystem) remediations
// ============================================================================

fn clear_icon_cache() -> anyhow::Result<FixResult> {
    let mut actions_taken = Vec::new();
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let icon_cache = std::path::Path::new(&profile).join(r"AppData\Local\IconCache.db");
        if icon_cache.exists() && std::fs::remove_file(&icon_cache).is_ok() {
            actions_taken.push("Deleted IconCache.db".to_string());
        }
        let explorer_dir =
            std::path::Path::new(&profile).join(r"AppData\Local\Microsoft\Windows\Explorer");
        if let Ok(entries) = std::fs::read_dir(&explorer_dir) {
            let mut removed = 0;
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.starts_with("thumbcache") && std::fs::remove_file(entry.path()).is_ok() {
                    removed += 1;
                }
            }
            if removed > 0 {
                actions_taken.push(format!("Deleted {} thumbnail cache file(s)", removed));
            }
        }
    }
    Ok(FixResult {
        success: true,
        message: "Icon and thumbnail caches cleared. They rebuild after you sign in again."
            .to_string(),
        actions_taken,
        requires_restart: true,
    })
}

fn clear_prefetch() -> anyhow::Result<FixResult> {
    let prefetch = std::path::Path::new(r"C:\Windows\Prefetch");
    let mut removed = 0;
    let mut denied = false;
    if let Ok(entries) = std::fs::read_dir(prefetch) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "pf") {
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => removed += 1,
                    Err(_) => denied = true,
                }
            }
        }
    } else {
        denied = true;
    }
    if removed == 0 && denied {
        return Ok(FixResult {
            success: false,
            message: "Could not clear prefetch files — administrator rights are required."
                .to_string(),
            actions_taken: vec![],
            requires_restart: false,
        });
    }
    Ok(FixResult {
        success: true,
        message: format!("Removed {} prefetch file(s).", removed),
        actions_taken: vec![format!("Deleted {} .pf files", removed)],
        requires_restart: false,
    })
}

fn clear_temp_files() -> anyhow::Result<FixResult> {
    let temp = std::env::temp_dir();
    let mut removed = 0u32;
    let mut skipped = 0u32;
    if let Ok(entries) = std::fs::read_dir(&temp) {
        for entry in entries.flatten() {
            let path = entry.path();
            let outcome = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match outcome {
                Ok(()) => removed += 1,
                Err(_) => skipped += 1, // locked by running apps — expected
            }
        }
    }
    Ok(FixResult {
        success: true,
        message: format!(
            "Removed {} temp item(s); {} in use were skipped.",
            removed, skipped
        ),
        actions_taken: vec![format!("Cleaned {}", temp.display())],
        requires_restart: false,
    })
}

fn reset_windows_update() -> anyhow::Result<FixResult> {
    // Stop -> clear download cache -> start. Sync commands are fine here: the
    // service operations take seconds and this runs on a blocking-ok path.
    let mut actions_taken = Vec::new();

    let run_quiet = |program: &str, args: &[&str]| -> anyhow::Result<bool> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        Ok(cmd.output()?.status.success())
    };

    if run_quiet("net", &["stop", "wuauserv"])? {
        actions_taken.push("Stopped Windows Update service".to_string());
    }
    let download_dir = std::path::Path::new(r"C:\Windows\SoftwareDistribution\Download");
    if let Ok(entries) = std::fs::read_dir(download_dir) {
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let ok = if path.is_dir() {
                std::fs::remove_dir_all(&path).is_ok()
            } else {
                std::fs::remove_file(&path).is_ok()
            };
            if ok {
                removed += 1;
            }
        }
        actions_taken.push(format!("Cleared {} item(s) from the update cache", removed));
    }
    let restarted = run_quiet("net", &["start", "wuauserv"])?;
    if restarted {
        actions_taken.push("Restarted Windows Update service".to_string());
    }

    Ok(FixResult {
        success: restarted,
        message: if restarted {
            "Windows Update components reset.".to_string()
        } else {
            "Could not restart the Windows Update service — administrator rights are required."
                .to_string()
        },
        actions_taken,
        requires_restart: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records every call; never touches the system.
    #[derive(Default)]
    struct RecordingRunner {
        calls: Mutex<Vec<String>>,
        fail_on: Option<&'static str>,
    }

    impl CommandRunner for RecordingRunner {
        fn spawn(&self, program: &str, args: &[&str]) -> anyhow::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("spawn:{} {}", program, args.join(" ")));
            Ok(())
        }
        fn run<'a>(
            &'a self,
            program: &'a str,
            args: &'a [&'a str],
            _timeout: Duration,
        ) -> RunFuture<'a> {
            let line = format!("run:{} {}", program, args.join(" "));
            self.calls.lock().unwrap().push(line);
            let fail = self.fail_on.is_some_and(|f| program == f);
            Box::pin(async move {
                Ok(CmdOutput {
                    success: !fail,
                    stdout: String::new(),
                    stderr: if fail { "boom".into() } else { String::new() },
                })
            })
        }
    }

    #[test]
    fn catalog_invariants() {
        let mut ids: Vec<&str> = remediations().iter().map(|r| r.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate remediation ids");

        for spec in remediations() {
            assert!(!spec.label.is_empty(), "{} missing label", spec.id);
            assert!(
                !spec.description.is_empty(),
                "{} missing description",
                spec.id
            );
        }

        // Every issue's remediation_id resolves to a catalog entry
        for issue in crate::issue_catalog::catalog() {
            if let Some(remediation_id) = issue.remediation_id {
                assert!(
                    find(remediation_id).is_some(),
                    "issue '{}' references unknown remediation '{}'",
                    issue.id,
                    remediation_id
                );
            }
        }
    }

    #[tokio::test]
    async fn repair_tier_without_confirmation_runs_nothing() {
        let runner = RecordingRunner::default();
        let outcome = execute("sfc_scannow", false, &runner).await.unwrap();
        match outcome {
            FixOutcome::NeedsConfirmation { remediation } => {
                assert_eq!(remediation.id, "sfc_scannow");
                assert_eq!(remediation.tier, RemediationTier::Repair);
            }
            FixOutcome::Completed { .. } => panic!("repair ran without confirmation"),
        }
        // The strongest guarantee: zero commands were even constructed
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn repair_tier_with_confirmation_runs_steps_in_order() {
        let runner = RecordingRunner::default();
        let outcome = execute("network_reset", true, &runner).await.unwrap();
        let FixOutcome::Completed { result } = outcome else {
            panic!("expected completion")
        };
        assert!(result.success);
        assert!(result.requires_restart);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(
            *calls,
            vec![
                "run:netsh winsock reset".to_string(),
                "run:netsh int ip reset".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn auto_safe_ignores_the_confirmed_flag() {
        let runner = RecordingRunner::default();
        let outcome = execute("flush_dns", false, &runner).await.unwrap();
        assert!(matches!(outcome, FixOutcome::Completed { .. }));
        assert_eq!(
            *runner.calls.lock().unwrap(),
            vec!["run:ipconfig /flushdns".to_string()]
        );
    }

    #[tokio::test]
    async fn open_tool_spawns_without_waiting() {
        let runner = RecordingRunner::default();
        let outcome = execute("open_task_manager", false, &runner).await.unwrap();
        let FixOutcome::Completed { result } = outcome else {
            panic!()
        };
        assert!(result.success);
        assert_eq!(
            *runner.calls.lock().unwrap(),
            vec!["spawn:taskmgr.exe ".to_string()]
        );
    }

    #[tokio::test]
    async fn failing_step_stops_the_sequence_and_reports() {
        let runner = RecordingRunner {
            fail_on: Some("netsh"),
            ..Default::default()
        };
        let outcome = execute("network_reset", true, &runner).await.unwrap();
        let FixOutcome::Completed { result } = outcome else {
            panic!()
        };
        assert!(!result.success);
        assert!(result.message.contains("netsh winsock reset"));
        // First step failed (not ignore_failure) => second never ran
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unknown_remediation_is_an_error() {
        let runner = RecordingRunner::default();
        assert!(execute("nuke_everything", true, &runner).await.is_err());
        assert!(runner.calls.lock().unwrap().is_empty());
    }
}
