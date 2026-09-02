//! Subscription-CLI account and installation state machines.
//!
//! Two invariants live here and nowhere else:
//!
//! * **One operation at a time, across both machines.** An install must not
//!   start while a sign-in is running against the same CLI, and neither may
//!   overlap itself.
//! * **Installation needs two separate confirmations.** The winget path needs
//!   one; falling back to the vendor's PowerShell bootstrap needs a second,
//!   because it downloads and executes a script the vendor can change.
//!
//! Nothing here spawns a process: it decides, and the service acts.

use wfdiag_native_ai_chat::{
    CliObstacle, SubscriptionAuthOperation, SubscriptionAuthProvider, SubscriptionAuthStatus,
    SubscriptionInstallFallbackReason, SubscriptionInstallMethod, SubscriptionInstallProgress,
    auth_status_from_probe,
};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, AIProviderStatus, AUTO_FALLBACK_ORDER, CliProbeSnapshot,
    ProviderAvailability, ProviderInfo, route_provider, subscription_cli_spec,
};

/// One subscription CLI's account read model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountState {
    /// The last known account status.
    pub status: Option<SubscriptionAuthStatus>,
    /// The operation currently running, if any.
    pub operation: Option<SubscriptionAuthOperation>,
    /// The last failure.
    pub error: Option<String>,
}

/// Whether an account operation may start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthAdmission {
    /// Start it.
    Start,
    /// Refuse; the reason is user-facing.
    Refuse {
        /// Why.
        reason: String,
    },
}

/// Decide whether `operation` may start.
///
/// `auth_busy` and `install_busy` are the two halves of the interlock: the
/// genuine CLIs keep account state in one place, so a status probe racing an
/// installer would read a half-written answer.
#[must_use]
pub fn admit_auth(
    auth_busy: bool,
    install_busy: bool,
    runtime_error: Option<&str>,
) -> AuthAdmission {
    if auth_busy || install_busy {
        return AuthAdmission::Refuse {
            reason: "A subscription account action is already active…".to_string(),
        };
    }
    if let Some(error) = runtime_error {
        return AuthAdmission::Refuse {
            reason: error.to_string(),
        };
    }
    AuthAdmission::Start
}

/// Whether a completed account operation should also refresh the model list.
///
/// A plain status probe must not, or the probe the debounced refresh itself
/// issues would loop forever.
#[must_use]
pub fn completion_refreshes_models(operation: SubscriptionAuthOperation) -> bool {
    operation != SubscriptionAuthOperation::Status
}

/// A confirmation the user has not answered yet.
///
/// The prompt carries everything the eventual dispatch needs, so answering it
/// cannot pick up state that moved on while the dialog was open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallPrompt {
    /// The first confirmation: install through Windows Package Manager.
    Winget {
        /// Which CLI.
        provider: SubscriptionAuthProvider,
    },
    /// The second confirmation: run the vendor's PowerShell bootstrap.
    VendorFallback {
        /// Which CLI.
        provider: SubscriptionAuthProvider,
        /// Why winget could not finish.
        reason: SubscriptionInstallFallbackReason,
    },
}

impl InstallPrompt {
    /// Which CLI this prompt is about.
    #[must_use]
    pub const fn provider(self) -> SubscriptionAuthProvider {
        match self {
            Self::Winget { provider } | Self::VendorFallback { provider, .. } => provider,
        }
    }

    /// The method an acceptance would run.
    #[must_use]
    pub const fn method(self) -> SubscriptionInstallMethod {
        match self {
            Self::Winget { .. } => SubscriptionInstallMethod::Winget,
            Self::VendorFallback { .. } => SubscriptionInstallMethod::VendorPowerShell,
        }
    }
}

/// Whether an installation may be offered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallAdmission {
    /// Show `prompt`; nothing runs until it is accepted.
    Confirm {
        /// The confirmation to show.
        prompt: InstallPrompt,
    },
    /// Refuse; the reason is user-facing.
    Refuse {
        /// Why.
        reason: String,
    },
}

/// Decide whether an install may be offered for `provider`.
#[must_use]
pub fn admit_install(
    provider: SubscriptionAuthProvider,
    install_busy: bool,
    auth_busy: bool,
    prompt_open: bool,
    runtime_error: Option<&str>,
) -> InstallAdmission {
    if install_busy || auth_busy || prompt_open {
        return InstallAdmission::Refuse {
            reason: "A subscription CLI action is already active…".to_string(),
        };
    }
    if let Some(error) = runtime_error {
        return InstallAdmission::Refuse {
            reason: error.to_string(),
        };
    }
    InstallAdmission::Confirm {
        prompt: InstallPrompt::Winget { provider },
    }
}

