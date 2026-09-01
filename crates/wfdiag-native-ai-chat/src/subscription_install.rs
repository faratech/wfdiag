//! Explicit, bounded installation of the genuine Codex and Claude Code CLIs.
//!
//! This module is shared by every desktop shell. Installation is a separate,
//! destructive boundary from account authentication: it accepts only static
//! provider specifications, checks its confirmation flags before touching the
//! filesystem or starting a probe, and never starts a login flow. Winget is
//! the primary method. A failed or unavailable winget run returns a structured
//! result that requires a second, method-specific confirmation before the
//! vendor PowerShell bootstrap can run.
//!
//! On Windows, every process is created suspended and assigned to a Job Object
//! configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` before it resumes. A
//! cancellation, timeout, or dropped future therefore terminates the complete
//! installer tree, including descendants created by winget or PowerShell.

use crate::{SubscriptionAuthProvider, SubscriptionAuthState, SubscriptionAuthStatus, cli_bridge};
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncRead;
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
use process_wrap::tokio::ProcessSession;
#[cfg(windows)]
use process_wrap::tokio::{CreationFlags, JobObject};
#[cfg(windows)]
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
const LOOKUP_STDOUT_LIMIT: usize = 16 * 1024;
const STATUS_STDOUT_LIMIT: usize = 8 * 1024;
const STATUS_STDERR_LIMIT: usize = 8 * 1024;

type InstallFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The only supported installation methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionInstallMethod {
    /// Official package from the Windows Package Manager community source.
    Winget,
    /// Vendor-owned PowerShell bootstrap. This always needs a second approval.
    VendorPowerShell,
}

impl fmt::Display for SubscriptionInstallMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Winget => "winget",
            Self::VendorPowerShell => "the vendor PowerShell installer",
        })
    }
}

/// A fully explicit installation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionInstallRequest {
    pub provider: SubscriptionAuthProvider,
    pub method: SubscriptionInstallMethod,
    /// Approval for installation itself. Checked before any filesystem probe.
    pub confirmed: bool,
    /// Separate approval for downloading and executing a vendor script.
    pub fallback_confirmed: bool,
}

impl SubscriptionInstallRequest {
    #[must_use]
    pub const fn winget(provider: SubscriptionAuthProvider, confirmed: bool) -> Self {
        Self {
            provider,
            method: SubscriptionInstallMethod::Winget,
            confirmed,
            fallback_confirmed: false,
        }
    }

    #[must_use]
    pub const fn vendor_fallback(
        provider: SubscriptionAuthProvider,
        confirmed: bool,
        fallback_confirmed: bool,
    ) -> Self {
        Self {
            provider,
            method: SubscriptionInstallMethod::VendorPowerShell,
            confirmed,
            fallback_confirmed,
        }
    }
}

/// Stable progress states. They intentionally contain no process output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionInstallStage {
    CheckingExisting,
    ResolvingInstaller,
    InstallingWinget,
    InstallingVendorFallback,
    Verifying,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionInstallProgress {
    pub provider: SubscriptionAuthProvider,
    pub method: SubscriptionInstallMethod,
    pub stage: SubscriptionInstallStage,
}

/// Verified post-install state. `path` is always absolute and points at a
/// file observed after confirmation. Account state comes from the exact same
/// enum as the shared sign-in controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionInstallStatus {
    pub provider: SubscriptionAuthProvider,
    pub path: PathBuf,
    pub state: SubscriptionAuthState,
}

impl SubscriptionInstallStatus {
    #[must_use]
    pub fn auth_status(&self) -> SubscriptionAuthStatus {
        SubscriptionAuthStatus {
            provider: self.provider,
            state: self.state,
            path: Some(self.path.clone()),
        }
    }
}

/// Why winget did not complete the primary installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionInstallFallbackReason {
    ExplicitApprovalMissing,
    WingetUnavailable,
    WingetFailed,
}

/// Sanitized failures safe to render. No variant carries stdout, stderr,
/// command text, an OS error, or any other child-controlled string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionInstallError {
    ConfirmationRequired {
        provider: SubscriptionAuthProvider,
        method: SubscriptionInstallMethod,
    },
    VendorFallbackConfirmationRequired {
        provider: SubscriptionAuthProvider,
        reason: SubscriptionInstallFallbackReason,
    },
    UnsupportedPlatform {
        provider: SubscriptionAuthProvider,
    },
    AlreadyInProgress {
        provider: SubscriptionAuthProvider,
    },
    Cancelled {
        provider: SubscriptionAuthProvider,
        method: SubscriptionInstallMethod,
    },
    VendorInstallerUnavailable {
        provider: SubscriptionAuthProvider,
    },
    InstallFailed {
        provider: SubscriptionAuthProvider,
        method: SubscriptionInstallMethod,
    },
    VerificationFailed {
        provider: SubscriptionAuthProvider,
    },
}

