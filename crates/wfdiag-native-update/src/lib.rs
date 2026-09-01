//! UI-framework-neutral update checks for the GitHub-distributed channel.
//!
//! The Microsoft Store services Store installs, so those installs never make
//! a GitHub request. All transport, status, JSON, and version failures remain
//! deliberately silent: an update check may return an available release or no
//! release, but it never creates a user-facing error surface.

pub mod policy;

use semver::Version;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// GitHub endpoint used by the direct-distribution update channel.
pub const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/faratech/wfdiag/releases/latest";
/// The complete request timeout retained from the shipping update checker.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// GitHub's versioned JSON media type retained from the shipping request.
pub const GITHUB_JSON_ACCEPT: &str = "application/vnd.github+json";

/// A newer public release that the UI may present to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub html_url: String,
    pub published_at: Option<String>,
    pub notes_excerpt: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// One fully specified HTTP request. Keeping this type public makes transport
/// behavior contract-testable without opening a socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseRequest {
    pub url: &'static str,
    pub user_agent: String,
    pub accept: &'static str,
    pub timeout: Duration,
}

/// Response subset needed by the update service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl ReleaseResponse {
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

/// Injectable release transport. Implementations may block because the
/// service always runs them on [`NativeUpdateRuntime`]'s dedicated worker.
pub trait ReleaseHttp: Send + Sync {
    /// Fetch the latest-release endpoint.
    ///
    /// # Errors
    ///
    /// Returns a transport-specific diagnostic. The service deliberately
    /// converts it to a silent no-update result.
    fn fetch_latest(&self, request: &ReleaseRequest) -> Result<ReleaseResponse, String>;
}

/// Production GitHub transport. It owns no persistent files or caches.
#[derive(Debug, Default)]
pub struct ReqwestReleaseHttp;

impl ReleaseHttp for ReqwestReleaseHttp {
    fn fetch_latest(&self, request: &ReleaseRequest) -> Result<ReleaseResponse, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(request.timeout)
            .build()
            .map_err(|error| error.to_string())?;
        let response = client
            .get(request.url)
            .header("User-Agent", &request.user_agent)
            .header("Accept", request.accept)
            .send()
            .map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            // The shipping checker rejects non-success status immediately,
            // without waiting for or parsing an error body.
            return Ok(ReleaseResponse {
                status,
                body: Vec::new(),
            });
        }
        let body = response
            .bytes()
            .map_err(|error| error.to_string())?
            .to_vec();
        Ok(ReleaseResponse { status, body })
    }
}

/// Signature classification relevant to update-channel selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageSignature {
    Store,
    Other,
}

/// Injectable package identity/signature reader.
///
/// Signature errors for an identified package intentionally count as Store.
/// This preserves the shipping fail-closed policy: Store users must never be
/// nagged about a GitHub update merely because a Windows package API failed.
pub trait SignatureProvider: Send + Sync {
    fn has_package_identity(&self) -> bool;

    /// Read the package's signature classification.
    ///
    /// # Errors
    ///
    /// Returns a platform diagnostic when signature metadata is unavailable.
    /// The policy deliberately treats that failure as a Store install.
    fn signature(&self) -> Result<PackageSignature, String>;
}

/// Shipping Windows package identity and signature provider.
///
/// The provider is intentionally independent of any UI shell. Both Tauri and
/// Reactor use the same `GetCurrentPackageFullName` identity probe and `WinRT`
/// `Package.SignatureKind` classification.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsPackageSignatureProvider;

impl WindowsPackageSignatureProvider {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl SignatureProvider for WindowsPackageSignatureProvider {
    fn has_package_identity(&self) -> bool {
        windows_has_package_identity()
    }

