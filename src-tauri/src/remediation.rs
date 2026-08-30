//! Remediation engine: the vetted, tiered catalog of everything wfdiag can
//! run to fix a detected issue.
//!
//! Tiers:
//! - **OpenTool** — launches a Windows GUI (Task Manager, Disk Cleanup, …).
//! - **AutoSafe** — non-destructive commands/cleanups, one click.
//! - **Repair**  — admin and/or system-altering (DISM, SFC, network reset,
//!   restart). Production execution is reachable only after the action broker
//!   atomically consumes an unexpired, exact catalog proposal approved by the
//!   user. The webview cannot submit an execution boolean or an argv.
//!
//! SAFETY: every command is a compile-time constant — no user or model input
//! ever reaches an argv. The AI fix-plan path only ever yields catalog IDs,
//! which are validated against this table before the UI offers a Run button.

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wfdiag_remediation_catalog as remediation_catalog;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixResult {
    /// Compatibility summary: true only when every required step succeeded
    /// or was already satisfied.
    pub success: bool,
    pub message: String,
    pub actions_taken: Vec<String>,
    pub requires_restart: bool,
    #[serde(default)]
    pub completion_status: FixCompletionStatus,
    #[serde(default)]
    pub steps: Vec<RemediationStepResult>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixCompletionStatus {
    #[default]
    Succeeded,
    Partial,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationStepStatus {
    Succeeded,
    AlreadySatisfied,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemediationStepResult {
    pub action: String,
    pub status: RemediationStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Wire forms shared with native issue detection. Execution remains in this
/// module and is never exposed by the portable metadata crate.
pub use wfdiag_remediation_catalog::{RemediationMetadata, RemediationSummary, RemediationTier};

/// Test-only shape for the removed boolean confirmation path. Keeping this in
/// regression tests proves repair commands are never constructed pre-approval
/// without carrying that legacy shape into production IPC.
#[cfg(test)]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum FixOutcome {
    NeedsConfirmation { remediation: RemediationSummary },
    Completed { result: FixResult },
}

pub struct CmdStep {
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// Continue the sequence even if this step fails. The failure is still
    /// returned to the caller and makes the overall result partial/failed.
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
        f: fn(&CancellationToken) -> anyhow::Result<FixResult>,
    },
}

pub struct RemediationSpec {
    /// Shared immutable UI/issue metadata. Execution-only data stays below.
    pub metadata: &'static RemediationMetadata,
    pub run: RunKind,
}

impl Deref for RemediationSpec {
    type Target = RemediationMetadata;

    fn deref(&self) -> &Self::Target {
        self.metadata
    }
}

impl RemediationSpec {
    pub fn batch_eligible(&self) -> bool {
        self.metadata.batch_eligible()
    }

    pub fn cancellable(&self) -> bool {
        match &self.run {
            // Do not terminate integrity-repair tools midway through a write.
            RunKind::Steps { .. } => !self.long_running && self.id != "restart_system",
            RunKind::Custom { .. } => matches!(self.id, "clear_icon_cache" | "clear_temp_files"),
            RunKind::Spawn { .. } => false,
        }
    }

    /// Human-facing, immutable preview derived only from catalog constants.
    pub fn preview_steps(&self) -> Vec<String> {
        match &self.run {
            RunKind::Spawn { program, args } => vec![format!(
                "Open {}{}",
                program,
                if !args.is_empty() {
                    format!(" {}", args.join(" "))
                } else {
                    String::new()
                }
            )],
            RunKind::Steps { steps, .. } => steps
                .iter()
                .map(|step| {
                    format!("{} {}", step.program, step.args.join(" "))
                        .trim()
                        .to_string()
                })
                .collect(),
            RunKind::Custom { .. } => vec![self.description.to_string()],
        }
    }

    pub fn summary(&self) -> RemediationSummary {
        let summary = self.metadata.summary();
        debug_assert_eq!(summary.batch_eligible, self.batch_eligible());
        debug_assert_eq!(summary.cancellable, self.cancellable());
        summary
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
    fn run<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
        timeout: Duration,
        cancel: &'a CancellationToken,
    ) -> RunFuture<'a>;
}

/// Real runner: tokio process with hidden console windows and timeouts.
pub struct RealRunner;

impl CommandRunner for RealRunner {
    fn spawn(&self, program: &str, args: &[&str]) -> anyhow::Result<()> {
        let mut cmd = std::process::Command::new(crate::security::trusted_system_program(program)?);
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
        cancel: &'a CancellationToken,
    ) -> RunFuture<'a> {
        Box::pin(async move {
            let mut cmd =
                tokio::process::Command::new(crate::security::trusted_system_program(program)?);
            cmd.args(args);
            #[cfg(windows)]
            {
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            cmd.kill_on_drop(true);
            let child = cmd.spawn()?;
            let output = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    return Err(anyhow::anyhow!("Remediation cancelled"));
                }
                result = tokio::time::timeout(timeout, child.wait_with_output()) => {
                    result.map_err(|_| {
                        anyhow::anyhow!(
                            "'{}' timed out after {} minute(s) and was stopped",
                            program,
                            timeout.as_secs() / 60
                        )
                    })??
                }
            };
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
            metadata: remediation_catalog::OPEN_DEFRAG,
            run: RunKind::Spawn {
                program: "dfrgui.exe",
                args: &[],
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::OPEN_DISK_CLEANUP,
            run: RunKind::Spawn {
                program: "cleanmgr.exe",
                args: &[],
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::OPEN_TASK_MANAGER,
            run: RunKind::Spawn {
                program: "taskmgr.exe",
                args: &[],
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::OPEN_WINDOWS_UPDATE,
            run: RunKind::Spawn {
                program: "explorer.exe",
                args: &["ms-settings:windowsupdate"],
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::OPEN_SECURITY_CENTER,
            run: RunKind::Spawn {
                program: "explorer.exe",
                args: &["windowsdefender://"],
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::OPEN_DEVICE_MANAGER,
            run: RunKind::Spawn {
                program: "mmc.exe",
                args: &["devmgmt.msc"],
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::OPEN_SYSTEM_PROTECTION,
            run: RunKind::Spawn {
                program: "SystemPropertiesProtection.exe",
                args: &[],
            },
        },
        // ---- AutoSafe ----
        RemediationSpec {
            metadata: remediation_catalog::FLUSH_DNS,
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
            metadata: remediation_catalog::CLEAR_ICON_CACHE,
            run: RunKind::Custom {
                f: clear_icon_cache,
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::EMPTY_RECYCLE_BIN,
            run: RunKind::Custom {
                f: empty_recycle_bin,
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::CLEAR_TEMP_FILES,
            run: RunKind::Custom {
                f: clear_temp_files,
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::START_CRITICAL_SERVICES,
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
            metadata: remediation_catalog::WINDOWS_UPDATE_RESET,
            run: RunKind::Custom {
                f: reset_windows_update,
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::DISM_RESTOREHEALTH,
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
            metadata: remediation_catalog::SFC_SCANNOW,
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
            metadata: remediation_catalog::NETWORK_RESET,
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
                        ignore_failure: false,
                        action_label: "Reset TCP/IP stack",
                    },
                ],
                timeout_secs: 120,
                success_msg: "Network stack reset. Restart Windows to apply.",
            },
        },
        RemediationSpec {
            metadata: remediation_catalog::RESTART_SYSTEM,
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

/// Test-only adapter that preserves the old confirmation-gate regression
/// checks. Production has no confirmation boolean; it enters through the
/// action broker's consumed grant and calls `execute_authorized`.
#[cfg(test)]
async fn execute(
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
    execute_authorized(remediation_id, runner, &CancellationToken::new())
        .await
        .map(|result| FixOutcome::Completed { result })
}

/// Authorized catalog execution with cooperative cancellation. The action
/// broker is the only production caller; model and frontend strings never
/// become programs, arguments, or a confirmation boolean.
pub(crate) async fn execute_authorized(
    remediation_id: &str,
    runner: &dyn CommandRunner,
    cancel: &CancellationToken,
) -> Result<FixResult, String> {
    let spec =
        find(remediation_id).ok_or_else(|| format!("Unknown remediation '{}'", remediation_id))?;

    let result = if cancel.is_cancelled() {
        cancelled_result("Remediation cancelled before it started")
    } else {
        match &spec.run {
            RunKind::Spawn { program, args } => match runner.spawn(program, args) {
                Ok(()) => FixResult {
                    success: true,
                    message: format!("{} opened.", spec.label),
                    actions_taken: vec![format!("Launched {}", program)],
                    requires_restart: false,
                    completion_status: FixCompletionStatus::Succeeded,
                    steps: vec![RemediationStepResult {
                        action: format!("Launch {}", program),
                        status: RemediationStepStatus::Succeeded,
                        detail: None,
                    }],
                },
                Err(e) => FixResult {
                    success: false,
                    message: format!("Could not launch {}: {}", program, e),
                    actions_taken: vec![],
                    requires_restart: false,
                    completion_status: FixCompletionStatus::Failed,
                    steps: vec![RemediationStepResult {
                        action: format!("Launch {}", program),
                        status: RemediationStepStatus::Failed,
                        detail: Some(e.to_string()),
                    }],
                },
            },
            RunKind::Steps {
                steps,
                timeout_secs,
                success_msg,
            } => {
                let mut actions_taken = Vec::new();
                let mut step_results = Vec::new();
                let timeout = Duration::from_secs(*timeout_secs);
                let mut failures = Vec::new();
                for step in *steps {
                    if cancel.is_cancelled() {
                        step_results.push(RemediationStepResult {
                            action: step.action_label.to_string(),
                            status: RemediationStepStatus::Cancelled,
                            detail: Some("Cancelled before this step started".to_string()),
                        });
                        break;
                    }
                    match runner.run(step.program, step.args, timeout, cancel).await {
                        Ok(output) if output.success => {
                            actions_taken.push(step.action_label.to_string());
                            step_results.push(RemediationStepResult {
                                action: step.action_label.to_string(),
                                status: RemediationStepStatus::Succeeded,
                                detail: None,
                            });
                        }
                        Ok(output) => {
                            let detail = if output.stderr.trim().is_empty() {
                                output.stdout
                            } else {
                                output.stderr
                            };
                            let detail = detail.trim().chars().take(400).collect::<String>();
                            if command_already_satisfied(&detail) {
                                step_results.push(RemediationStepResult {
                                    action: step.action_label.to_string(),
                                    status: RemediationStepStatus::AlreadySatisfied,
                                    detail: (!detail.is_empty()).then_some(detail),
                                });
                                continue;
                            }
                            let failure = format!(
                                "'{} {}' failed: {}",
                                step.program,
                                step.args.join(" "),
                                detail
                            );
                            step_results.push(RemediationStepResult {
                                action: step.action_label.to_string(),
                                status: RemediationStepStatus::Failed,
                                detail: Some(failure.clone()),
                            });
                            failures.push(failure);
                            if !step.ignore_failure {
                                break;
                            }
                        }
                        Err(e) => {
                            if cancel.is_cancelled() {
                                step_results.push(RemediationStepResult {
                                    action: step.action_label.to_string(),
                                    status: RemediationStepStatus::Cancelled,
                                    detail: Some(
                                        "Cancelled while this step was running".to_string(),
                                    ),
                                });
                                break;
                            }
                            let failure = e.to_string();
                            step_results.push(RemediationStepResult {
                                action: step.action_label.to_string(),
                                status: RemediationStepStatus::Failed,
                                detail: Some(failure.clone()),
                            });
                            failures.push(failure);
                            if !step.ignore_failure {
                                break;
                            }
                        }
                    }
                }
                let completion_status = completion_status(&step_results);
                let success = completion_status == FixCompletionStatus::Succeeded;
                FixResult {
                    success,
                    message: if completion_status == FixCompletionStatus::Cancelled {
                        "Remediation cancelled.".to_string()
                    } else if success {
                        success_msg.to_string()
                    } else {
                        format!(
                            "{} remediation step(s) failed: {}",
                            failures.len(),
                            failures.join("; ")
                        )
                    },
                    requires_restart: spec.requires_restart
                        && step_results
                            .iter()
                            .any(|step| step.status == RemediationStepStatus::Succeeded),
                    actions_taken,
                    completion_status,
                    steps: step_results,
                }
            }
            RunKind::Custom { f } => {
                // Filesystem and Windows Shell custom actions are blocking. Keep
                // them off Tokio's async worker threads.
                let custom = *f;
                let cancel = cancel.clone();
                match tokio::task::spawn_blocking(move || custom(&cancel)).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(e)) => failed_result(e.to_string()),
                    Err(e) => failed_result(format!("Remediation worker failed: {}", e)),
                }
            }
        }
    };

    Ok(result)
}

fn command_already_satisfied(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("already been started")
        || detail.contains("service is already running")
        || detail.contains("instance of the service is already running")
}

fn completion_status(steps: &[RemediationStepResult]) -> FixCompletionStatus {
    let succeeded = steps.iter().any(|step| {
        matches!(
            step.status,
            RemediationStepStatus::Succeeded | RemediationStepStatus::AlreadySatisfied
        )
    });
    let failed = steps
        .iter()
        .any(|step| step.status == RemediationStepStatus::Failed);
    let cancelled = steps
        .iter()
        .any(|step| step.status == RemediationStepStatus::Cancelled);
    if cancelled {
        return FixCompletionStatus::Cancelled;
    }
    match (succeeded, failed) {
        (_, false) => FixCompletionStatus::Succeeded,
        (true, true) => FixCompletionStatus::Partial,
        (false, true) => FixCompletionStatus::Failed,
    }
}

fn cancelled_result(message: &str) -> FixResult {
    FixResult {
        success: false,
        message: message.to_string(),
        actions_taken: vec![],
        requires_restart: false,
        completion_status: FixCompletionStatus::Cancelled,
        steps: vec![RemediationStepResult {
            action: "Run remediation".to_string(),
            status: RemediationStepStatus::Cancelled,
            detail: Some(message.to_string()),
        }],
    }
}

fn failed_result(message: String) -> FixResult {
    FixResult {
        success: false,
        message: message.clone(),
        actions_taken: vec![],
        requires_restart: false,
        completion_status: FixCompletionStatus::Failed,
        steps: vec![RemediationStepResult {
            action: "Run remediation".to_string(),
            status: RemediationStepStatus::Failed,
            detail: Some(message),
        }],
    }
}

// ============================================================================
// Custom (filesystem) remediations
// ============================================================================

fn clear_icon_cache(cancel: &CancellationToken) -> anyhow::Result<FixResult> {
    let mut actions_taken = Vec::new();
    let mut failures = Vec::new();
    let profile =
        std::env::var("USERPROFILE").map_err(|_| anyhow::anyhow!("USERPROFILE is unavailable"))?;
    let icon_cache = std::path::Path::new(&profile).join(r"AppData\Local\IconCache.db");
    if cancel.is_cancelled() {
        return Ok(cancelled_result(
            "Icon-cache cleanup cancelled before it started",
        ));
    }
    if icon_cache.exists() {
        match std::fs::remove_file(&icon_cache) {
            Ok(()) => actions_taken.push("Deleted IconCache.db".to_string()),
            Err(error) => failures.push(format!("IconCache.db: {}", error)),
        }
    }
    let explorer_dir =
        std::path::Path::new(&profile).join(r"AppData\Local\Microsoft\Windows\Explorer");
    if explorer_dir.exists() {
        match std::fs::read_dir(&explorer_dir) {
            Ok(entries) => {
                let mut removed = 0;
                for entry in entries {
                    if cancel.is_cancelled() {
                        return Ok(cancelled_result("Icon-cache cleanup cancelled"));
                    }
                    let Ok(entry) = entry else {
                        failures.push("Could not enumerate a thumbnail cache entry".to_string());
                        continue;
                    };
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if name.starts_with("thumbcache") {
                        match std::fs::remove_file(entry.path()) {
                            Ok(()) => removed += 1,
                            Err(error) => failures.push(format!("{}: {}", name, error)),
                        }
                    }
                }
                if removed > 0 {
                    actions_taken.push(format!("Deleted {} thumbnail cache file(s)", removed));
                }
            }
            Err(error) => failures.push(format!("Thumbnail cache directory: {}", error)),
        }
    }
    let completion_status = if failures.is_empty() {
        FixCompletionStatus::Succeeded
    } else if actions_taken.is_empty() {
        FixCompletionStatus::Failed
    } else {
        FixCompletionStatus::Partial
    };
    let changed = !actions_taken.is_empty();
    Ok(FixResult {
        success: failures.is_empty(),
        message: if failures.is_empty() && !changed {
            "Icon and thumbnail caches were already clear.".to_string()
        } else if failures.is_empty() {
            "Icon and thumbnail caches cleared. They rebuild after you sign in again.".to_string()
        } else {
            format!(
                "Some icon-cache items could not be removed: {}",
                failures.join("; ")
            )
        },
        actions_taken,
        requires_restart: changed && completion_status != FixCompletionStatus::Failed,
        completion_status,
        steps: vec![RemediationStepResult {
            action: "Clear icon and thumbnail caches".to_string(),
            status: if failures.is_empty() && !changed {
                RemediationStepStatus::AlreadySatisfied
            } else if failures.is_empty() {
                RemediationStepStatus::Succeeded
            } else {
                RemediationStepStatus::Failed
            },
            detail: (!failures.is_empty()).then(|| failures.join("; ")),
        }],
    })
}

#[cfg(windows)]
fn empty_recycle_bin(_cancel: &CancellationToken) -> anyhow::Result<FixResult> {
    use windows::Win32::UI::Shell::{
        SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW,
    };
    use windows::core::PCWSTR;

    unsafe {
        SHEmptyRecycleBinW(
            None,
            PCWSTR(std::ptr::null()),
            SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
        )?;
    }

    Ok(FixResult {
        success: true,
        message: "Recycle Bin emptied.".to_string(),
        actions_taken: vec!["Emptied the Recycle Bin via Windows Shell API".to_string()],
        requires_restart: false,
        completion_status: FixCompletionStatus::Succeeded,
        steps: vec![RemediationStepResult {
            action: "Empty the Recycle Bin".to_string(),
            status: RemediationStepStatus::Succeeded,
            detail: None,
        }],
    })
}

#[cfg(not(windows))]
fn empty_recycle_bin(_cancel: &CancellationToken) -> anyhow::Result<FixResult> {
    anyhow::bail!("Empty Recycle Bin is only supported on Windows")
}

fn clear_temp_files(cancel: &CancellationToken) -> anyhow::Result<FixResult> {
    let temp = std::env::temp_dir();
    let mut removed = 0u32;
    let mut skipped = 0u32;
    let entries = std::fs::read_dir(&temp)
        .map_err(|error| anyhow::anyhow!("Could not read {}: {}", temp.display(), error))?;
    for entry in entries {
        if cancel.is_cancelled() {
            return Ok(cancelled_result(&format!(
                "Temp cleanup cancelled after removing {} item(s)",
                removed
            )));
        }
        let Ok(entry) = entry else {
            skipped += 1;
            continue;
        };
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
    Ok(FixResult {
        success: skipped == 0,
        message: format!(
            "Removed {} temp item(s); {} in use were skipped.",
            removed, skipped
        ),
        actions_taken: (removed > 0)
            .then(|| format!("Removed {} item(s) from {}", removed, temp.display()))
            .into_iter()
            .collect(),
        requires_restart: false,
        completion_status: if skipped == 0 {
            FixCompletionStatus::Succeeded
        } else if removed > 0 {
            FixCompletionStatus::Partial
        } else {
            FixCompletionStatus::Failed
        },
        steps: vec![RemediationStepResult {
            action: format!("Clean {}", temp.display()),
            status: if skipped == 0 {
                RemediationStepStatus::Succeeded
            } else {
                RemediationStepStatus::Failed
            },
            detail: (skipped > 0)
                .then(|| format!("{} locked item(s) could not be removed", skipped)),
        }],
    })
}

/// `net stop`/`net start` report "the service is ALREADY in that state" as a
/// FAILURE exit (NET HELPMSG 3521 / 2182). For this reset those are no-op
/// successes — recognize the benign messages (pure for testability).
fn benign_service_state_output(output: &str) -> bool {
    let text = output.to_lowercase();
    // Matched by phrase AND by NET HELPMSG number: localized Windows
    // translates the text but keeps the message number.
    const BENIGN_MARKERS: [&str; 5] = [
        "not started",
        "3521",
        "already been started",
        "already running",
        "2182",
    ];
    BENIGN_MARKERS.iter().any(|marker| text.contains(marker))
}

fn reset_windows_update(_cancel: &CancellationToken) -> anyhow::Result<FixResult> {
    // Stop -> clear download cache -> start. Bound service-control waits so a
    // hung SCM call cannot stall the remediation indefinitely.
    let mut actions_taken = Vec::new();

    // Returns (exit_ok, combined output) — the output lets callers tell a
    // real failure from a benign "already in target state" result.
    let run_quiet = |program: &str, args: &[&str]| -> anyhow::Result<(bool, String)> {
        let mut cmd = std::process::Command::new(crate::security::trusted_system_program(program)?);
        cmd.args(args);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn()?;
        let timeout = Duration::from_secs(60);
        let deadline = std::time::Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!(
                    "'{} {}' timed out after {} second(s)",
                    program,
                    args.join(" "),
                    timeout.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        // Child has exited, so both pipes are complete (net.exe writes only
        // a few lines — they cannot have filled and deadlocked it).
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(mut pipe) = child.stdout.take() {
            let _ = std::io::Read::read_to_end(&mut pipe, &mut stdout);
        }
        if let Some(mut pipe) = child.stderr.take() {
            let _ = std::io::Read::read_to_end(&mut pipe, &mut stderr);
        }
        let mut output = String::from_utf8_lossy(&stdout).into_owned();
        output.push('\n');
        output.push_str(&String::from_utf8_lossy(&stderr));
        Ok((status.success(), output))
    };

    let mut steps = Vec::new();
    match run_quiet("net", &["stop", "wuauserv"]) {
        Ok((true, _)) => {
            actions_taken.push("Stopped Windows Update service".to_string());
            steps.push(RemediationStepResult {
                action: "Stop Windows Update service".to_string(),
                status: RemediationStepStatus::Succeeded,
                detail: None,
            });
        }
        Ok((false, output)) if benign_service_state_output(&output) => {
            actions_taken.push("Windows Update service was already stopped".to_string());
            steps.push(RemediationStepResult {
                action: "Stop Windows Update service".to_string(),
                status: RemediationStepStatus::Succeeded,
                detail: Some("Service was already stopped".to_string()),
            });
        }
        Ok((false, _)) => steps.push(RemediationStepResult {
            action: "Stop Windows Update service".to_string(),
            status: RemediationStepStatus::Failed,
            detail: Some("Service-control command returned a failure status".to_string()),
        }),
        Err(error) => steps.push(RemediationStepResult {
            action: "Stop Windows Update service".to_string(),
            status: RemediationStepStatus::Failed,
            detail: Some(error.to_string()),
        }),
    }

    let download_dir = std::path::Path::new(r"C:\Windows\SoftwareDistribution\Download");
    match std::fs::read_dir(download_dir) {
        Ok(entries) => {
            let mut removed = 0;
            let mut failed = 0;
            for entry in entries {
                let Ok(entry) = entry else {
                    failed += 1;
                    continue;
                };
                let path = entry.path();
                let ok = if path.is_dir() {
                    std::fs::remove_dir_all(&path).is_ok()
                } else {
                    std::fs::remove_file(&path).is_ok()
                };
                if ok {
                    removed += 1;
                } else {
                    failed += 1;
                }
            }
            actions_taken.push(format!("Cleared {} item(s) from the update cache", removed));
            steps.push(RemediationStepResult {
                action: "Clear Windows Update download cache".to_string(),
                status: if failed == 0 {
                    RemediationStepStatus::Succeeded
                } else {
                    RemediationStepStatus::Failed
                },
                detail: (failed > 0)
                    .then(|| format!("{} cache item(s) could not be removed", failed)),
            });
        }
        Err(error) => steps.push(RemediationStepResult {
            action: "Clear Windows Update download cache".to_string(),
            status: RemediationStepStatus::Failed,
            detail: Some(error.to_string()),
        }),
    }

    // Finally-style restoration: starting the service is attempted regardless
    // of stop/cache failures so a partial reset cannot leave it disabled.
    match run_quiet("net", &["start", "wuauserv"]) {
        Ok((true, _)) => {
            actions_taken.push("Restarted Windows Update service".to_string());
            steps.push(RemediationStepResult {
                action: "Start Windows Update service".to_string(),
                status: RemediationStepStatus::Succeeded,
                detail: None,
            });
        }
        Ok((false, output)) if benign_service_state_output(&output) => {
            actions_taken.push("Windows Update service was already running".to_string());
            steps.push(RemediationStepResult {
                action: "Start Windows Update service".to_string(),
                status: RemediationStepStatus::Succeeded,
                detail: Some("Service was already running".to_string()),
            });
        }
        Ok((false, _)) => steps.push(RemediationStepResult {
            action: "Start Windows Update service".to_string(),
            status: RemediationStepStatus::Failed,
            detail: Some("Service-control command returned a failure status".to_string()),
        }),
        Err(error) => steps.push(RemediationStepResult {
            action: "Start Windows Update service".to_string(),
            status: RemediationStepStatus::Failed,
            detail: Some(error.to_string()),
        }),
    }

    let completion_status = completion_status(&steps);
    let success = completion_status == FixCompletionStatus::Succeeded;
    Ok(FixResult {
        success,
        message: if success {
            "Windows Update components reset.".to_string()
        } else {
            "Windows Update reset completed only partially; review the failed steps.".to_string()
        },
        actions_taken,
        requires_restart: false,
        completion_status,
        steps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn benign_service_states_are_recognized() {
        // `net stop` on an already-stopped service (English + message number)
        assert!(benign_service_state_output(
            "The Windows Update service is not started.\n\nNET HELPMSG 3521"
        ));
        // `net start` on an already-running service
        assert!(benign_service_state_output(
            "The wuauserv service is already running.\n\nNET HELPMSG 2182"
        ));
        assert!(benign_service_state_output(
            "Der Dienst wurde bereits gestartet.\n\nNET HELPMSG 2182"
        ));
        // Real failures must stay failures.
        assert!(!benign_service_state_output(
            "System error 5 has occurred.\n\nAccess is denied."
        ));
        assert!(!benign_service_state_output(""));
    }

    /// Records every call; never touches the system.
    #[derive(Default)]
    struct RecordingRunner {
        calls: Mutex<Vec<String>>,
        fail_on: Option<&'static str>,
        fail_args: Option<&'static str>,
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
            _cancel: &'a CancellationToken,
        ) -> RunFuture<'a> {
            let line = format!("run:{} {}", program, args.join(" "));
            self.calls.lock().unwrap().push(line);
            let fail = self.fail_on.is_some_and(|f| program == f)
                || self
                    .fail_args
                    .is_some_and(|needle| args.join(" ") == needle);
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

        let metadata_catalog = remediation_catalog::catalog();
        assert_eq!(
            remediations().len(),
            metadata_catalog.len(),
            "every metadata entry must have one trusted execution entry"
        );
        for metadata in metadata_catalog {
            let matches: Vec<_> = remediations()
                .iter()
                .filter(|spec| spec.id == metadata.id)
                .collect();
            assert_eq!(
                matches.len(),
                1,
                "metadata '{}' must map to exactly one execution entry",
                metadata.id
            );
            let spec = matches[0];
            assert!(
                std::ptr::eq(spec.metadata, metadata),
                "execution '{}' must reference the canonical metadata object",
                metadata.id
            );
            assert_eq!(spec.summary(), metadata.summary());
            assert_eq!(
                spec.cancellable(),
                metadata.cancellable,
                "execution cancellation policy drift for '{}'",
                metadata.id
            );
        }

        // Every issue's remediation_id resolves to exactly one metadata and
        // trusted execution entry.
        for issue in crate::issue_catalog::catalog() {
            if let Some(remediation_id) = issue.remediation_id {
                assert_eq!(
                    metadata_catalog
                        .iter()
                        .filter(|metadata| metadata.id == remediation_id)
                        .count(),
                    1,
                    "issue '{}' must resolve canonical remediation '{}' exactly once",
                    issue.id,
                    remediation_id
                );
                assert_eq!(
                    remediations()
                        .iter()
                        .filter(|spec| spec.id == remediation_id)
                        .count(),
                    1,
                    "issue '{}' must resolve execution remediation '{}' exactly once",
                    issue.id,
                    remediation_id
                );
            }
        }
    }

    #[test]
    fn catalog_does_not_shell_out_to_powershell() {
        for spec in remediations() {
            if let RunKind::Steps { steps, .. } = &spec.run {
                for step in *steps {
                    assert_ne!(
                        step.program.to_ascii_lowercase(),
                        "powershell",
                        "{}",
                        spec.id
                    );
                    assert_ne!(
                        step.program.to_ascii_lowercase(),
                        "powershell.exe",
                        "{}",
                        spec.id
                    );
                }
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
    async fn destructive_cleanups_require_confirmation() {
        let runner = RecordingRunner::default();
        for remediation_id in ["empty_recycle_bin", "clear_temp_files"] {
            let outcome = execute(remediation_id, false, &runner).await.unwrap();
            assert!(matches!(outcome, FixOutcome::NeedsConfirmation { .. }));
        }
        assert!(runner.calls.lock().unwrap().is_empty());
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
    async fn later_step_failure_reports_partial_result() {
        let runner = RecordingRunner {
            fail_args: Some("int ip reset"),
            ..Default::default()
        };
        let outcome = execute("network_reset", true, &runner).await.unwrap();
        let FixOutcome::Completed { result } = outcome else {
            panic!()
        };
        assert!(!result.success);
        assert_eq!(result.completion_status, FixCompletionStatus::Partial);
        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[0].status, RemediationStepStatus::Succeeded);
        assert_eq!(result.steps[1].status, RemediationStepStatus::Failed);
        assert!(result.requires_restart);
    }

    #[test]
    fn prefetch_cleanup_is_not_in_the_catalog() {
        assert!(find("clear_prefetch").is_none());
    }

    #[test]
    fn already_running_service_is_not_a_failed_step() {
        assert!(command_already_satisfied(
            "An instance of the service is already running."
        ));
        let steps = vec![RemediationStepResult {
            action: "Start service".to_string(),
            status: RemediationStepStatus::AlreadySatisfied,
            detail: None,
        }];
        assert_eq!(completion_status(&steps), FixCompletionStatus::Succeeded);
    }

    #[tokio::test]
    async fn unknown_remediation_is_an_error() {
        let runner = RecordingRunner::default();
        assert!(execute("nuke_everything", true, &runner).await.is_err());
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancelled_execution_never_constructs_a_command() {
        let runner = RecordingRunner::default();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = execute_authorized("flush_dns", &runner, &cancel)
            .await
            .unwrap();
        assert_eq!(result.completion_status, FixCompletionStatus::Cancelled);
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn only_low_impact_actions_are_batch_eligible() {
        for spec in remediations() {
            if spec.batch_eligible() {
                assert_eq!(spec.tier, RemediationTier::AutoSafe);
                assert!(!spec.admin_required);
                assert!(!spec.requires_restart);
                assert!(!spec.long_running);
            }
            assert_eq!(spec.summary().batch_eligible, spec.batch_eligible());
        }
        assert!(find("flush_dns").unwrap().batch_eligible());
        assert!(!find("sfc_scannow").unwrap().batch_eligible());
    }
}
