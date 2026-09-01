//! Explicit, UI-framework-neutral subscription CLI account actions.
//!
//! The genuine Codex and Claude Code CLIs remain the sole owners of their
//! credentials. This adapter never reads credential files or returns child
//! output. Construction has no side effects, status performs only executable
//! resolution plus the vendor's status command, and account mutations happen
//! only through explicit `sign_in` / `sign_out` calls.

use crate::cli_bridge;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wfdiag_native_ai_provider::SubscriptionCli;

const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);
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
    const fn spec(self) -> &'static AuthSpec {
        match self {
            Self::Codex => &CODEX_SPEC,
            Self::ClaudeCode => &CLAUDE_SPEC,
        }
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
}

impl SubscriptionAuthStatus {
    #[must_use]
    pub const fn installed(&self) -> bool {
        self.path.is_some()
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

struct AuthSpec {
    binary: &'static str,
    sign_in_args: &'static [&'static str],
    sign_out_args: &'static [&'static str],
    status_args: &'static [&'static str],
    signed_out_markers: &'static [&'static str],
}

const CODEX_SPEC: AuthSpec = AuthSpec {
    binary: "codex",
    sign_in_args: &["login"],
    sign_out_args: &["logout"],
    status_args: &["login", "status"],
    signed_out_markers: &["not logged in"],
};