    fn signature(&self) -> Result<PackageSignature, String> {
        #[cfg(windows)]
        {
            windows_package_signature()
        }
        #[cfg(not(windows))]
        {
            Ok(PackageSignature::Other)
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn windows_has_package_identity() -> bool {
    use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;

    let mut length = 0;
    // A null buffer intentionally asks only for the package-name length.
    // ERROR_INSUFFICIENT_BUFFER means identity exists; error 15700 means the
    // process is unpackaged. This is the existing shipping identity probe.
    let result = unsafe { GetCurrentPackageFullName(&raw mut length, None) };
    result == ERROR_INSUFFICIENT_BUFFER
}

#[cfg(not(windows))]
const fn windows_has_package_identity() -> bool {
    false
}

#[cfg(windows)]
fn windows_package_signature() -> Result<PackageSignature, String> {
    use windows::ApplicationModel::{Package, PackageSignatureKind};

    Package::Current()
        .and_then(|package| package.SignatureKind())
        .map(|kind| {
            if kind == PackageSignatureKind::Store {
                PackageSignature::Store
            } else {
                PackageSignature::Other
            }
        })
        .map_err(|error| error.to_string())
}

/// Injectable running-version provider shared by Tauri and Reactor shells.
pub trait CurrentVersionProvider: Send + Sync {
    fn current_version(&self) -> Version;
}

/// Immutable provider useful for application package metadata.
#[derive(Debug, Clone)]
pub struct StaticCurrentVersion(Version);

impl StaticCurrentVersion {
    #[must_use]
    pub const fn new(version: Version) -> Self {
        Self(version)
    }

    /// Parse an application-owned version string without reading UI framework
    /// or package metadata.
    ///
    /// # Errors
    ///
    /// Returns [`semver::Error`] when `version` is not valid semantic version
    /// syntax.
    pub fn parse(version: &str) -> Result<Self, semver::Error> {
        Version::parse(version).map(Self)
    }
}

impl CurrentVersionProvider for StaticCurrentVersion {
    fn current_version(&self) -> Version {
        self.0.clone()
    }
}

/// True only for a Store-signed identified package, except that package API
/// failure is treated as Store by design.
#[must_use]
pub fn is_store_install(provider: &dyn SignatureProvider) -> bool {
    if !provider.has_package_identity() {
        return false;
    }
    match provider.signature() {
        Ok(kind) => kind == PackageSignature::Store,
        Err(_) => true,
    }
}

/// Pure release policy: drafts, prereleases, malformed tags, and versions not
/// newer than the running application remain silent.
fn evaluate_release(release: GithubRelease, current: &Version) -> Option<UpdateInfo> {
    if release.draft || release.prerelease {
        return None;
    }
    let remote = Version::parse(release.tag_name.trim().trim_start_matches('v')).ok()?;
    if remote <= *current {
        return None;
    }
    Some(UpdateInfo {
        version: remote.to_string(),
        html_url: release.html_url,
        published_at: release.published_at,
        notes_excerpt: release
            .body
            .map(|body| body.chars().take(300).collect::<String>()),
    })
}

/// Why a check could not complete.
///
/// The passive update path still presents nothing to the user, but the
/// distinction is preserved so callers and logs can tell "GitHub was
/// unreachable" apart from "you are on the latest release".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateFailure {
    /// The transport itself failed (offline, DNS, TLS, timeout).
    Transport(String),
    /// GitHub answered with a non-success status, `403` rate limiting
    /// included. No error body is read.
    Status(u16),
    /// The response was not the release JSON this checker understands.
    Parse(String),
}

impl fmt::Display for UpdateFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(reason) => write!(formatter, "update request failed: {reason}"),
            Self::Status(status) => {
                write!(formatter, "update request returned HTTP status {status}")
            }
            Self::Parse(reason) => {
                write!(formatter, "update response could not be parsed: {reason}")
            }
        }
    }
}

impl std::error::Error for UpdateFailure {}

/// The complete result of one update check.
///
/// Collapsing every one of these to "no update" is what made a rate-limited or
/// offline check indistinguishable from a current install (#223). Shells still
/// choose to display nothing for anything but [`Self::Available`]; they can now
/// make that choice knowingly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// This channel does not check at all: a Store install or a debug build.
    /// No request was made and none should be reported.
    Silent,
    /// The check completed and the running build is current.
    UpToDate,
    /// A newer public release is available.
    Available(UpdateInfo),
    /// The check could not complete.
    Failed(UpdateFailure),
}

impl UpdateOutcome {
    /// The newer release, if this check found one.
    #[must_use]
    pub const fn available(&self) -> Option<&UpdateInfo> {
        match self {
            Self::Available(update) => Some(update),
            _ => None,
        }
    }

    /// Consume the outcome, keeping only a newer release.
    #[must_use]
    pub fn into_available(self) -> Option<UpdateInfo> {
        match self {
            Self::Available(update) => Some(update),
            _ => None,
        }
    }

    /// Did the check fail to complete, as opposed to completing with nothing
    /// to offer?
    #[must_use]
    pub const fn failure(&self) -> Option<&UpdateFailure> {
        match self {
            Self::Failed(failure) => Some(failure),
            _ => None,
        }
    }
}

/// Complete update-check policy with injectable environmental boundaries.
#[derive(Clone)]
pub struct UpdateService {
    http: Arc<dyn ReleaseHttp>,
    signature: Arc<dyn SignatureProvider>,
    current_version: Arc<dyn CurrentVersionProvider>,
    debug_build: bool,
}

impl fmt::Debug for UpdateService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateService")
            .field("debug_build", &self.debug_build)
            .finish_non_exhaustive()
    }
}

impl UpdateService {
    #[must_use]
    pub fn new(
        http: Arc<dyn ReleaseHttp>,
        signature: Arc<dyn SignatureProvider>,
        current_version: Arc<dyn CurrentVersionProvider>,
        debug_build: bool,
    ) -> Self {
        Self {
            http,
            signature,
            current_version,
            debug_build,
        }
    }

