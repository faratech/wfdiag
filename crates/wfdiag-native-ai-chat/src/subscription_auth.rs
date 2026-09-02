//! Explicit, UI-framework-neutral subscription CLI account actions.
//!
//! The genuine Codex and Claude Code CLIs remain the sole owners of their
//! credentials. This adapter never reads credential files or returns child
//! output. Construction has no side effects, status performs only executable
//! resolution plus the vendor's status command, and account mutations happen
//! only through explicit `sign_in` / `sign_out` calls.

use crate::cli_bridge;
#[cfg(unix)]
use process_wrap::tokio::ProcessSession;
use process_wrap::tokio::{CommandWrap, KillOnDrop};
#[cfg(windows)]
use process_wrap::tokio::{CreationFlags, JobObject};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_provider::{
    CliObstacle, CliProbeSnapshot, ProcessSubscriptionCliStatusSource, SubscriptionCli,
    SubscriptionCliSpec, SubscriptionCliStatusSource, is_batch_shim, subscription_cli_spec,
};
#[cfg(windows)]
use windows::Win32::System::Threading::CREATE_NEW_CONSOLE;

/// The vendor's sign-in runs in its own console window; a user reading a
/// device code or a browser page needs minutes, not seconds.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);
const SIGN_OUT_TIMEOUT: Duration = Duration::from_secs(15);

type AuthFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Subscription-backed vendors whose genuine CLIs can own account state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionAuthProvider {
    Codex,
    ClaudeCode,
}

impl SubscriptionAuthProvider {
    /// The one shared description of this CLI.
    #[must_use]
    pub const fn spec(self) -> &'static SubscriptionCliSpec {
        subscription_cli_spec(match self {
            Self::Codex => SubscriptionCli::Codex,
            Self::ClaudeCode => SubscriptionCli::ClaudeCode,
        })
    }
}

impl From<SubscriptionCli> for SubscriptionAuthProvider {
    fn from(provider: SubscriptionCli) -> Self {
        match provider {
            SubscriptionCli::Codex => Self::Codex,
            SubscriptionCli::ClaudeCode => Self::ClaudeCode,
        }
    }
}

impl From<SubscriptionAuthProvider> for SubscriptionCli {
    fn from(provider: SubscriptionAuthProvider) -> Self {
        match provider {
            SubscriptionAuthProvider::Codex => Self::Codex,
            SubscriptionAuthProvider::ClaudeCode => Self::ClaudeCode,
        }
    }
}

impl fmt::Display for SubscriptionAuthProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
        })
    }
}

/// Conclusive or safely indeterminate account state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionAuthState {
    NotInstalled,
    SignedOut,
    SignedIn,
    /// The executable exists, but its status command failed or timed out
    /// without emitting a recognized signed-out marker.
    Unknown,
}

/// Public account status. No vendor output or credential material crosses
/// this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionAuthStatus {
    pub provider: SubscriptionAuthProvider,
    pub state: SubscriptionAuthState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Why an installed CLI is not usable (the probe's finding).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obstacle: Option<CliObstacle>,
}

impl SubscriptionAuthStatus {
    #[must_use]
    pub const fn installed(&self) -> bool {
        self.path.is_some()
    }

    /// Whether signing in is the next step: signed out, or unclear for a
    /// reason sign-in can fix. A shim-only install needs the native CLI first.
    #[must_use]
    pub const fn needs_sign_in(&self) -> bool {
        match self.state {
            SubscriptionAuthState::SignedOut => true,
            SubscriptionAuthState::Unknown => match self.obstacle {
                Some(obstacle) => obstacle.needs_sign_in(),
                None => true,
            },
            SubscriptionAuthState::NotInstalled | SubscriptionAuthState::SignedIn => false,
        }
    }

    #[must_use]
    pub const fn signed_in(&self) -> bool {
        matches!(self.state, SubscriptionAuthState::SignedIn)
    }
}

/// The only commands this module can run against a subscription CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionAuthOperation {
    Status,
    SignIn,
    SignOut,
}

/// Sanitized failures suitable for a UI boundary.
///
/// These variants intentionally carry no stdout, stderr, OS error text, or
/// CLI-provided strings. In particular, vendor output can never smuggle an
/// inherited token into a rendered error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionAuthError {
    InvalidCliPath {
        provider: SubscriptionAuthProvider,
    },
    NotInstalled {
        provider: SubscriptionAuthProvider,
    },
    Cancelled {
        provider: SubscriptionAuthProvider,
        operation: SubscriptionAuthOperation,
    },
    AlreadyInProgress {
        provider: SubscriptionAuthProvider,
    },
    SignInFailed {
        provider: SubscriptionAuthProvider,
    },
    OperationUnavailable {
        provider: SubscriptionAuthProvider,
        operation: SubscriptionAuthOperation,
    },
    /// Only an npm script shim of the CLI exists; `WFDiag` cannot run it.
    BatchShimOnly {
        provider: SubscriptionAuthProvider,
    },
    /// The vendor's sign-in window stayed open past the cap and was closed.
    SignInTimedOut {
        provider: SubscriptionAuthProvider,
    },
}