impl fmt::Display for SubscriptionInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfirmationRequired { provider, method } => write!(
                formatter,
                "Installing {provider} with {method} requires explicit confirmation."
            ),
            Self::VendorFallbackConfirmationRequired { provider, reason } => {
                let explanation = match reason {
                    SubscriptionInstallFallbackReason::ExplicitApprovalMissing => {
                        "has not been separately approved"
                    }
                    SubscriptionInstallFallbackReason::WingetUnavailable => {
                        "is available only after winget could not be found"
                    }
                    SubscriptionInstallFallbackReason::WingetFailed => {
                        "is available only after winget did not complete the installation"
                    }
                };
                write!(
                    formatter,
                    "The {provider} vendor PowerShell fallback {explanation}; confirm that fallback explicitly before it runs."
                )
            }
            Self::UnsupportedPlatform { provider } => write!(
                formatter,
                "Automatic {provider} installation is available only on Windows."
            ),
            Self::AlreadyInProgress { provider } => {
                write!(
                    formatter,
                    "A {provider} installation is already in progress."
                )
            }
            Self::Cancelled { provider, method } => {
                write!(
                    formatter,
                    "Installing {provider} with {method} was cancelled."
                )
            }
            Self::VendorInstallerUnavailable { provider } => write!(
                formatter,
                "The trusted Windows PowerShell host required to install {provider} was not found."
            ),
            Self::InstallFailed { provider, method } => write!(
                formatter,
                "The {provider} installation with {method} did not complete."
            ),
            Self::VerificationFailed { provider } => write!(
                formatter,
                "The {provider} installer finished, but its CLI could not be verified. Restart the app and check again."
            ),
        }
    }
}

impl std::error::Error for SubscriptionInstallError {}

struct InstallSpec {
    binary: &'static str,
    winget_package: &'static str,
    vendor_script: &'static str,
    status_args: &'static [&'static str],
    signed_out_markers: &'static [&'static str],
}

const CODEX_SPEC: InstallSpec = InstallSpec {
    binary: "codex",
    winget_package: "OpenAI.Codex",
    vendor_script: "$env:CODEX_NON_INTERACTIVE = '1'; irm https://chatgpt.com/codex/install.ps1 | iex",
    status_args: &["login", "status"],
    signed_out_markers: &["not logged in"],
};

const CLAUDE_SPEC: InstallSpec = InstallSpec {
    binary: "claude",
    winget_package: "Anthropic.ClaudeCode",
    vendor_script: "irm https://claude.ai/install.ps1 | iex",
    status_args: &["auth", "status"],
    signed_out_markers: &["not logged in", "please run /login"],
};

fn spec(provider: SubscriptionAuthProvider) -> &'static InstallSpec {
    match provider {
        SubscriptionAuthProvider::Codex => &CODEX_SPEC,
        SubscriptionAuthProvider::ClaudeCode => &CLAUDE_SPEC,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessPurpose {
    Lookup,
    InstallWinget,
    InstallVendorPowerShell,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessPlan {
    program: PathBuf,
    args: Vec<&'static str>,
    purpose: ProcessPurpose,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    /// The real runner treats this as a mandatory contract, not a hint.
    terminate_entire_tree_on_drop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessFailure {
    Cancelled,
    TimedOut,
    Spawn,
    Wait,
}

trait InstallProcess: Send + Sync + 'static {
    fn env_var(&self, name: &'static str) -> Option<OsString>;
    fn file_exists(&self, path: &Path) -> bool;
    fn run(
        &self,
        plan: ProcessPlan,
        cancellation: CancellationToken,
    ) -> InstallFuture<'_, Result<ProcessOutput, ProcessFailure>>;
}

#[derive(Debug, Default)]
struct RealInstallProcess;

impl InstallProcess for RealInstallProcess {
    fn env_var(&self, name: &'static str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn file_exists(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn run(
        &self,
        plan: ProcessPlan,
        cancellation: CancellationToken,
    ) -> InstallFuture<'_, Result<ProcessOutput, ProcessFailure>> {
        Box::pin(run_contained_process(plan, cancellation))
    }
}

/// #204: one bounded-drain implementation, now shared with the chat bridge
/// process runner in `cli_bridge.rs`.
async fn drain_bounded<R: AsyncRead + Unpin>(
    reader: Option<R>,
    limit: usize,
) -> Result<Vec<u8>, ProcessFailure> {
    cli_bridge::drain_bounded(reader, limit)
        .await
        .map_err(|_| ProcessFailure::Wait)
}

async fn run_contained_process(
    plan: ProcessPlan,
    cancellation: CancellationToken,
) -> Result<ProcessOutput, ProcessFailure> {
    debug_assert!(plan.terminate_entire_tree_on_drop);
    let mut command = tokio::process::Command::new(&plan.program);
    command.args(&plan.args);
    command.stdin(Stdio::null());
    command.stdout(if plan.stdout_limit == 0 {
        Stdio::null()
    } else {
        Stdio::piped()
    });
    command.stderr(if plan.stderr_limit == 0 {
        Stdio::null()
    } else {
        Stdio::piped()
    });
    for variable in cli_bridge::SUBSCRIPTION_OVERRIDE_ENV_VARS {
        command.env_remove(variable);
    }
    let workdir = std::env::temp_dir();
    if workdir.is_dir() {
        command.current_dir(workdir);
    }

    let mut wrapped = CommandWrap::from(command);
    wrapped.wrap(KillOnDrop);
    #[cfg(windows)]
    {
        // JobObject temporarily adds CREATE_SUSPENDED, assigns the child, then
        // resumes it. KillOnDrop enables KILL_ON_JOB_CLOSE for descendants.
        wrapped.wrap(CreationFlags(CREATE_NO_WINDOW));
        wrapped.wrap(JobObject);
    }
    #[cfg(unix)]
    wrapped.wrap(ProcessSession);

    let mut child = wrapped.spawn().map_err(|_| ProcessFailure::Spawn)?;
    let stdout = child.stdout().take();
    let stderr = child.stderr().take();
    let operation = async {
        let (status, stdout, stderr) = tokio::join!(
            child.wait(),
            drain_bounded(stdout, plan.stdout_limit),
            drain_bounded(stderr, plan.stderr_limit),
        );
        Ok(ProcessOutput {
            success: status.map_err(|_| ProcessFailure::Wait)?.success(),
            stdout: stdout?,
            stderr: stderr?,
        })
    };

    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(ProcessFailure::Cancelled),
        result = tokio::time::timeout(plan.timeout, operation) => {
            result.map_err(|_| ProcessFailure::TimedOut)?
        }
    }
    // `child` is dropped here on every exit. On Windows that closes the Job
    // Object and synchronously requests termination of every descendant.
}

/// Shared installer controller. Construction is side-effect-free.
#[derive(Clone)]
pub struct SubscriptionInstallController {
    process: Arc<dyn InstallProcess>,
    active: Arc<Mutex<HashSet<SubscriptionAuthProvider>>>,
    platform_supported: bool,
}

impl fmt::Debug for SubscriptionInstallController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubscriptionInstallController")
            .finish_non_exhaustive()
    }
}