    /// Construct the complete shipping service without a Tauri dependency.
    #[must_use]
    pub fn shipping(current_version: Version, debug_build: bool) -> Self {
        Self::new(
            Arc::new(ReqwestReleaseHttp),
            Arc::new(WindowsPackageSignatureProvider::new()),
            Arc::new(StaticCurrentVersion::new(current_version)),
            debug_build,
        )
    }

    /// Parse a version string and construct the complete shipping service.
    /// This is the convenient Reactor path for its build-generated version
    /// constant.
    ///
    /// # Errors
    ///
    /// Returns [`semver::Error`] when `current_version` is invalid.
    pub fn shipping_from_str(
        current_version: &str,
        debug_build: bool,
    ) -> Result<Self, semver::Error> {
        Ok(Self::shipping(
            Version::parse(current_version)?,
            debug_build,
        ))
    }

    /// Check once, classifying the result.
    ///
    /// Deliberately not public: this blocks for up to [`REQUEST_TIMEOUT`] and
    /// performs Windows package calls, so every caller goes through
    /// [`NativeUpdateRuntime::request_check`], which owns the dedicated worker
    /// thread. Making it callable directly is what let a blocking check reach
    /// an async executor (#211).
    #[must_use]
    pub(crate) fn check_outcome(&self) -> UpdateOutcome {
        if self.debug_build || is_store_install(self.signature.as_ref()) {
            return UpdateOutcome::Silent;
        }
        let current = self.current_version.current_version();
        let request = ReleaseRequest {
            url: RELEASES_LATEST_URL,
            user_agent: format!("wfdiag/{current}"),
            accept: GITHUB_JSON_ACCEPT,
            timeout: REQUEST_TIMEOUT,
        };
        let response = match self.http.fetch_latest(&request) {
            Ok(response) => response,
            Err(reason) => return UpdateOutcome::Failed(UpdateFailure::Transport(reason)),
        };
        if !response.is_success() {
            return UpdateOutcome::Failed(UpdateFailure::Status(response.status));
        }
        match serde_json::from_slice::<GithubRelease>(&response.body) {
            Ok(release) => evaluate_release(release, &current)
                .map_or(UpdateOutcome::UpToDate, UpdateOutcome::Available),
            Err(error) => UpdateOutcome::Failed(UpdateFailure::Parse(error.to_string())),
        }
    }
}

enum UpdateCommand {
    Check {
        reply: oneshot::Sender<UpdateOutcome>,
    },
    Shutdown,
}

/// Reply handle returned immediately to a native UI thread.
pub type UpdateReply = oneshot::Receiver<UpdateOutcome>;

/// Queue/runtime errors are separate from deliberately silent check results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateRuntimeError {
    SpawnFailed,
    WorkerStopped,
}

impl fmt::Display for UpdateRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpawnFailed => formatter.write_str("failed to start native update worker"),
            Self::WorkerStopped => formatter.write_str("native update worker stopped"),
        }
    }
}

impl std::error::Error for UpdateRuntimeError {}

fn reap_worker(worker: JoinHandle<()>) {
    let _ = std::thread::Builder::new()
        .name("wfdiag-update-reaper".to_string())
        .spawn(move || {
            let _ = worker.join();
        });
}

/// Dedicated update worker for native UI shells.
///
/// [`Self::request_check`] only enqueues a command and returns a typed oneshot;
/// neither Windows package calls nor network I/O runs on the `WinUI` thread.
pub struct NativeUpdateRuntime {
    commands: mpsc::UnboundedSender<UpdateCommand>,
    worker: Option<JoinHandle<()>>,
}

impl NativeUpdateRuntime {
    /// Start the background worker.
    ///
    /// # Errors
    ///
    /// Returns [`UpdateRuntimeError::SpawnFailed`] if the operating system
    /// cannot create the worker thread.
    pub fn start(service: UpdateService) -> Result<Self, UpdateRuntimeError> {
        let (commands, mut receiver) = mpsc::unbounded_channel();
        let worker = std::thread::Builder::new()
            .name("wfdiag-native-update".to_string())
            .spawn(move || {
                while let Some(command) = receiver.blocking_recv() {
                    match command {
                        UpdateCommand::Check { reply } => {
                            let _ = reply.send(service.check_outcome());
                        }
                        UpdateCommand::Shutdown => break,
                    }
                }
            })
            .map_err(|_| UpdateRuntimeError::SpawnFailed)?;
        Ok(Self {
            commands,
            worker: Some(worker),
        })
    }

    /// Queue one check without blocking the caller.
    ///
    /// # Errors
    ///
    /// Returns [`UpdateRuntimeError::WorkerStopped`] after worker shutdown.
    pub fn request_check(&self) -> Result<UpdateReply, UpdateRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.commands
            .send(UpdateCommand::Check { reply })
            .map_err(|_| UpdateRuntimeError::WorkerStopped)?;
        Ok(receiver)
    }
}

impl Drop for NativeUpdateRuntime {
    fn drop(&mut self) {
        let _ = self.commands.send(UpdateCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            reap_worker(worker);
        }
    }
}

#[cfg(test)]
mod tests;