/// A user-facing label for one installer stage.
#[must_use]
pub const fn progress_label(progress: &SubscriptionInstallProgress) -> &'static str {
    use wfdiag_native_ai_chat::SubscriptionInstallStage as Stage;
    match progress.stage {
        Stage::CheckingExisting => "Checking for an existing CLI installation…",
        Stage::ResolvingInstaller => "Resolving the approved installer…",
        Stage::InstallingWinget => "Installing with Windows Package Manager…",
        Stage::InstallingVendorFallback => "Running the separately approved vendor installer…",
        Stage::Verifying => "Verifying the installed CLI path…",
        Stage::Completed => "CLI installation verified",
    }
}

/// The subscription CLI behind a provider row, if any.
#[must_use]
pub const fn subscription_provider(id: AIProvider) -> Option<SubscriptionAuthProvider> {
    match id {
        AIProvider::CodexCli => Some(SubscriptionAuthProvider::Codex),
        AIProvider::ClaudeCode => Some(SubscriptionAuthProvider::ClaudeCode),
        _ => None,
    }
}

/// The account state a provider status row already tells us: for the
/// subscription CLIs `configured` means installed, `available` means signed
/// in, and the row carries the obstacle and the path.
#[must_use]
pub fn account_from_row(row: &ProviderInfo) -> Option<SubscriptionAuthStatus> {
    let provider = subscription_provider(row.id)?;
    Some(auth_status_from_probe(
        provider,
        &CliProbeSnapshot {
            usable: row.available,
            installed: row.configured,
            path: row.endpoint.clone(),
            obstacle: row.obstacle,
        },
    ))
}

/// Why a sign-in is what stands between the user and a working assistant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInRequiredReason {
    /// The user chose this CLI explicitly.
    ExplicitPreference,
    /// Auto routing found nothing usable and this installed CLI is the first
    /// candidate that would work once signed in.
    OnlyCandidate,
}

/// An installed-but-unusable subscription CLI that is the reason no provider
/// is ready.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignInRequirement {
    /// Which CLI.
    pub provider: SubscriptionAuthProvider,
    /// Why it is unusable.
    pub obstacle: CliObstacle,
    /// Why it matters right now.
    pub reason: SignInRequiredReason,
}

impl SignInRequirement {
    /// "Sign in to `ChatGPT` before sending: Codex CLI has no stored login."
    #[must_use]
    pub fn refusal(&self, verb: &str) -> String {
        let spec = subscription_cli_spec(self.provider.into());
        let summary = self.obstacle.summary();
        if self.obstacle == CliObstacle::BatchShimOnly {
            format!(
                "Install the native {} before {verb}: {} {summary}.",
                spec.label, spec.label
            )
        } else {
            format!(
                "Sign in to {} before {verb}: {} {summary}.",
                spec.account_label, spec.label
            )
        }
    }
}

/// Whether the reason nothing routes is an installed subscription CLI that
/// is not signed in. `None` whenever some provider routes, or when nothing
/// installed would help.
#[must_use]
pub fn sign_in_requirement(
    preference: AIProviderPreference,
    status: &AIProviderStatus,
) -> Option<SignInRequirement> {
    if route_provider(preference, status.availability()) != AIProvider::None {
        return None;
    }
    let candidate = |id: AIProvider, reason: SignInRequiredReason| {
        let row = status.providers.iter().find(|row| row.id == id)?;
        if !row.configured || row.available {
            return None;
        }
        Some(SignInRequirement {
            provider: subscription_provider(id)?,
            obstacle: row.obstacle.unwrap_or(CliObstacle::StatusUnclear),
            reason,
        })
    };
    if preference == AIProviderPreference::Auto {
        return AUTO_FALLBACK_ORDER
            .iter()
            .find_map(|id| candidate(*id, SignInRequiredReason::OnlyCandidate));
    }
    // The explicit provider is whatever the preference routes to when
    // everything is available.
    let everything = ProviderAvailability {
        phi: true,
        foundry: true,
        ollama: true,
        custom: true,
        codex: true,
        claude: true,
        openai: true,
        anthropic: true,
        gemini: true,
        deepseek: true,
    };
    candidate(
        route_provider(preference, everything),
        SignInRequiredReason::ExplicitPreference,
    )
}