impl Default for SubscriptionInstallController {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriptionInstallController {
    #[must_use]
    pub fn new() -> Self {
        Self {
            process: Arc::new(RealInstallProcess),
            active: Arc::new(Mutex::new(HashSet::new())),
            platform_supported: cfg!(windows),
        }
    }

    #[cfg(test)]
    fn with_process(process: Arc<dyn InstallProcess>) -> Self {
        Self {
            process,
            active: Arc::new(Mutex::new(HashSet::new())),
            platform_supported: true,
        }
    }

    /// Install or verify a subscription CLI without ever authenticating it.
    ///
    /// The callback receives static progress states only. It is called from
    /// this future and should return promptly.
    pub async fn install<F>(
        &self,
        request: SubscriptionInstallRequest,
        cancellation: CancellationToken,
        on_progress: F,
    ) -> Result<SubscriptionInstallStatus, SubscriptionInstallError>
    where
        F: Fn(SubscriptionInstallProgress) + Send + Sync,
    {
        // This must remain the first effectful boundary in the method. In
        // particular, do not move platform or existing-install probes above it.
        validate_confirmation(request)?;
        if !self.platform_supported {
            return Err(SubscriptionInstallError::UnsupportedPlatform {
                provider: request.provider,
            });
        }
        let _reservation = InstallReservation::acquire(self.active.clone(), request.provider)?;
        check_cancelled(request, &cancellation)?;

        emit_progress(
            &on_progress,
            request,
            SubscriptionInstallStage::CheckingExisting,
        );
        if let Some(status) = self
            .discover_and_verify(request.provider, request.method, cancellation.clone())
            .await?
        {
            emit_progress(&on_progress, request, SubscriptionInstallStage::Completed);
            return Ok(status);
        }

        emit_progress(
            &on_progress,
            request,
            SubscriptionInstallStage::ResolvingInstaller,
        );
        let installer = match self.installer_path(request.method) {
            Some(path) => path,
            None if request.method == SubscriptionInstallMethod::Winget => {
                return Err(
                    SubscriptionInstallError::VendorFallbackConfirmationRequired {
                        provider: request.provider,
                        reason: SubscriptionInstallFallbackReason::WingetUnavailable,
                    },
                );
            }
            None => {
                return Err(SubscriptionInstallError::VendorInstallerUnavailable {
                    provider: request.provider,
                });
            }
        };

        let stage = match request.method {
            SubscriptionInstallMethod::Winget => SubscriptionInstallStage::InstallingWinget,
            SubscriptionInstallMethod::VendorPowerShell => {
                SubscriptionInstallStage::InstallingVendorFallback
            }
        };
        emit_progress(&on_progress, request, stage);
        let plan = install_plan(request, installer);
        let output = self.process.run(plan, cancellation.clone()).await;
        match output {
            Err(ProcessFailure::Cancelled) => {
                return Err(cancelled(request));
            }
            Ok(output) if output.success => {}
            Ok(_)
            | Err(ProcessFailure::TimedOut | ProcessFailure::Spawn | ProcessFailure::Wait)
                if request.method == SubscriptionInstallMethod::Winget =>
            {
                return Err(
                    SubscriptionInstallError::VendorFallbackConfirmationRequired {
                        provider: request.provider,
                        reason: SubscriptionInstallFallbackReason::WingetFailed,
                    },
                );
            }
            Ok(_) | Err(_) => {
                return Err(SubscriptionInstallError::InstallFailed {
                    provider: request.provider,
                    method: request.method,
                });
            }
        }

        emit_progress(&on_progress, request, SubscriptionInstallStage::Verifying);
        // A successful installer exit is the external mutation's commit
        // point. A cancellation arriving after that must not relabel an
        // already-installed CLI as cancelled, so the short, read-only
        // verification gets a fresh bounded token.
        let status = self
            .discover_and_verify(request.provider, request.method, CancellationToken::new())
            .await?
            .ok_or(SubscriptionInstallError::VerificationFailed {
                provider: request.provider,
            })?;
        emit_progress(&on_progress, request, SubscriptionInstallStage::Completed);
        Ok(status)
    }