impl fmt::Display for SubscriptionAuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCliPath { provider } => write!(
                formatter,
                "The configured {provider} CLI path must be an absolute path to an existing file."
            ),
            Self::NotInstalled { provider } => {
                write!(
                    formatter,
                    "The {provider} CLI is not installed or could not be found."
                )
            }
            Self::Cancelled {
                provider,
                operation,
            } => write!(
                formatter,
                "{} for {provider} was cancelled.",
                operation.present_participle()
            ),
            Self::AlreadyInProgress { provider } => {
                write!(
                    formatter,
                    "An account action for {provider} is already in progress."
                )
            }
            Self::SignInFailed { provider } => write!(
                formatter,
                "{provider} did not complete sign-in. Retry the vendor's sign-in flow."
            ),
            Self::OperationUnavailable {
                provider,
                operation,
            } => write!(
                formatter,
                "{} for {provider} could not be completed.",
                operation.noun()
            ),
            Self::BatchShimOnly { provider } => write!(
                formatter,
                "Only the npm script shim of the {provider} CLI was found; WFDiag cannot run it. Install the native CLI from Settings."
            ),
            Self::SignInTimedOut { provider } => write!(
                formatter,
                "The {provider} sign-in window was still open after 10 minutes and was closed. Try again."
            ),
        }
    }
}

impl std::error::Error for SubscriptionAuthError {}

impl SubscriptionAuthOperation {
    const fn noun(self) -> &'static str {
        match self {
            Self::Status => "Status check",
            Self::SignIn => "Sign-in",
            Self::SignOut => "Sign-out",
        }
    }

    const fn present_participle(self) -> &'static str {
        match self {
            Self::Status => "Checking status",
            Self::SignIn => "Signing in",
            Self::SignOut => "Signing out",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessAction {
    SignIn,
    SignOut,
}

impl ProcessAction {
    const fn timeout(self) -> Duration {
        match self {
            Self::SignIn => SIGN_IN_TIMEOUT,
            Self::SignOut => SIGN_OUT_TIMEOUT,
        }
    }

    const fn safe_label(self) -> &'static str {
        match self {
            Self::SignIn => "subscription sign-in",
            Self::SignOut => "subscription sign-out",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessRequest {
    program: PathBuf,
    args: Vec<&'static str>,
    action: ProcessAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// How an interactive (console) child ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExitOutcome {
    success: bool,
}

/// Why an interactive child did not end normally. No child output exists
/// for these: the console is the user's, not ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractiveFailure {
    Spawn,
    Cancelled,
    TimedOut,
    Wait,
}

trait AuthProcess: Send + Sync + 'static {
    fn resolve(
        &self,
        binary: &'static str,
        draft_path: Option<String>,
    ) -> AuthFuture<'_, Result<PathBuf, ResolveFailure>>;

    /// A hidden, output-capturing child (sign-out).
    fn run(&self, request: ProcessRequest) -> AuthFuture<'_, Result<ProcessOutput, ()>>;

    /// A visible child in its own console the user interacts with (sign-in);
    /// only its exit matters. Cancelling kills the whole process tree.
    fn run_interactive(
        &self,
        request: ProcessRequest,
        cancellation: CancellationToken,
    ) -> AuthFuture<'_, Result<ExitOutcome, InteractiveFailure>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolveFailure {
    InvalidDraft,
    NotFound,
    Unknown,
}

#[derive(Debug, Default)]
struct TokioAuthProcess;

impl AuthProcess for TokioAuthProcess {
    fn resolve(
        &self,
        binary: &'static str,
        draft_path: Option<String>,
    ) -> AuthFuture<'_, Result<PathBuf, ResolveFailure>> {
        Box::pin(async move {
            let has_draft = draft_path.is_some();
            cli_bridge::resolve_cli(binary, draft_path.as_deref())
                .await
                .map_err(|error| {
                    if has_draft {
                        ResolveFailure::InvalidDraft
                    } else if error.contains("was not found on PATH") {
                        ResolveFailure::NotFound
                    } else {
                        ResolveFailure::Unknown
                    }
                })
        })
    }

    fn run(&self, request: ProcessRequest) -> AuthFuture<'_, Result<ProcessOutput, ()>> {
        Box::pin(async move {
            let mut command = tokio::process::Command::new(request.program);
            command.args(request.args);
            cli_bridge::run_headless(
                command,
                None,
                request.action.timeout(),
                request.action.safe_label(),
            )
            .await
            .map(|output| ProcessOutput {
                success: output.status.success(),
                stdout: output.stdout,
                stderr: output.stderr,
            })
            .map_err(|_| ())
        })
    }

    fn run_interactive(
        &self,
        request: ProcessRequest,
        cancellation: CancellationToken,
    ) -> AuthFuture<'_, Result<ExitOutcome, InteractiveFailure>> {
        Box::pin(async move { run_console_process(request, cancellation).await })
    }
}