const CLAUDE_SPEC: AuthSpec = AuthSpec {
    binary: "claude",
    sign_in_args: &["auth", "login"],
    sign_out_args: &["auth", "logout"],
    status_args: &["auth", "status"],
    signed_out_markers: &["not logged in", "please run /login"],
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessAction {
    Status,
    SignIn,
    SignOut,
}

impl ProcessAction {
    const fn timeout(self) -> Duration {
        match self {
            Self::Status => STATUS_TIMEOUT,
            Self::SignIn => SIGN_IN_TIMEOUT,
            Self::SignOut => SIGN_OUT_TIMEOUT,
        }
    }

    const fn safe_label(self) -> &'static str {
        match self {
            Self::Status => "subscription status",
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

trait AuthProcess: Send + Sync + 'static {
    fn resolve(
        &self,
        binary: &'static str,
        draft_path: Option<String>,
    ) -> AuthFuture<'_, Result<PathBuf, ResolveFailure>>;

    fn run(&self, request: ProcessRequest) -> AuthFuture<'_, Result<ProcessOutput, ()>>;
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
}

/// Concrete account controller shared by desktop shells.
///
/// `new` is side-effect-free. There is intentionally no installation method:
/// the UI may explain how to install a vendor CLI, but this boundary cannot do
/// so implicitly (or explicitly). API-key override variables are scrubbed,
/// child processes are hidden on Windows, time bounded, and killed when a
/// cancelled future is dropped by [`cli_bridge::run_headless`].
#[derive(Clone)]
pub struct SubscriptionAuthController {
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
            process: Arc::new(TokioAuthProcess),
            active_mutations: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    #[cfg(test)]
    fn with_process(process: Arc<dyn AuthProcess>) -> Self {
        Self {
            process,
            active_mutations: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Resolve the selected CLI and run only its documented status command.
    ///
    /// A non-blank draft path always wins and must be absolute. A missing CLI
    /// is a normal `NotInstalled` state; a present CLI whose status cannot be
    /// established is `Unknown`, never falsely reported as signed out.
    pub async fn status(
        &self,
        provider: SubscriptionAuthProvider,
        draft_cli_path: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<SubscriptionAuthStatus, SubscriptionAuthError> {
        let draft = normalized_absolute_draft(provider, draft_cli_path)?;
        let spec = provider.spec();
        let resolved = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                return Err(SubscriptionAuthError::Cancelled {
                    provider,
                    operation: SubscriptionAuthOperation::Status,
                });
            }
            result = self.process.resolve(spec.binary, draft.clone()) => result,
        };
        let path = match resolved {
            Ok(path) => path,
            Err(ResolveFailure::InvalidDraft) => {
                return Err(SubscriptionAuthError::InvalidCliPath { provider });
            }
            Err(ResolveFailure::NotFound) => {
                return Ok(SubscriptionAuthStatus {
                    provider,
                    state: SubscriptionAuthState::NotInstalled,
                    path: None,
                });
            }
            Err(ResolveFailure::Unknown) => {
                return Ok(SubscriptionAuthStatus {
                    provider,
                    state: SubscriptionAuthState::Unknown,
                    path: None,
                });
            }
        };

        let request = ProcessRequest {
            program: path.clone(),
            args: spec.status_args.to_vec(),
            action: ProcessAction::Status,
        };
        let output = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                return Err(SubscriptionAuthError::Cancelled {
                    provider,
                    operation: SubscriptionAuthOperation::Status,
                });
            }
            result = self.process.run(request) => result,
        };
        let state = output.map_or(SubscriptionAuthState::Unknown, |output| {
            parse_status(spec, &output)
        });
        Ok(SubscriptionAuthStatus {
            provider,
            state,
            path: Some(path),
        })
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
            ProcessAction::Status => SubscriptionAuthOperation::Status,
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
        let args = match action {
            ProcessAction::Status => spec.status_args,
            ProcessAction::SignIn => spec.sign_in_args,
            ProcessAction::SignOut => spec.sign_out_args,
        };
        let output = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                return Err(SubscriptionAuthError::Cancelled { provider, operation });
            }
            result = self.process.run(ProcessRequest {
                program: path,
                args: args.to_vec(),
                action,
            }) => result,
        }
        .map_err(|()| SubscriptionAuthError::OperationUnavailable {
            provider,
            operation,
        })?;

        if action == ProcessAction::SignIn && !output.success {
            return Err(SubscriptionAuthError::SignInFailed { provider });
        }
        // The vendor child completing is the mutation's commit point. A late
        // UI cancellation must not report the already-committed credential
        // change as cancelled. The truthful follow-up probe remains bounded by
        // STATUS_TIMEOUT, but intentionally gets a fresh token.
        // Logout on an already signed-out CLI may exit non-zero, so this fresh
        // status remains the source of truth.
        self.status(provider, draft.as_deref(), CancellationToken::new())
            .await
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

fn parse_status(spec: &AuthSpec, output: &ProcessOutput) -> SubscriptionAuthState {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let signed_out = spec.signed_out_markers.iter().any(|marker| {
        stdout.to_ascii_lowercase().contains(marker) || stderr.to_ascii_lowercase().contains(marker)
    });
    if signed_out {
        SubscriptionAuthState::SignedOut
    } else if output.success {
        SubscriptionAuthState::SignedIn
    } else {
        SubscriptionAuthState::Unknown
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
    }

    fn output(success: bool, stdout: &str, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            success,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn status_parser_is_conservative_and_case_insensitive() {
        assert_eq!(
            parse_status(&CLAUDE_SPEC, &output(true, "Logged in", "")),
            SubscriptionAuthState::SignedIn
        );
        assert_eq!(
            parse_status(
                &CLAUDE_SPEC,
                &output(true, "NOT LOGGED IN", "Please run /LOGIN")
            ),
            SubscriptionAuthState::SignedOut
        );
        assert_eq!(
            parse_status(&CODEX_SPEC, &output(false, "", "unexpected failure")),
            SubscriptionAuthState::Unknown
        );
    }

    #[tokio::test]
    async fn status_is_read_only_and_uses_the_shipping_vendor_commands() {
        for (provider, expected_args) in [
            (SubscriptionAuthProvider::Codex, vec!["login", "status"]),
            (SubscriptionAuthProvider::ClaudeCode, vec!["auth", "status"]),
        ] {
            let process = Arc::new(FakeProcess::scripted(
                [Ok(PathBuf::from("/opt/vendor"))],
                [Ok(output(true, "signed in", ""))],
            ));
            let controller = SubscriptionAuthController::with_process(process.clone());

            let status = controller
                .status(provider, None, CancellationToken::new())
                .await
                .unwrap();

            assert_eq!(status.state, SubscriptionAuthState::SignedIn);
            let requests = process.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].args, expected_args);
            assert_eq!(requests[0].action, ProcessAction::Status);
        }
    }

    #[tokio::test]
    async fn missing_cli_is_a_normal_not_installed_status() {
        let process = Arc::new(FakeProcess::scripted([], []));
        process
            .resolutions
            .lock()
            .unwrap()
            .push_back(Err(ResolveFailure::NotFound));
        let controller = SubscriptionAuthController::with_process(process.clone());

        let status = controller
            .status(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(status.state, SubscriptionAuthState::NotInstalled);
        assert!(status.path.is_none());
        assert!(process.requests().is_empty());
    }

    #[tokio::test]
    async fn inconclusive_lookup_is_unknown_not_signed_out() {
        let process = Arc::new(FakeProcess::scripted([Err(ResolveFailure::Unknown)], []));
        let controller = SubscriptionAuthController::with_process(process);

        let status = controller
            .status(
                SubscriptionAuthProvider::Codex,
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(status.state, SubscriptionAuthState::Unknown);
        assert!(!status.installed());
        assert!(!status.signed_in());
    }

    #[tokio::test]
    async fn sign_in_is_explicit_and_refreshes_status_after_success() {
        let process = Arc::new(FakeProcess::scripted(
            [
                Ok(PathBuf::from("/opt/codex")),
                Ok(PathBuf::from("/opt/codex")),
            ],
            [
                Ok(output(true, "browser complete", "")),
                Ok(output(true, "Logged in using ChatGPT", "")),
            ],
        ));
        let controller = SubscriptionAuthController::with_process(process.clone());

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
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].args, ["login"]);
        assert_eq!(requests[0].action, ProcessAction::SignIn);
        assert_eq!(requests[1].args, ["login", "status"]);
        assert_eq!(requests[1].action, ProcessAction::Status);
    }

    #[tokio::test]
    async fn sign_out_uses_vendor_command_and_truthful_fresh_status() {
        let process = Arc::new(FakeProcess::scripted(
            [
                Ok(PathBuf::from("/opt/claude")),
                Ok(PathBuf::from("/opt/claude")),
            ],
            [
                Ok(output(false, "already logged out", "")),
                Ok(output(false, "", "Not logged in; please run /login")),
            ],
        ));
        let controller = SubscriptionAuthController::with_process(process.clone());

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
        assert_eq!(requests[0].args, ["auth", "logout"]);
        assert_eq!(requests[1].args, ["auth", "status"]);
    }

    #[tokio::test]
    async fn late_cancel_after_vendor_commit_does_not_relabel_success() {
        let process = Arc::new(FakeProcess::scripted(
            [
                Ok(PathBuf::from("/opt/codex")),
                Ok(PathBuf::from("/opt/codex")),
            ],
            [
                Ok(output(true, "browser complete", "")),
                Ok(output(true, "Logged in using ChatGPT", "")),
            ],
        ));
        let cancellation = CancellationToken::new();
        process.cancel_after_next_run(cancellation.clone());
        let controller = SubscriptionAuthController::with_process(process.clone());

        let status = controller
            .sign_in(SubscriptionAuthProvider::Codex, None, cancellation.clone())
            .await
            .expect("the committed sign-in must be reported truthfully");

        assert!(cancellation.is_cancelled());
        assert_eq!(status.state, SubscriptionAuthState::SignedIn);
        assert_eq!(process.requests().len(), 2);
    }

    #[tokio::test]
    async fn cancelled_action_never_starts_a_process() {
        let process = Arc::new(FakeProcess::scripted(
            [Ok(PathBuf::from("/opt/codex"))],
            [Ok(output(true, "", ""))],
        ));
        let controller = SubscriptionAuthController::with_process(process.clone());
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
        let controller = SubscriptionAuthController::with_process(process.clone());

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
    }

    #[tokio::test]
    async fn public_errors_never_include_raw_child_output() {
        let secret = "OPENAI_API_KEY=sk-do-not-render";
        let process = Arc::new(FakeProcess::scripted(
            [Ok(PathBuf::from("/opt/codex"))],
            [Ok(output(false, "", secret))],
        ));
        let controller = SubscriptionAuthController::with_process(process);

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