/// A projection from provider status must not overwrite an account whose
/// own operation is in flight; that operation reports the truth when done.
#[must_use]
pub const fn account_projection_allowed(account: &AccountState) -> bool {
    account.operation.is_none()
}

/// Whether a reported install path can be trusted.
///
/// The installer must return an absolute path it actually observed; a relative
/// one would be resolved against whatever directory the app happens to have.
#[must_use]
pub fn verified_install_path(path: &std::path::Path) -> bool {
    path.is_absolute()
}

#[cfg(test)]
mod tests {
    use super::{
        AccountState, AuthAdmission, InstallAdmission, InstallPrompt, SignInRequiredReason,
        account_from_row, account_projection_allowed, admit_auth, admit_install,
        completion_refreshes_models, progress_label, sign_in_requirement, verified_install_path,
    };
    use wfdiag_native_ai_chat::{CliObstacle, SubscriptionAuthState};
    use wfdiag_native_ai_provider::{
        AIProvider, AIProviderPreference, AIProviderStatus, ProviderInfo,
    };

    fn row(
        id: AIProvider,
        available: bool,
        configured: bool,
        obstacle: Option<CliObstacle>,
    ) -> ProviderInfo {
        ProviderInfo {
            id,
            available,
            configured,
            model: None,
            endpoint: configured.then(|| "C:/tools/cli.exe".to_string()),
            obstacle,
            supports_tools: false,
            supports_streaming: false,
        }
    }

    fn status(rows: Vec<ProviderInfo>) -> AIProviderStatus {
        let openai = rows
            .iter()
            .any(|row| row.id == AIProvider::OpenAI && row.available);
        AIProviderStatus {
            preferred_provider: AIProvider::None,
            openai_available: openai,
            openai_api_key_set: false,
            phi_silica_available: false,
            phi_silica_ready: false,
            phi_silica_message: None,
            foundry_local_available: false,
            foundry_local_endpoint: None,
            active_provider: AIProvider::None,
            providers: rows,
        }
    }

    #[test]
    fn account_from_row_maps_installed_and_usable_to_states() {
        let installed_no_login = account_from_row(&row(
            AIProvider::CodexCli,
            false,
            true,
            Some(CliObstacle::NoStoredLogin),
        ))
        .unwrap();
        assert_eq!(installed_no_login.state, SubscriptionAuthState::SignedOut);
        assert_eq!(
            installed_no_login.obstacle,
            Some(CliObstacle::NoStoredLogin)
        );
        assert!(installed_no_login.installed());
        let signed_in = account_from_row(&row(AIProvider::ClaudeCode, true, true, None)).unwrap();
        assert_eq!(signed_in.state, SubscriptionAuthState::SignedIn);
        let absent = account_from_row(&row(AIProvider::CodexCli, false, false, None)).unwrap();
        assert_eq!(absent.state, SubscriptionAuthState::NotInstalled);
        assert!(account_from_row(&row(AIProvider::OpenAI, true, true, None)).is_none());
    }

    #[test]
    fn sign_in_requirement_only_when_no_other_provider_is_usable() {
        let codex_out = row(
            AIProvider::CodexCli,
            false,
            true,
            Some(CliObstacle::NoStoredLogin),
        );
        let only_codex = status(vec![codex_out.clone()]);
        let requirement = sign_in_requirement(AIProviderPreference::Auto, &only_codex).unwrap();
        assert_eq!(requirement.reason, SignInRequiredReason::OnlyCandidate);
        assert_eq!(requirement.obstacle, CliObstacle::NoStoredLogin);
        assert!(
            requirement
                .refusal("sending")
                .starts_with("Sign in to ChatGPT before sending")
        );

        let with_openai = status(vec![
            codex_out.clone(),
            row(AIProvider::OpenAI, true, true, None),
        ]);
        assert!(sign_in_requirement(AIProviderPreference::Auto, &with_openai).is_none());
        let explicit = sign_in_requirement(AIProviderPreference::CodexCli, &with_openai).unwrap();
        assert_eq!(explicit.reason, SignInRequiredReason::ExplicitPreference);

        let nothing_installed = status(vec![row(AIProvider::CodexCli, false, false, None)]);
        assert!(sign_in_requirement(AIProviderPreference::Auto, &nothing_installed).is_none());
        assert!(sign_in_requirement(AIProviderPreference::OpenAI, &only_codex).is_none());
    }