/// Run the vendor CLI in a console window of its own and wait for it to
/// exit. This is the one deliberately visible child: `codex login` prints a
/// URL or device code and `claude auth login` is interactive, so a hidden
/// process with no stdin could never finish. Inherited standard handles are
/// all null in a GUI parent, so the child attaches to the new console. The
/// Job Object (Windows) / session (Unix) plus `KillOnDrop` terminate the
/// whole tree when the future is dropped by a cancel or the cap.
async fn run_console_process(
    request: ProcessRequest,
    cancellation: CancellationToken,
) -> Result<ExitOutcome, InteractiveFailure> {
    let mut command = tokio::process::Command::new(request.program);
    command.args(request.args);
    for variable in cli_bridge::SUBSCRIPTION_OVERRIDE_ENV_VARS {
        command.env_remove(variable);
    }
    let workdir = std::env::temp_dir();
    if workdir.is_dir() {
        command.current_dir(workdir);
    }
    #[cfg(windows)]
    {
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    }
    #[cfg(not(windows))]
    {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }
    let mut wrapped = CommandWrap::from(command);
    wrapped.wrap(KillOnDrop);
    #[cfg(windows)]
    {
        wrapped.wrap(CreationFlags(CREATE_NEW_CONSOLE));
        wrapped.wrap(JobObject);
    }
    #[cfg(unix)]
    wrapped.wrap(ProcessSession);

    let mut child = wrapped.spawn().map_err(|_| InteractiveFailure::Spawn)?;
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(InteractiveFailure::Cancelled),
        waited = tokio::time::timeout(request.action.timeout(), child.wait()) => {
            match waited {
                Ok(Ok(status)) => Ok(ExitOutcome { success: status.success() }),
                Ok(Err(_)) => Err(InteractiveFailure::Wait),
                Err(_elapsed) => Err(InteractiveFailure::TimedOut),
            }
        }
    }
    // `child` drops here on every path; the job / session and KillOnDrop
    // take the rest of the tree with it.
}

/// Concrete account controller shared by desktop shells.
///
/// `new` is side-effect-free. There is intentionally no installation method:
/// the UI may explain how to install a vendor CLI, but this boundary cannot do
/// so implicitly (or explicitly). API-key override variables are scrubbed,
/// the sign-out child is hidden and output-capturing while the sign-in child
/// is the one deliberately visible console window; both are time bounded and
/// killed (with their process tree) when a cancelled future is dropped.
#[derive(Clone)]
pub struct SubscriptionAuthController {
    /// The shared, cached status probe (credential-store fast path included).
    probe: Arc<dyn SubscriptionCliStatusSource>,
    process: Arc<dyn AuthProcess>,
    active_mutations: Arc<Mutex<HashSet<SubscriptionAuthProvider>>>,
}

impl fmt::Debug for SubscriptionAuthController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubscriptionAuthController")
            .finish_non_exhaustive()
    }
}