    async fn discover_and_verify(
        &self,
        provider: SubscriptionAuthProvider,
        method: SubscriptionInstallMethod,
        cancellation: CancellationToken,
    ) -> Result<Option<SubscriptionInstallStatus>, SubscriptionInstallError> {
        let Some(path) = self
            .discover_cli(provider, method, cancellation.clone())
            .await?
        else {
            return Ok(None);
        };
        let install_spec = spec(provider);
        let plan = ProcessPlan {
            program: path.clone(),
            args: install_spec.status_args.to_vec(),
            purpose: ProcessPurpose::Status,
            timeout: STATUS_TIMEOUT,
            stdout_limit: STATUS_STDOUT_LIMIT,
            stderr_limit: STATUS_STDERR_LIMIT,
            terminate_entire_tree_on_drop: true,
        };
        let output = self
            .process
            .run(plan, cancellation)
            .await
            .map_err(|failure| {
                if failure == ProcessFailure::Cancelled {
                    SubscriptionInstallError::Cancelled { provider, method }
                } else {
                    SubscriptionInstallError::VerificationFailed { provider }
                }
            })?;
        let state = parse_auth_state(install_spec, &output);
        Ok(Some(SubscriptionInstallStatus {
            provider,
            path,
            state,
        }))
    }

    async fn discover_cli(
        &self,
        provider: SubscriptionAuthProvider,
        method: SubscriptionInstallMethod,
        cancellation: CancellationToken,
    ) -> Result<Option<PathBuf>, SubscriptionInstallError> {
        for candidate in known_cli_candidates(self.process.as_ref(), provider) {
            if candidate.is_absolute() && self.process.file_exists(&candidate) {
                return Ok(Some(candidate));
            }
        }

        let install_spec = spec(provider);
        let plan = ProcessPlan {
            program: lookup_program(self.process.as_ref()),
            args: vec![install_spec.binary],
            purpose: ProcessPurpose::Lookup,
            timeout: LOOKUP_TIMEOUT,
            stdout_limit: LOOKUP_STDOUT_LIMIT,
            stderr_limit: 0,
            terminate_entire_tree_on_drop: true,
        };
        let output = match self.process.run(plan, cancellation).await {
            Ok(output) if output.success => output,
            Ok(_)
            | Err(ProcessFailure::Spawn | ProcessFailure::TimedOut | ProcessFailure::Wait) => {
                return Ok(None);
            }
            Err(ProcessFailure::Cancelled) => {
                return Err(SubscriptionInstallError::Cancelled { provider, method });
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(cli_bridge::pick_lookup_candidate(&stdout).filter(|path| {
            path.is_absolute()
                && binary_filename_matches(path, install_spec.binary)
                && self.process.file_exists(path)
        }))
    }

    fn installer_path(&self, method: SubscriptionInstallMethod) -> Option<PathBuf> {
        match method {
            SubscriptionInstallMethod::Winget => winget_candidates(self.process.as_ref())
                .into_iter()
                .find(|path| self.process.file_exists(path)),
            SubscriptionInstallMethod::VendorPowerShell => {
                let path = system32(self.process.as_ref())
                    .join("WindowsPowerShell")
                    .join("v1.0")
                    .join("powershell.exe");
                self.process.file_exists(&path).then_some(path)
            }
        }
    }
}

fn validate_confirmation(
    request: SubscriptionInstallRequest,
) -> Result<(), SubscriptionInstallError> {
    if !request.confirmed {
        return Err(SubscriptionInstallError::ConfirmationRequired {
            provider: request.provider,
            method: request.method,
        });
    }
    if request.method == SubscriptionInstallMethod::VendorPowerShell && !request.fallback_confirmed
    {
        return Err(
            SubscriptionInstallError::VendorFallbackConfirmationRequired {
                provider: request.provider,
                reason: SubscriptionInstallFallbackReason::ExplicitApprovalMissing,
            },
        );
    }
    Ok(())
}

fn check_cancelled(
    request: SubscriptionInstallRequest,
    cancellation: &CancellationToken,
) -> Result<(), SubscriptionInstallError> {
    if cancellation.is_cancelled() {
        Err(cancelled(request))
    } else {
        Ok(())
    }
}

const fn cancelled(request: SubscriptionInstallRequest) -> SubscriptionInstallError {
    SubscriptionInstallError::Cancelled {
        provider: request.provider,
        method: request.method,
    }
}

fn emit_progress<F>(
    callback: &F,
    request: SubscriptionInstallRequest,
    stage: SubscriptionInstallStage,
) where
    F: Fn(SubscriptionInstallProgress),
{
    callback(SubscriptionInstallProgress {
        provider: request.provider,
        method: request.method,
        stage,
    });
}

fn install_plan(request: SubscriptionInstallRequest, program: PathBuf) -> ProcessPlan {
    let install_spec = spec(request.provider);
    let (args, purpose) = match request.method {
        SubscriptionInstallMethod::Winget => (
            vec![
                "install",
                "--exact",
                "--id",
                install_spec.winget_package,
                "--source",
                "winget",
                "--silent",
                "--disable-interactivity",
                "--accept-package-agreements",
                "--accept-source-agreements",
            ],
            ProcessPurpose::InstallWinget,
        ),
        SubscriptionInstallMethod::VendorPowerShell => (
            vec![
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                install_spec.vendor_script,
            ],
            ProcessPurpose::InstallVendorPowerShell,
        ),
    };
    ProcessPlan {
        program,
        args,
        purpose,
        timeout: INSTALL_TIMEOUT,
        // Installer output is neither needed nor retained. Null pipes also
        // eliminate output-based memory and back-pressure failure modes.
        stdout_limit: 0,
        stderr_limit: 0,
        terminate_entire_tree_on_drop: true,
    }
}

fn parse_auth_state(spec: &InstallSpec, output: &ProcessOutput) -> SubscriptionAuthState {
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    if spec
        .signed_out_markers
        .iter()
        .any(|marker| stdout.contains(marker) || stderr.contains(marker))
    {
        SubscriptionAuthState::SignedOut
    } else if output.success {
        SubscriptionAuthState::SignedIn
    } else {
        SubscriptionAuthState::Unknown
    }
}

fn system32(process: &dyn InstallProcess) -> PathBuf {
    process.env_var("SystemRoot").map_or_else(
        || PathBuf::from(r"C:\Windows\System32"),
        |root| PathBuf::from(root).join("System32"),
    )
}

fn lookup_program(process: &dyn InstallProcess) -> PathBuf {
    #[cfg(windows)]
    return system32(process).join("where.exe");
    #[cfg(not(windows))]
    {
        let _ = process;
        PathBuf::from("/usr/bin/which")
    }
}

fn winget_candidates(process: &dyn InstallProcess) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(local) = process.env_var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("Microsoft")
                .join("WindowsApps")
                .join("winget.exe"),
        );
    }
    candidates.push(system32(process).join("winget.exe"));
    candidates
}