    #[test]
    fn batch_shim_is_an_install_requirement_not_a_sign_in() {
        let shim = status(vec![row(
            AIProvider::ClaudeCode,
            false,
            true,
            Some(CliObstacle::BatchShimOnly),
        )]);
        let requirement = sign_in_requirement(AIProviderPreference::ClaudeCode, &shim).unwrap();
        assert!(
            requirement
                .refusal("generating")
                .starts_with("Install the native Claude Code")
        );
        assert!(account_projection_allowed(&AccountState::default()));
        assert!(!account_projection_allowed(&AccountState {
            operation: Some(wfdiag_native_ai_chat::SubscriptionAuthOperation::SignIn),
            ..AccountState::default()
        }));
    }
    use wfdiag_native_ai_chat::{
        SubscriptionAuthOperation, SubscriptionAuthProvider, SubscriptionInstallFallbackReason,
        SubscriptionInstallMethod, SubscriptionInstallProgress, SubscriptionInstallStage,
    };

    #[test]
    fn account_and_install_work_are_mutually_exclusive() {
        assert_eq!(admit_auth(false, false, None), AuthAdmission::Start);
        assert!(matches!(
            admit_auth(true, false, None),
            AuthAdmission::Refuse { .. }
        ));
        assert!(
            matches!(admit_auth(false, true, None), AuthAdmission::Refuse { .. }),
            "an installer owns the same CLI's account files"
        );
        assert_eq!(
            admit_auth(false, false, Some("worker stopped")),
            AuthAdmission::Refuse {
                reason: "worker stopped".to_string()
            }
        );
    }

    #[test]
    fn a_status_probe_never_triggers_a_model_refresh() {
        assert!(!completion_refreshes_models(
            SubscriptionAuthOperation::Status
        ));
        assert!(completion_refreshes_models(
            SubscriptionAuthOperation::SignIn
        ));
        assert!(completion_refreshes_models(
            SubscriptionAuthOperation::SignOut
        ));
    }

    #[test]
    fn installation_always_starts_with_a_confirmation_and_never_a_process() {
        let provider = SubscriptionAuthProvider::Codex;
        assert_eq!(
            admit_install(provider, false, false, false, None),
            InstallAdmission::Confirm {
                prompt: InstallPrompt::Winget { provider }
            }
        );
        for (install, auth, prompt) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            assert!(matches!(
                admit_install(provider, install, auth, prompt, None),
                InstallAdmission::Refuse { .. }
            ));
        }
        assert!(matches!(
            admit_install(provider, false, false, false, Some("unavailable")),
            InstallAdmission::Refuse { .. }
        ));
    }

    #[test]
    fn the_vendor_fallback_is_a_separate_second_confirmation() {
        let provider = SubscriptionAuthProvider::ClaudeCode;
        let first = InstallPrompt::Winget { provider };
        assert_eq!(first.method(), SubscriptionInstallMethod::Winget);
        assert_eq!(first.provider(), provider);

        let second = InstallPrompt::VendorFallback {
            provider,
            reason: SubscriptionInstallFallbackReason::WingetFailed,
        };
        assert_eq!(second.method(), SubscriptionInstallMethod::VendorPowerShell);
        assert_ne!(
            first, second,
            "accepting the first never implies the second"
        );
    }

    #[test]
    fn every_installer_stage_has_a_label_and_paths_must_be_absolute() {
        let stages = [
            SubscriptionInstallStage::CheckingExisting,
            SubscriptionInstallStage::ResolvingInstaller,
            SubscriptionInstallStage::InstallingWinget,
            SubscriptionInstallStage::InstallingVendorFallback,
            SubscriptionInstallStage::Verifying,
            SubscriptionInstallStage::Completed,
        ];
        for stage in stages {
            let progress = SubscriptionInstallProgress {
                provider: SubscriptionAuthProvider::Codex,
                method: SubscriptionInstallMethod::Winget,
                stage,
            };
            assert!(!progress_label(&progress).is_empty());
        }
        assert!(!verified_install_path(std::path::Path::new("codex.cmd")));
        #[cfg(windows)]
        assert!(verified_install_path(std::path::Path::new(
            r"C:\Program Files\codex\codex.cmd"
        )));
        #[cfg(not(windows))]
        assert!(verified_install_path(std::path::Path::new(
            "/usr/local/bin/codex"
        )));
    }
}