impl Default for SubscriptionAuthController {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriptionAuthController {
    #[must_use]
    pub fn new() -> Self {
        Self {
            probe: Arc::new(ProcessSubscriptionCliStatusSource::new()),
            process: Arc::new(TokioAuthProcess),
            active_mutations: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    #[cfg(test)]
    fn with_dependencies(
        probe: Arc<dyn SubscriptionCliStatusSource>,
        process: Arc<dyn AuthProcess>,
    ) -> Self {
        Self {
            probe,
            process,
            active_mutations: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// The account state, from the shared probe: a missing CLI is a normal
    /// `NotInstalled`; a missing credential cache or a shim-only install is
    /// decided without spawning; a present CLI whose status cannot be
    /// established is `Unknown`, never falsely reported as signed out.
    ///
    /// A non-blank draft path always wins and must be an absolute path to an
    /// existing file.
    pub async fn status(
        &self,
        provider: SubscriptionAuthProvider,
        draft_cli_path: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<SubscriptionAuthStatus, SubscriptionAuthError> {
        let draft = normalized_absolute_draft(provider, draft_cli_path)?;
        if draft
            .as_deref()
            .is_some_and(|path| !Path::new(path).is_file())
        {
            return Err(SubscriptionAuthError::InvalidCliPath { provider });
        }
        let probe = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                return Err(SubscriptionAuthError::Cancelled {
                    provider,
                    operation: SubscriptionAuthOperation::Status,
                });
            }
            probe = self.probe.probe(provider.into(), draft) => probe,
        };
        Ok(auth_status_from_probe(provider, &probe))
    }

    /// Explicitly run the vendor CLI's own browser-based sign-in flow.
    ///
    /// The caller owns cancellation. Cancelling drops the bounded process
    /// future, which kills the child; credentials are never observed here.
    pub async fn sign_in(
        &self,
        provider: SubscriptionAuthProvider,
        draft_cli_path: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<SubscriptionAuthStatus, SubscriptionAuthError> {
        self.mutate(
            provider,
            draft_cli_path,
            ProcessAction::SignIn,
            cancellation,
        )
        .await
    }

    /// Explicitly ask the vendor CLI to remove its own stored credentials.
    pub async fn sign_out(
        &self,
        provider: SubscriptionAuthProvider,
        draft_cli_path: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<SubscriptionAuthStatus, SubscriptionAuthError> {
        self.mutate(
            provider,
            draft_cli_path,
            ProcessAction::SignOut,
            cancellation,
        )
        .await
    }

    async fn mutate(
        &self,
        provider: SubscriptionAuthProvider,
        draft_cli_path: Option<&str>,
        action: ProcessAction,
        cancellation: CancellationToken,
    ) -> Result<SubscriptionAuthStatus, SubscriptionAuthError> {
        let draft = normalized_absolute_draft(provider, draft_cli_path)?;
        let operation = match action {
            ProcessAction::SignIn => SubscriptionAuthOperation::SignIn,
            ProcessAction::SignOut => SubscriptionAuthOperation::SignOut,
        };
        let _reservation = MutationReservation::acquire(self.active_mutations.clone(), provider)?;
        let spec = provider.spec();
        let resolved = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                return Err(SubscriptionAuthError::Cancelled { provider, operation });
            }
            result = self.process.resolve(spec.binary, draft.clone()) => result,
        };
        let path = resolved.map_err(|failure| match failure {
            ResolveFailure::InvalidDraft => SubscriptionAuthError::InvalidCliPath { provider },
            ResolveFailure::NotFound => SubscriptionAuthError::NotInstalled { provider },
            ResolveFailure::Unknown => SubscriptionAuthError::OperationUnavailable {
                provider,
                operation,
            },
        })?;
        if is_batch_shim(&path) {
            return Err(SubscriptionAuthError::BatchShimOnly { provider });
        }
        let args = match action {
            ProcessAction::SignIn => spec.sign_in_args,
            ProcessAction::SignOut => spec.sign_out_args,
        };
        let request = ProcessRequest {
            program: path,
            args: args.to_vec(),
            action,
        };
        match action {
            ProcessAction::SignIn => {
                // The vendor's own flow, in its own console; we only learn
                // whether it ended well.
                let outcome = self
                    .process
                    .run_interactive(request, cancellation)
                    .await
                    .map_err(|failure| match failure {
                        InteractiveFailure::Cancelled => SubscriptionAuthError::Cancelled {
                            provider,
                            operation,
                        },
                        InteractiveFailure::TimedOut => {
                            SubscriptionAuthError::SignInTimedOut { provider }
                        }
                        InteractiveFailure::Spawn | InteractiveFailure::Wait => {
                            SubscriptionAuthError::OperationUnavailable {
                                provider,
                                operation,
                            }
                        }
                    })?;
                if !outcome.success {
                    return Err(SubscriptionAuthError::SignInFailed { provider });
                }
            }
            ProcessAction::SignOut => {
                tokio::select! {
                    biased;
                    () = cancellation.cancelled() => {
                        return Err(SubscriptionAuthError::Cancelled { provider, operation });
                    }
                    result = self.process.run(request) => result,
                }
                .map_err(|()| SubscriptionAuthError::OperationUnavailable {
                    provider,
                    operation,
                })?;
            }
        }
        // The vendor child completing is the mutation's commit point. A late
        // UI cancellation must not report the already-committed credential
        // change as cancelled. The truthful follow-up probe remains bounded by
        // the probe's own timeout, but intentionally gets a fresh token.
        // Logout on an already signed-out CLI may exit non-zero, so this fresh
        // status remains the source of truth — fresh, so the probe's cached
        // answer from before the change is dropped first.
        self.probe.invalidate(provider.into());
        self.status(provider, draft.as_deref(), CancellationToken::new())
            .await
    }
}

/// The account state a probe result means.
#[must_use]
pub fn auth_status_from_probe(
    provider: SubscriptionAuthProvider,
    probe: &CliProbeSnapshot,
) -> SubscriptionAuthStatus {
    let state = if !probe.installed {
        SubscriptionAuthState::NotInstalled
    } else if probe.usable {
        SubscriptionAuthState::SignedIn
    } else {
        match probe.obstacle {
            Some(CliObstacle::NoStoredLogin | CliObstacle::SignedOut) => {
                SubscriptionAuthState::SignedOut
            }
            Some(CliObstacle::BatchShimOnly | CliObstacle::StatusUnclear) | None => {
                SubscriptionAuthState::Unknown
            }
        }
    };
    SubscriptionAuthStatus {
        provider,
        state,
        path: probe
            .installed
            .then(|| PathBuf::from(probe.path.as_deref().unwrap_or_default())),
        obstacle: if probe.usable { None } else { probe.obstacle },
    }
}

fn normalized_absolute_draft(
    provider: SubscriptionAuthProvider,
    draft_cli_path: Option<&str>,
) -> Result<Option<String>, SubscriptionAuthError> {
    let draft = draft_cli_path
        .map(str::trim)
        .filter(|path| !path.is_empty());
    if let Some(path) = draft {
        if !Path::new(path).is_absolute() {
            return Err(SubscriptionAuthError::InvalidCliPath { provider });
        }
        Ok(Some(path.to_string()))
    } else {
        Ok(None)
    }
}

struct MutationReservation {
    active: Arc<Mutex<HashSet<SubscriptionAuthProvider>>>,
    provider: SubscriptionAuthProvider,
}

impl MutationReservation {
    fn acquire(
        active: Arc<Mutex<HashSet<SubscriptionAuthProvider>>>,
        provider: SubscriptionAuthProvider,
    ) -> Result<Self, SubscriptionAuthError> {
        let inserted = active
            .lock()
            .is_ok_and(|mut active| active.insert(provider));
        if !inserted {
            return Err(SubscriptionAuthError::AlreadyInProgress { provider });
        }
        Ok(Self { active, provider })
    }
}

impl Drop for MutationReservation {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.provider);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct FakeProcess {
        resolutions: Mutex<VecDeque<Result<PathBuf, ResolveFailure>>>,
        outputs: Mutex<VecDeque<Result<ProcessOutput, ()>>>,
        requests: Mutex<Vec<ProcessRequest>>,
        /// Requests that ran in the visible console (sign-in).
        interactive: Mutex<Vec<ProcessRequest>>,
        /// Scripted answers for `run_interactive` when set; otherwise the
        /// next `outputs` entry is turned into an exit outcome.
        interactive_failures: Mutex<VecDeque<InteractiveFailure>>,
        resolve_calls: Mutex<Vec<(&'static str, Option<String>)>>,
        cancel_after_next_run: Mutex<Option<CancellationToken>>,
    }

    impl FakeProcess {
        fn scripted(
            resolutions: impl IntoIterator<Item = Result<PathBuf, ResolveFailure>>,
            outputs: impl IntoIterator<Item = Result<ProcessOutput, ()>>,
        ) -> Self {
            Self {
                resolutions: Mutex::new(resolutions.into_iter().collect()),
                outputs: Mutex::new(outputs.into_iter().collect()),
                requests: Mutex::new(Vec::new()),
                interactive: Mutex::new(Vec::new()),
                interactive_failures: Mutex::new(VecDeque::new()),
                resolve_calls: Mutex::new(Vec::new()),
                cancel_after_next_run: Mutex::new(None),
            }
        }

        fn requests(&self) -> Vec<ProcessRequest> {
            self.requests.lock().unwrap().clone()
        }

        fn cancel_after_next_run(&self, cancellation: CancellationToken) {
            *self.cancel_after_next_run.lock().unwrap() = Some(cancellation);
        }

        fn interactive_requests(&self) -> Vec<ProcessRequest> {
            self.interactive.lock().unwrap().clone()
        }

        fn fail_interactive(&self, failure: InteractiveFailure) {
            self.interactive_failures.lock().unwrap().push_back(failure);
        }
    }

    impl AuthProcess for FakeProcess {
        fn resolve(
            &self,
            binary: &'static str,
            draft_path: Option<String>,
        ) -> AuthFuture<'_, Result<PathBuf, ResolveFailure>> {
            Box::pin(async move {
                self.resolve_calls
                    .lock()
                    .unwrap()
                    .push((binary, draft_path));
                self.resolutions
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("fake resolution exhausted")
            })
        }

        fn run(&self, request: ProcessRequest) -> AuthFuture<'_, Result<ProcessOutput, ()>> {
            Box::pin(async move {
                self.requests.lock().unwrap().push(request);
                let output = self
                    .outputs
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("fake output exhausted");
                if let Some(cancellation) = self.cancel_after_next_run.lock().unwrap().take() {
                    cancellation.cancel();
                }
                output
            })
        }

        fn run_interactive(
            &self,
            request: ProcessRequest,
            _cancellation: CancellationToken,
        ) -> AuthFuture<'_, Result<ExitOutcome, InteractiveFailure>> {
            Box::pin(async move {
                self.requests.lock().unwrap().push(request.clone());
                self.interactive.lock().unwrap().push(request);
                if let Some(failure) = self.interactive_failures.lock().unwrap().pop_front() {
                    return Err(failure);
                }
                let output = self
                    .outputs
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("fake output exhausted");
                if let Some(cancellation) = self.cancel_after_next_run.lock().unwrap().take() {
                    cancellation.cancel();
                }
                output
                    .map(|output| ExitOutcome {
                        success: output.success,
                    })
                    .map_err(|()| InteractiveFailure::Spawn)
            })
        }
    }