fn known_cli_candidates(
    process: &dyn InstallProcess,
    provider: SubscriptionAuthProvider,
) -> Vec<PathBuf> {
    let binary = spec(provider).binary;
    let executable = format!("{binary}.exe");
    let mut candidates = Vec::new();
    if let Some(local) = process.env_var("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        if provider == SubscriptionAuthProvider::Codex {
            candidates.push(
                local
                    .join("Programs")
                    .join("OpenAI")
                    .join("Codex")
                    .join("bin")
                    .join(&executable),
            );
        }
        candidates.push(
            local
                .join("Microsoft")
                .join("WinGet")
                .join("Links")
                .join(&executable),
        );
        candidates.push(
            local
                .join("Microsoft")
                .join("WindowsApps")
                .join(&executable),
        );
    }
    if provider == SubscriptionAuthProvider::ClaudeCode
        && let Some(profile) = process.env_var("USERPROFILE")
    {
        candidates.push(
            PathBuf::from(profile)
                .join(".local")
                .join("bin")
                .join(executable),
        );
    }
    candidates
}

fn binary_filename_matches(path: &Path, binary: &str) -> bool {
    let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    [
        binary.to_string(),
        format!("{binary}.exe"),
        format!("{binary}.cmd"),
        format!("{binary}.bat"),
    ]
    .iter()
    .any(|allowed| filename.eq_ignore_ascii_case(allowed))
}

struct InstallReservation {
    active: Arc<Mutex<HashSet<SubscriptionAuthProvider>>>,
    provider: SubscriptionAuthProvider,
}

impl InstallReservation {
    fn acquire(
        active: Arc<Mutex<HashSet<SubscriptionAuthProvider>>>,
        provider: SubscriptionAuthProvider,
    ) -> Result<Self, SubscriptionInstallError> {
        if !active
            .lock()
            .is_ok_and(|mut active| active.insert(provider))
        {
            return Err(SubscriptionInstallError::AlreadyInProgress { provider });
        }
        Ok(Self { active, provider })
    }
}