    fn output(success: bool, stdout: &str, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            success,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    /// A shared probe with a scripted answer that records its calls.
    struct FakeProbe {
        snapshot: Mutex<CliProbeSnapshot>,
        calls: Mutex<Vec<(SubscriptionCli, Option<String>)>>,
        invalidations: Mutex<Vec<SubscriptionCli>>,
    }

    impl FakeProbe {
        fn answering(snapshot: CliProbeSnapshot) -> Arc<Self> {
            Arc::new(Self {
                snapshot: Mutex::new(snapshot),
                calls: Mutex::new(Vec::new()),
                invalidations: Mutex::new(Vec::new()),
            })
        }

        fn calls(&self) -> Vec<(SubscriptionCli, Option<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl SubscriptionCliStatusSource for FakeProbe {
        fn probe(
            &self,
            provider: SubscriptionCli,
            configured_path: Option<String>,
        ) -> wfdiag_native_ai_provider::BackendFuture<'_, CliProbeSnapshot> {
            Box::pin(async move {
                self.calls.lock().unwrap().push((provider, configured_path));
                self.snapshot.lock().unwrap().clone()
            })
        }

        fn invalidate(&self, provider: SubscriptionCli) {
            self.invalidations.lock().unwrap().push(provider);
        }
    }

    fn installed(usable: bool, obstacle: Option<CliObstacle>) -> CliProbeSnapshot {
        CliProbeSnapshot {
            usable,
            installed: true,
            path: Some("/opt/vendor".to_string()),
            obstacle,
        }
    }

    fn controller(
        probe: &Arc<FakeProbe>,
        process: &Arc<FakeProcess>,
    ) -> SubscriptionAuthController {
        SubscriptionAuthController::with_dependencies(probe.clone(), process.clone())
    }

    #[tokio::test]
    async fn status_delegates_to_the_shared_probe_and_maps_every_obstacle() {
        let cases = [
            (installed(true, None), SubscriptionAuthState::SignedIn, None),
            (
                installed(false, Some(CliObstacle::NoStoredLogin)),
                SubscriptionAuthState::SignedOut,
                Some(CliObstacle::NoStoredLogin),
            ),
            (
                installed(false, Some(CliObstacle::SignedOut)),
                SubscriptionAuthState::SignedOut,
                Some(CliObstacle::SignedOut),
            ),
            (
                installed(false, Some(CliObstacle::StatusUnclear)),
                SubscriptionAuthState::Unknown,
                Some(CliObstacle::StatusUnclear),
            ),
            (
                installed(false, Some(CliObstacle::BatchShimOnly)),
                SubscriptionAuthState::Unknown,
                Some(CliObstacle::BatchShimOnly),
            ),
        ];
        for (snapshot, expected_state, expected_obstacle) in cases {
            let probe = FakeProbe::answering(snapshot);
            let process = Arc::new(FakeProcess::default());
            let status = controller(&probe, &process)
                .status(
                    SubscriptionAuthProvider::ClaudeCode,
                    None,
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            assert_eq!(status.state, expected_state);
            assert_eq!(status.obstacle, expected_obstacle);
            assert_eq!(status.path.as_deref(), Some(Path::new("/opt/vendor")));
            assert_eq!(probe.calls(), [(SubscriptionCli::ClaudeCode, None)]);
            assert!(
                process.requests().is_empty(),
                "status never spawns the CLI itself"
            );
        }
    }

    #[tokio::test]
    async fn no_stored_login_status_never_spawns_the_cli() {
        let probe = FakeProbe::answering(installed(false, Some(CliObstacle::NoStoredLogin)));
        let process = Arc::new(FakeProcess::default());
        let status = controller(&probe, &process)
            .status(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(status.state, SubscriptionAuthState::SignedOut);
        assert!(status.needs_sign_in());
        assert!(process.requests().is_empty());
        assert!(process.resolve_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn batch_shim_only_is_reported_as_its_own_obstacle_not_a_bare_unknown() {
        let probe = FakeProbe::answering(installed(false, Some(CliObstacle::BatchShimOnly)));
        let process = Arc::new(FakeProcess::default());
        let status = controller(&probe, &process)
            .status(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(status.state, SubscriptionAuthState::Unknown);
        assert_eq!(status.obstacle, Some(CliObstacle::BatchShimOnly));
        assert!(status.installed());
        assert!(!status.needs_sign_in(), "the fix is the native install");
    }

    #[tokio::test]
    async fn missing_cli_is_a_normal_not_installed_status() {
        let probe = FakeProbe::answering(CliProbeSnapshot::default());
        let process = Arc::new(FakeProcess::default());
        let status = controller(&probe, &process)
            .status(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(status.state, SubscriptionAuthState::NotInstalled);
        assert!(status.path.is_none());
        assert!(!status.needs_sign_in());
    }

    #[tokio::test]
    async fn inconclusive_probe_is_unknown_not_signed_out() {
        let probe = FakeProbe::answering(installed(false, Some(CliObstacle::StatusUnclear)));
        let process = Arc::new(FakeProcess::default());
        let status = controller(&probe, &process)
            .status(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(status.state, SubscriptionAuthState::Unknown);
        assert!(status.installed());
        assert!(!status.signed_in());
        assert!(status.needs_sign_in(), "unclear is worth a sign-in attempt");
    }

    #[tokio::test]
    async fn sign_in_refuses_a_batch_shim_with_a_typed_error_before_spawning() {
        let probe = FakeProbe::answering(installed(false, Some(CliObstacle::BatchShimOnly)));
        let process = Arc::new(FakeProcess::scripted(
            [Ok(PathBuf::from(
                "C:/Users/mike/AppData/Roaming/npm/codex.cmd",
            ))],
            [Ok(output(true, "", ""))],
        ));
        let error = controller(&probe, &process)
            .sign_in(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            SubscriptionAuthError::BatchShimOnly {
                provider: SubscriptionAuthProvider::Codex
            }
        );
        assert!(error.to_string().contains("npm script shim"));
        assert!(process.requests().is_empty());
    }

    #[tokio::test]
    async fn sign_in_is_explicit_and_refreshes_status_after_success() {
        let probe = FakeProbe::answering(installed(true, None));
        let process = Arc::new(FakeProcess::scripted(
            [Ok(PathBuf::from("/opt/codex"))],
            [Ok(output(true, "browser complete", ""))],
        ));
        let controller = controller(&probe, &process);

        let status = controller
            .sign_in(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(status.state, SubscriptionAuthState::SignedIn);
        let requests = process.requests();
        assert_eq!(
            requests.len(),
            1,
            "sign-in only; the fresh status is the probe's"
        );
        assert_eq!(requests[0].args, ["login"]);
        assert_eq!(requests[0].action, ProcessAction::SignIn);
        assert_eq!(
            probe.invalidations.lock().unwrap().as_slice(),
            [SubscriptionCli::Codex],
            "the cached pre-sign-in answer is dropped before the fresh status"
        );
        assert_eq!(probe.calls().len(), 1);
    }

    #[tokio::test]
    async fn sign_out_uses_vendor_command_and_truthful_fresh_status() {
        let probe = FakeProbe::answering(installed(false, Some(CliObstacle::SignedOut)));
        let process = Arc::new(FakeProcess::scripted(
            [Ok(PathBuf::from("/opt/claude"))],
            [Ok(output(false, "already logged out", ""))],
        ));
        let controller = controller(&probe, &process);

        let status = controller
            .sign_out(
                SubscriptionAuthProvider::ClaudeCode,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(status.state, SubscriptionAuthState::SignedOut);
        let requests = process.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].args, ["auth", "logout"]);
    }

    #[tokio::test]
    async fn late_cancel_after_vendor_commit_does_not_relabel_success() {
        let probe = FakeProbe::answering(installed(true, None));
        let process = Arc::new(FakeProcess::scripted(
            [Ok(PathBuf::from("/opt/codex"))],
            [Ok(output(true, "browser complete", ""))],
        ));
        let cancellation = CancellationToken::new();
        process.cancel_after_next_run(cancellation.clone());
        let controller = controller(&probe, &process);

        let status = controller
            .sign_in(SubscriptionAuthProvider::Codex, None, cancellation.clone())
            .await
            .expect("the committed sign-in must be reported truthfully");

        assert!(cancellation.is_cancelled());
        assert_eq!(status.state, SubscriptionAuthState::SignedIn);
        assert_eq!(process.requests().len(), 1);
    }

    #[tokio::test]
    async fn cancelled_action_never_starts_a_process() {
        let process = Arc::new(FakeProcess::scripted(
            [Ok(PathBuf::from("/opt/codex"))],
            [Ok(output(true, "", ""))],
        ));
        let probe = FakeProbe::answering(CliProbeSnapshot::default());
        let controller = controller(&probe, &process);
        let token = CancellationToken::new();
        token.cancel();

        let error = controller
            .sign_in(SubscriptionAuthProvider::Codex, None, token)
            .await
            .unwrap_err();

        assert_eq!(
            error,
            SubscriptionAuthError::Cancelled {
                provider: SubscriptionAuthProvider::Codex,
                operation: SubscriptionAuthOperation::SignIn,
            }
        );
        assert!(process.requests().is_empty());
        assert!(process.resolve_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn relative_draft_path_is_rejected_before_resolution() {
        let process = Arc::new(FakeProcess::default());
        let probe = FakeProbe::answering(installed(true, None));
        let controller = controller(&probe, &process);

        let error = controller
            .status(
                SubscriptionAuthProvider::ClaudeCode,
                Some("bin/claude"),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            SubscriptionAuthError::InvalidCliPath {
                provider: SubscriptionAuthProvider::ClaudeCode
            }
        ));
        assert!(process.resolve_calls.lock().unwrap().is_empty());
        assert!(probe.calls().is_empty(), "rejected before the probe ran");
    }

    #[tokio::test]
    async fn sign_in_runs_interactively_and_reprobes_on_exit() {
        let probe = FakeProbe::answering(installed(true, None));
        let process = Arc::new(FakeProcess::scripted(
            [Ok(PathBuf::from("/opt/claude"))],
            [Ok(output(true, "", ""))],
        ));
        let status = controller(&probe, &process)
            .sign_in(
                SubscriptionAuthProvider::ClaudeCode,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(status.state, SubscriptionAuthState::SignedIn);
        let interactive = process.interactive_requests();
        assert_eq!(interactive.len(), 1, "sign-in is the console child");
        assert_eq!(interactive[0].args, ["auth", "login"]);
        assert_eq!(interactive[0].action, ProcessAction::SignIn);
        assert_eq!(interactive[0].action.timeout(), Duration::from_secs(600));
    }

    #[tokio::test]
    async fn sign_in_timeout_is_reported_as_a_typed_timeout() {
        let probe = FakeProbe::answering(installed(false, Some(CliObstacle::NoStoredLogin)));
        let process = Arc::new(FakeProcess::scripted([Ok(PathBuf::from("/opt/codex"))], []));
        process.fail_interactive(InteractiveFailure::TimedOut);
        let error = controller(&probe, &process)
            .sign_in(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            SubscriptionAuthError::SignInTimedOut {
                provider: SubscriptionAuthProvider::Codex
            }
        );
        assert!(error.to_string().contains("10 minutes"));
        assert!(probe.calls().is_empty(), "no status after a failed sign-in");
    }

    #[tokio::test]
    async fn sign_out_remains_headless() {
        let probe = FakeProbe::answering(installed(false, Some(CliObstacle::SignedOut)));
        let process = Arc::new(FakeProcess::scripted(
            [Ok(PathBuf::from("/opt/codex"))],
            [Ok(output(true, "logged out", ""))],
        ));
        controller(&probe, &process)
            .sign_out(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(process.interactive_requests().is_empty());
        assert_eq!(process.requests()[0].action, ProcessAction::SignOut);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_an_interactive_sign_in_kills_the_child() {
        let request = ProcessRequest {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c", "sleep 30"],
            action: ProcessAction::SignIn,
        };
        let cancellation = CancellationToken::new();
        let canceller = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            canceller.cancel();
        });
        let started = std::time::Instant::now();
        let result = run_console_process(request, cancellation).await;
        assert_eq!(result, Err(InteractiveFailure::Cancelled));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the cancel must not wait for the 30 s child"
        );
    }

    #[tokio::test]
    async fn public_errors_never_include_raw_child_output() {
        let secret = "OPENAI_API_KEY=sk-do-not-render";
        let process = Arc::new(FakeProcess::scripted(
            [Ok(PathBuf::from("/opt/codex"))],
            [Ok(output(false, "", secret))],
        ));
        let probe = FakeProbe::answering(installed(false, Some(CliObstacle::SignedOut)));
        let controller = controller(&probe, &process);

        let error = controller
            .sign_in(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            SubscriptionAuthError::SignInFailed {
                provider: SubscriptionAuthProvider::Codex
            }
        ));
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }

    #[test]
    fn shared_process_runner_scrubs_every_subscription_key() {
        for key in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CODEX_API_KEY",
            "OPENAI_API_KEY",
        ] {
            assert!(cli_bridge::SUBSCRIPTION_OVERRIDE_ENV_VARS.contains(&key));
        }
    }
}