impl Drop for InstallReservation {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.provider);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct FakeProcess {
        calls: Mutex<Vec<ProcessPlan>>,
        file_checks: AtomicUsize,
        env: HashMap<&'static str, OsString>,
        installed: AtomicBool,
        install_success: AtomicBool,
        cancel_install: AtomicBool,
        cancel_after_install: AtomicBool,
        winget_available: AtomicBool,
        powershell_available: AtomicBool,
        status_failure: AtomicBool,
        tree_terminated: AtomicBool,
        status_output: Mutex<ProcessOutput>,
    }

    impl FakeProcess {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                file_checks: AtomicUsize::new(0),
                env: HashMap::from([
                    ("SystemRoot", OsString::from("/windows")),
                    ("LOCALAPPDATA", OsString::from("/local")),
                    ("USERPROFILE", OsString::from("/user")),
                ]),
                installed: AtomicBool::new(false),
                install_success: AtomicBool::new(true),
                cancel_install: AtomicBool::new(false),
                cancel_after_install: AtomicBool::new(false),
                winget_available: AtomicBool::new(true),
                powershell_available: AtomicBool::new(true),
                status_failure: AtomicBool::new(false),
                tree_terminated: AtomicBool::new(false),
                status_output: Mutex::new(ProcessOutput {
                    success: false,
                    stdout: b"Not logged in".to_vec(),
                    stderr: Vec::new(),
                }),
            }
        }

        fn calls(&self) -> Vec<ProcessPlan> {
            self.calls.lock().unwrap().clone()
        }

        fn install_calls(&self) -> Vec<ProcessPlan> {
            self.calls()
                .into_iter()
                .filter(|plan| {
                    matches!(
                        plan.purpose,
                        ProcessPurpose::InstallWinget | ProcessPurpose::InstallVendorPowerShell
                    )
                })
                .collect()
        }

        fn cli_path(provider: SubscriptionAuthProvider) -> PathBuf {
            match provider {
                SubscriptionAuthProvider::Codex => {
                    PathBuf::from("/local/Programs/OpenAI/Codex/bin/codex.exe")
                }
                SubscriptionAuthProvider::ClaudeCode => {
                    PathBuf::from("/user/.local/bin/claude.exe")
                }
            }
        }
    }

    impl InstallProcess for FakeProcess {
        fn env_var(&self, name: &'static str) -> Option<OsString> {
            self.env.get(name).cloned()
        }

        fn file_exists(&self, path: &Path) -> bool {
            self.file_checks.fetch_add(1, Ordering::Relaxed);
            (self.winget_available.load(Ordering::Acquire)
                && path == Path::new("/windows/System32/winget.exe"))
                || (self.powershell_available.load(Ordering::Acquire)
                    && path == Path::new("/windows/System32/WindowsPowerShell/v1.0/powershell.exe"))
                || (self.installed.load(Ordering::Acquire)
                    && [
                        Self::cli_path(SubscriptionAuthProvider::Codex),
                        Self::cli_path(SubscriptionAuthProvider::ClaudeCode),
                    ]
                    .iter()
                    .any(|candidate| candidate == path))
        }

        fn run(
            &self,
            plan: ProcessPlan,
            cancellation: CancellationToken,
        ) -> InstallFuture<'_, Result<ProcessOutput, ProcessFailure>> {
            Box::pin(async move {
                assert!(plan.terminate_entire_tree_on_drop);
                self.calls.lock().unwrap().push(plan.clone());
                match plan.purpose {
                    ProcessPurpose::Lookup => Ok(ProcessOutput {
                        success: false,
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                    }),
                    ProcessPurpose::InstallWinget | ProcessPurpose::InstallVendorPowerShell => {
                        if self.cancel_install.load(Ordering::Acquire) {
                            cancellation.cancel();
                            self.tree_terminated.store(true, Ordering::Release);
                            return Err(ProcessFailure::Cancelled);
                        }
                        let success = self.install_success.load(Ordering::Acquire);
                        if success {
                            self.installed.store(true, Ordering::Release);
                            if self.cancel_after_install.load(Ordering::Acquire) {
                                cancellation.cancel();
                            }
                        }
                        Ok(ProcessOutput {
                            success,
                            stdout: b"RAW-INSTALLER-SECRET".to_vec(),
                            stderr: b"RAW-INSTALLER-ERROR".to_vec(),
                        })
                    }
                    ProcessPurpose::Status if self.status_failure.load(Ordering::Acquire) => {
                        Err(ProcessFailure::Wait)
                    }
                    ProcessPurpose::Status => Ok(self.status_output.lock().unwrap().clone()),
                }
            })
        }
    }

    async fn install_with_fake(
        request: SubscriptionInstallRequest,
        fake: Arc<FakeProcess>,
    ) -> Result<SubscriptionInstallStatus, SubscriptionInstallError> {
        SubscriptionInstallController::with_process(fake)
            .install(request, CancellationToken::new(), |_| {})
            .await
    }

    #[tokio::test]
    async fn every_confirmation_matrix_rejects_before_any_probe() {
        for provider in [
            SubscriptionAuthProvider::Codex,
            SubscriptionAuthProvider::ClaudeCode,
        ] {
            for fallback_confirmed in [false, true] {
                let fake = Arc::new(FakeProcess::new());
                let error = install_with_fake(
                    SubscriptionInstallRequest {
                        provider,
                        method: SubscriptionInstallMethod::Winget,
                        confirmed: false,
                        fallback_confirmed,
                    },
                    fake.clone(),
                )
                .await
                .unwrap_err();
                assert!(matches!(
                    error,
                    SubscriptionInstallError::ConfirmationRequired { .. }
                ));
                assert!(fake.calls().is_empty());
                assert_eq!(fake.file_checks.load(Ordering::Relaxed), 0);
            }

            for (confirmed, fallback_confirmed, expected_primary) in [
                (false, false, true),
                (false, true, true),
                (true, false, false),
            ] {
                let fake = Arc::new(FakeProcess::new());
                let error = install_with_fake(
                    SubscriptionInstallRequest::vendor_fallback(
                        provider,
                        confirmed,
                        fallback_confirmed,
                    ),
                    fake.clone(),
                )
                .await
                .unwrap_err();
                if expected_primary {
                    assert!(matches!(
                        error,
                        SubscriptionInstallError::ConfirmationRequired { .. }
                    ));
                } else {
                    assert_eq!(
                        error,
                        SubscriptionInstallError::VendorFallbackConfirmationRequired {
                            provider,
                            reason: SubscriptionInstallFallbackReason::ExplicitApprovalMissing,
                        }
                    );
                }
                assert!(fake.calls().is_empty());
                assert_eq!(fake.file_checks.load(Ordering::Relaxed), 0);
            }
        }
    }

    #[tokio::test]
    async fn fixed_allowlisted_commands_and_budgets_cover_every_provider_and_method() {
        for (provider, package, script, status_args) in [
            (
                SubscriptionAuthProvider::Codex,
                "OpenAI.Codex",
                "$env:CODEX_NON_INTERACTIVE = '1'; irm https://chatgpt.com/codex/install.ps1 | iex",
                vec!["login", "status"],
            ),
            (
                SubscriptionAuthProvider::ClaudeCode,
                "Anthropic.ClaudeCode",
                "irm https://claude.ai/install.ps1 | iex",
                vec!["auth", "status"],
            ),
        ] {
            for method in [
                SubscriptionInstallMethod::Winget,
                SubscriptionInstallMethod::VendorPowerShell,
            ] {
                let fake = Arc::new(FakeProcess::new());
                let request = match method {
                    SubscriptionInstallMethod::Winget => {
                        SubscriptionInstallRequest::winget(provider, true)
                    }
                    SubscriptionInstallMethod::VendorPowerShell => {
                        SubscriptionInstallRequest::vendor_fallback(provider, true, true)
                    }
                };
                let status = install_with_fake(request, fake.clone()).await.unwrap();
                assert_eq!(status.path, FakeProcess::cli_path(provider));
                assert_eq!(status.state, SubscriptionAuthState::SignedOut);
                assert_eq!(status.auth_status().path, Some(status.path.clone()));

                let installs = fake.install_calls();
                assert_eq!(installs.len(), 1);
                let install = &installs[0];
                assert_eq!(install.timeout, Duration::from_secs(600));
                assert_eq!(install.stdout_limit, 0);
                assert_eq!(install.stderr_limit, 0);
                assert!(install.terminate_entire_tree_on_drop);
                match method {
                    SubscriptionInstallMethod::Winget => assert_eq!(
                        install.args,
                        vec![
                            "install",
                            "--exact",
                            "--id",
                            package,
                            "--source",
                            "winget",
                            "--silent",
                            "--disable-interactivity",
                            "--accept-package-agreements",
                            "--accept-source-agreements",
                        ]
                    ),
                    SubscriptionInstallMethod::VendorPowerShell => assert_eq!(
                        install.args,
                        vec![
                            "-NoLogo",
                            "-NoProfile",
                            "-NonInteractive",
                            "-ExecutionPolicy",
                            "Bypass",
                            "-Command",
                            script,
                        ]
                    ),
                }
                let calls = fake.calls();
                let status_plan = calls
                    .iter()
                    .find(|plan| plan.purpose == ProcessPurpose::Status)
                    .unwrap();
                assert_eq!(status_plan.args, status_args);
                assert_eq!(status_plan.timeout, Duration::from_secs(10));
                assert_eq!(
                    status_plan.stdout_limit + status_plan.stderr_limit,
                    16 * 1024
                );
                assert!(
                    !status_plan.args.contains(&"login")
                        || provider == SubscriptionAuthProvider::Codex
                );
                assert_ne!(status_plan.args, vec!["auth", "login"]);
            }
        }
    }

    #[tokio::test]
    async fn winget_failure_requires_a_second_confirmation_and_never_auto_falls_back() {
        let fake = Arc::new(FakeProcess::new());
        fake.install_success.store(false, Ordering::Release);
        let error = install_with_fake(
            SubscriptionInstallRequest::winget(SubscriptionAuthProvider::Codex, true),
            fake.clone(),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error,
            SubscriptionInstallError::VendorFallbackConfirmationRequired {
                provider: SubscriptionAuthProvider::Codex,
                reason: SubscriptionInstallFallbackReason::WingetFailed,
            }
        );
        let installs = fake.install_calls();
        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].purpose, ProcessPurpose::InstallWinget);
    }

    #[tokio::test]
    async fn missing_winget_is_structured_and_does_not_start_any_installer() {
        let fake = Arc::new(FakeProcess::new());
        fake.winget_available.store(false, Ordering::Release);
        let error = install_with_fake(
            SubscriptionInstallRequest::winget(SubscriptionAuthProvider::ClaudeCode, true),
            fake.clone(),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error,
            SubscriptionInstallError::VendorFallbackConfirmationRequired {
                provider: SubscriptionAuthProvider::ClaudeCode,
                reason: SubscriptionInstallFallbackReason::WingetUnavailable,
            }
        );
        assert!(fake.install_calls().is_empty());
    }

    #[tokio::test]
    async fn pre_cancelled_request_never_starts_a_probe_or_process() {
        let fake = Arc::new(FakeProcess::new());
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error = SubscriptionInstallController::with_process(fake.clone())
            .install(
                SubscriptionInstallRequest::winget(SubscriptionAuthProvider::Codex, true),
                cancellation,
                |_| {},
            )
            .await
            .unwrap_err();

        assert!(matches!(error, SubscriptionInstallError::Cancelled { .. }));
        assert!(fake.calls().is_empty());
        assert_eq!(fake.file_checks.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn an_existing_verified_cli_runs_status_but_never_an_installer() {
        let fake = Arc::new(FakeProcess::new());
        fake.installed.store(true, Ordering::Release);
        let status = install_with_fake(
            SubscriptionInstallRequest::winget(SubscriptionAuthProvider::Codex, true),
            fake.clone(),
        )
        .await
        .unwrap();

        assert_eq!(
            status.path,
            FakeProcess::cli_path(SubscriptionAuthProvider::Codex)
        );
        assert!(fake.install_calls().is_empty());
        assert_eq!(
            fake.calls()
                .iter()
                .filter(|plan| plan.purpose == ProcessPurpose::Status)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn post_install_status_failure_is_sanitized_verification_failure() {
        let fake = Arc::new(FakeProcess::new());
        fake.status_failure.store(true, Ordering::Release);
        let error = install_with_fake(
            SubscriptionInstallRequest::vendor_fallback(
                SubscriptionAuthProvider::ClaudeCode,
                true,
                true,
            ),
            fake,
        )
        .await
        .unwrap_err();

        assert_eq!(
            error,
            SubscriptionInstallError::VerificationFailed {
                provider: SubscriptionAuthProvider::ClaudeCode,
            }
        );
        assert!(!error.to_string().contains("stdout"));
        assert!(!error.to_string().contains("stderr"));
    }

    #[tokio::test]
    async fn cancellation_uses_the_entire_tree_termination_contract() {
        let fake = Arc::new(FakeProcess::new());
        fake.cancel_install.store(true, Ordering::Release);
        let error = install_with_fake(
            SubscriptionInstallRequest::winget(SubscriptionAuthProvider::ClaudeCode, true),
            fake.clone(),
        )
        .await
        .unwrap_err();

        assert!(matches!(error, SubscriptionInstallError::Cancelled { .. }));
        assert!(fake.tree_terminated.load(Ordering::Acquire));
        assert!(
            fake.calls()
                .iter()
                .all(|plan| plan.terminate_entire_tree_on_drop)
        );
    }

    #[tokio::test]
    async fn cancellation_after_installer_commit_does_not_relabel_success() {
        let fake = Arc::new(FakeProcess::new());
        fake.cancel_after_install.store(true, Ordering::Release);
        let status = install_with_fake(
            SubscriptionInstallRequest::winget(SubscriptionAuthProvider::Codex, true),
            fake,
        )
        .await
        .unwrap();

        assert_eq!(status.provider, SubscriptionAuthProvider::Codex);
        assert_eq!(status.state, SubscriptionAuthState::SignedOut);
    }

    #[tokio::test]
    async fn raw_child_output_never_crosses_errors_or_progress() {
        let fake = Arc::new(FakeProcess::new());
        fake.install_success.store(false, Ordering::Release);
        let progress = Arc::new(Mutex::new(Vec::new()));
        let captured = progress.clone();
        let error = SubscriptionInstallController::with_process(fake)
            .install(
                SubscriptionInstallRequest::vendor_fallback(
                    SubscriptionAuthProvider::Codex,
                    true,
                    true,
                ),
                CancellationToken::new(),
                move |event| captured.lock().unwrap().push(event),
            )
            .await
            .unwrap_err();

        let rendered = error.to_string();
        assert!(!rendered.contains("RAW-INSTALLER-SECRET"));
        assert!(!rendered.contains("RAW-INSTALLER-ERROR"));
        let serialized = serde_json::to_string(&*progress.lock().unwrap()).unwrap();
        assert!(!serialized.contains("RAW"));
    }

    #[tokio::test]
    async fn progress_is_ordered_and_contains_only_static_stages() {
        let fake = Arc::new(FakeProcess::new());
        let stages = Arc::new(Mutex::new(Vec::new()));
        let captured = stages.clone();
        install_with_progress(fake, move |event| {
            captured.lock().unwrap().push(event.stage);
        })
        .await
        .unwrap();

        assert_eq!(
            *stages.lock().unwrap(),
            [
                SubscriptionInstallStage::CheckingExisting,
                SubscriptionInstallStage::ResolvingInstaller,
                SubscriptionInstallStage::InstallingWinget,
                SubscriptionInstallStage::Verifying,
                SubscriptionInstallStage::Completed,
            ]
        );
    }

    async fn install_with_progress<F>(
        fake: Arc<FakeProcess>,
        progress: F,
    ) -> Result<SubscriptionInstallStatus, SubscriptionInstallError>
    where
        F: Fn(SubscriptionInstallProgress) + Send + Sync,
    {
        SubscriptionInstallController::with_process(fake)
            .install(
                SubscriptionInstallRequest::winget(SubscriptionAuthProvider::Codex, true),
                CancellationToken::new(),
                progress,
            )
            .await
    }

    #[test]
    fn public_errors_are_structured_and_never_accept_child_strings() {
        let error = SubscriptionInstallError::InstallFailed {
            provider: SubscriptionAuthProvider::ClaudeCode,
            method: SubscriptionInstallMethod::VendorPowerShell,
        };
        assert_eq!(
            error.to_string(),
            "The Claude Code installation with the vendor PowerShell installer did not complete."
        );
    }

    #[test]
    fn output_drain_never_retains_more_than_its_fixed_budget() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let bytes = vec![b'x'; 50_000];
            let retained = drain_bounded(Some(&bytes[..]), 1_024).await.unwrap();
            assert_eq!(retained.len(), 1_024);
        });
    }
}
