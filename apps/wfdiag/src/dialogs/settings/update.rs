//! The Settings dialog: its draft, its epochs, its provider setup, its
//! subscription CLIs, and its one Save command.
//!
//! The draft is the dialog's — it is what the user is editing and has not
//! committed. Persistence, validation and the credential transaction all
//! belong to the engine, so Save is a single [`AppCommand::SaveSettings`] and
//! the answer arrives as a [`wfdiag_app::SettingsEvent`].
//!
//! These are `WfdiagShell` methods rather than `SettingsDialog` ones, which is
//! the one deliberate exception to the per-screen split: opening, cancelling
//! and saving all write shell state that
//! [`crate::app::screen::ScreenCx`] deliberately makes read-only (the
//! persisted settings document, the live theme, the navigation rail), and the
//! same `persist_shell_settings` is what the theme toggle and the rail toggle
//! call from outside the dialog.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::{
    AI_PROVIDER_IDS, CODEX_MODEL_IDS, PROVIDER_SETUP_LABELS, SETTINGS_MAX_CONCURRENT_TASKS,
};
use crate::app::message::SettingsDialogAction;
use crate::app::policy::{
    configured_provider_setup_index, phi_preference_gate, provider_catalog_draft,
    provider_display_name, provider_from_wire, provider_setup_index_for_provider,
    provider_setup_provider, rejection_text, set_provider_key_configured, set_provider_key_value,
    set_provider_setup_model, settings_dialog_callback_is_current, subscription_auth_state_index,
    validate_phi_preference, window_theme_from_setting, window_theme_setting,
};
use crate::platform::window;
use wfdiag_app::{
    AppCommand, DispatchOutcome, ModelCatalogEvent, ProviderCredentialCommand, ProviderEvent,
    SettingsEvent, SubscriptionEvent, SubscriptionOperation,
};
use wfdiag_native_ai_chat::SubscriptionAuthProvider;
use wfdiag_native_ai_provider::{AIProvider, AIProviderPreference, parse_provider_preference};
use wfdiag_native_settings::{
    AppSettings, CloudFallbackPolicy, ProviderCredentialAction, ProviderKeyId,
};
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn next_settings_dialog_epoch(&mut self) -> u64 {
        self.settings.epoch = self.settings.epoch.wrapping_add(1);
        if self.settings.epoch == 0 {
            self.settings.epoch = 1;
        }
        self.settings.epoch
    }

    pub(crate) fn settings_dialog_is_current(&self, epoch: u64) -> bool {
        settings_dialog_callback_is_current(self.settings.open, self.settings.epoch, epoch)
    }

    /// Adopt a document the engine persisted or reloaded.
    pub(crate) fn adopt_persisted_settings(&mut self, settings: &AppSettings, adopt_draft: bool) {
        self.shell.settings = settings.clone();
        self.settings.provider_setup_index = configured_provider_setup_index(settings);
        self.shell.pane_open = !settings.nav_rail_collapsed;
        window::set_close_to_tray(settings.close_to_tray);
        if adopt_draft {
            self.settings.draft = settings.clone();
            self.shell.theme = window_theme_from_setting(&settings.theme);
        }
        self.settings.provider_key_drafts = Default::default();
        self.settings.credential_transaction.discard();
        self.settings.provider_key_busy = false;
        self.settings.error = None;
    }

    /// Persist a settings change the shell made outside the dialog (the nav
    /// rail and the theme toggle).
    pub(crate) fn persist_shell_settings(&mut self, submitted: AppSettings) -> bool {
        if self.shell.deterministic_visual {
            self.shell.settings = submitted.clone();
            self.settings.draft = submitted;
            return true;
        }
        if self.settings.loading || self.settings.saving || self.settings.save_epoch.is_some() {
            self.shell.status = "Settings are already being saved…".to_string();
            return false;
        }
        match self.dispatch(AppCommand::SaveSettings(Box::new(submitted.clone()))) {
            DispatchOutcome::Accepted { .. } => {
                self.settings.save_epoch = Some(self.settings.epoch);
                self.settings.saving = true;
                self.settings.save_error = None;
                true
            }
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    self.shell.status =
                        format!("Settings were not saved · {}", rejection_text(reason));
                }
                false
            }
        }
    }

    pub(crate) fn open_settings(&mut self) {
        if self.settings.open || self.settings.saving || self.about.open {
            return;
        }
        self.next_settings_dialog_epoch();
        self.settings.draft = self.shell.settings.clone();
        self.settings.provider_setup_index = configured_provider_setup_index(&self.shell.settings);
        self.settings.provider_key_drafts = Default::default();
        self.settings.credential_transaction.discard();
        self.settings.provider_key_busy = false;
        self.shell.theme = window_theme_from_setting(&self.shell.settings.theme);
        self.settings.save_error = None;
        self.settings.subscription_install_error = None;
        self.settings.open = true;
        let _ = self.dispatch(AppCommand::RequestProviderStatus);
        self.probe_selected_subscription_account();
        self.request_provider_model_refresh(false);
    }

    pub(crate) fn cancel_settings(&mut self, epoch: u64) {
        if !self.settings_dialog_is_current(epoch) || self.settings.saving {
            return;
        }
        self.settings.draft = self.shell.settings.clone();
        self.settings.provider_key_drafts = Default::default();
        self.settings.credential_transaction.discard();
        self.settings.provider_key_busy = false;
        self.shell.theme = window_theme_from_setting(&self.shell.settings.theme);
        window::set_close_to_tray(self.shell.settings.close_to_tray);
        self.settings.save_error = None;
        self.settings.open = false;
        self.cancel_provider_model_request();
        self.cancel_subscription_auth();
        self.cancel_subscription_install();
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn apply_settings_dialog_action(
        &mut self,
        epoch: u64,
        action: SettingsDialogAction,
    ) {
        if !self.settings_dialog_is_current(epoch) {
            return;
        }
        if !self.settings.loading && !self.settings.saving && action.changes_draft() {
            self.settings.save_error = None;
        }

        match action {
            SettingsDialogAction::Cancel => self.cancel_settings(epoch),
            SettingsDialogAction::Save => self.request_settings_save(epoch),
            _ if self.settings.loading || self.settings.saving => {}
            SettingsDialogAction::ThemeSelectionChanged(Some(index)) => {
                self.shell.theme = match index {
                    0 => WindowTheme::System,
                    1 => WindowTheme::Light,
                    _ => WindowTheme::Dark,
                };
                self.settings.draft.theme = window_theme_setting(self.shell.theme).to_string();
            }
            SettingsDialogAction::ThemeSelectionChanged(None)
            | SettingsDialogAction::ExportFormatSelectionChanged(None)
            | SettingsDialogAction::PreferredAiProviderSelectionChanged(None)
            | SettingsDialogAction::CloudFallbackSelectionChanged(None)
            | SettingsDialogAction::CodexModelSelectionChanged(None)
            | SettingsDialogAction::ProviderModelSelectionChanged(None)
            | SettingsDialogAction::ProviderSetupSelectionChanged(None)
            | SettingsDialogAction::MaxConcurrentTasksChanged(None) => {}
            SettingsDialogAction::ExportFormatSelectionChanged(Some(index)) => {
                self.settings.draft.export_format = match index {
                    1 => "json",
                    2 => "html",
                    _ => "text",
                }
                .to_string();
            }
            SettingsDialogAction::AiEnabledChanged(value) => {
                self.settings.draft.ai_enabled = value;
                if value {
                    let _ = self.dispatch(AppCommand::RequestProviderStatus);
                }
            }
            SettingsDialogAction::PreferredAiProviderSelectionChanged(Some(index)) => {
                if let Some(provider) = AI_PROVIDER_IDS.get(index) {
                    let phi_gate = phi_preference_gate(
                        self.ai.provider_status.as_ref(),
                        self.ai.status_loading,
                    );
                    if let Err(reason) = validate_phi_preference(provider, &phi_gate) {
                        self.settings.save_error = Some(reason);
                        self.shell.status = "Phi Silica preference was not changed".to_string();
                        return;
                    }
                    self.settings.draft.preferred_ai_provider = (*provider).to_string();
                    let preference = parse_provider_preference(provider);
                    if preference != AIProviderPreference::Auto {
                        let selected_provider = match preference {
                            AIProviderPreference::Auto => AIProvider::None,
                            AIProviderPreference::OpenAI => AIProvider::OpenAI,
                            AIProviderPreference::PhiSilica => AIProvider::PhiSilica,
                            AIProviderPreference::FoundryLocal => AIProvider::FoundryLocal,
                            AIProviderPreference::Ollama => AIProvider::Ollama,
                            AIProviderPreference::CustomOpenAI => AIProvider::CustomOpenAI,
                            AIProviderPreference::CodexCli => AIProvider::CodexCli,
                            AIProviderPreference::ClaudeCode => AIProvider::ClaudeCode,
                            AIProviderPreference::Anthropic => AIProvider::Anthropic,
                            AIProviderPreference::Gemini => AIProvider::Gemini,
                            AIProviderPreference::DeepSeek => AIProvider::DeepSeek,
                        };
                        if let Some(setup_index) =
                            provider_setup_index_for_provider(selected_provider)
                        {
                            self.settings.provider_setup_index = setup_index;
                            self.request_provider_model_refresh(false);
                        }
                    }
                }
            }
            SettingsDialogAction::CloudFallbackSelectionChanged(Some(index)) => {
                self.settings.draft.cloud_fallback_policy = match index {
                    1 => CloudFallbackPolicy::Allow,
                    2 => CloudFallbackPolicy::Never,
                    _ => CloudFallbackPolicy::Ask,
                };
            }
            SettingsDialogAction::NetworkGroundingChanged(value) => {
                self.settings.draft.network_grounding_enabled = value;
            }
            SettingsDialogAction::CodexCliPathChanged(value) => {
                self.settings.draft.codex_cli_path =
                    if value.is_empty() { None } else { Some(value) };
                self.request_provider_model_refresh(false);
            }
            SettingsDialogAction::CodexModelSelectionChanged(Some(index)) => match index {
                0 => self.settings.draft.codex_model = None,
                1..=3 => {
                    self.settings.draft.codex_model =
                        CODEX_MODEL_IDS.get(index - 1).map(ToString::to_string);
                }
                _ => {}
            },
            SettingsDialogAction::ProviderSetupSelectionChanged(Some(index)) => {
                if index < PROVIDER_SETUP_LABELS.len() {
                    self.settings.provider_setup_index = index;
                    self.probe_selected_subscription_account();
                    self.request_provider_model_refresh(false);
                }
            }
            SettingsDialogAction::ProviderModelSelectionChanged(Some(index)) => {
                let model = if index == 0 {
                    None
                } else {
                    self.settings
                        .provider_catalogs
                        .get(self.settings.provider_setup_index)
                        .and_then(|state| state.catalog.as_ref())
                        .and_then(|catalog| catalog.models.get(index - 1))
                        .map(|model| model.id.clone())
                };
                if index == 0 || model.is_some() {
                    set_provider_setup_model(
                        self.settings.provider_setup_index,
                        &mut self.settings.draft,
                        model,
                    );
                }
            }
            SettingsDialogAction::ProviderTextChanged(field, value) => {
                let value = (!value.trim().is_empty()).then(|| value.trim().to_string());
                let refresh_catalog = matches!(field, 1 | 3 | 5 | 11);
                match field {
                    0 => self.settings.draft.phi_silica_laf_token = value,
                    1 => self.settings.draft.local_ai_endpoint = value,
                    2 => self.settings.draft.local_ai_model = value,
                    3 => self.settings.draft.ollama_endpoint = value,
                    4 => self.settings.draft.ollama_model = value,
                    5 => self.settings.draft.claude_cli_path = value,
                    6 => self.settings.draft.claude_model = value,
                    7 => self.settings.draft.open_ai_model = value,
                    8 => self.settings.draft.anthropic_model = value,
                    9 => self.settings.draft.gemini_model = value,
                    10 => self.settings.draft.deepseek_model = value,
                    11 => self.settings.draft.custom_endpoint = value,
                    12 => self.settings.draft.custom_model = value,
                    _ => {}
                }
                if refresh_catalog {
                    self.request_provider_model_refresh(false);
                }
            }
            SettingsDialogAction::ScanOnStartupChanged(value) => {
                self.settings.draft.scan_on_startup = value;
            }
            SettingsDialogAction::CloseToTrayChanged(value) => {
                self.settings.draft.close_to_tray = value;
            }
            SettingsDialogAction::MaxConcurrentTasksChanged(Some(value)) => {
                if value.is_finite() {
                    self.settings.draft.max_concurrent_tasks =
                        (value.round() as u32).clamp(1, SETTINGS_MAX_CONCURRENT_TASKS);
                }
            }
            SettingsDialogAction::AutoSaveChanged(value) => {
                self.settings.draft.auto_save = value;
            }
            SettingsDialogAction::NotificationsChanged(value) => {
                self.settings.draft.show_notifications = value;
            }
        }
    }

    pub(crate) fn request_settings_save(&mut self, epoch: u64) {
        if !self.settings_dialog_is_current(epoch) {
            return;
        }
        let phi_gate =
            phi_preference_gate(self.ai.provider_status.as_ref(), self.ai.status_loading);
        if let Err(reason) =
            validate_phi_preference(&self.settings.draft.preferred_ai_provider, &phi_gate)
        {
            self.settings.save_error = Some(reason);
            self.shell.status = "Settings were not saved · Phi Silica is not ready".to_string();
            return;
        }
        if self.shell.deterministic_visual {
            self.shell.settings = self.settings.draft.clone();
            crate::app::policy::strip_provider_key_values(&mut self.shell.settings);
            self.settings.draft = self.shell.settings.clone();
            self.settings.provider_key_drafts = Default::default();
            self.settings.credential_transaction.discard();
            window::set_close_to_tray(self.shell.settings.close_to_tray);
            self.settings.open = false;
            self.settings.save_error = None;
            self.shell.status = "Settings saved".to_string();
            return;
        }
        if self.settings.loading || self.settings.saving {
            return;
        }

        // The staged credential transaction is committed as one compensating
        // unit by the engine; the document itself carries only the
        // `*_configured` flags the staged actions imply.
        let mut submitted = self.settings.draft.clone();
        for (index, provider) in ProviderKeyId::ALL.into_iter().enumerate() {
            match self.settings.credential_transaction.staged_action(provider) {
                Some(ProviderCredentialAction::Store) => {
                    if let Some(draft) = self.settings.provider_key_drafts.get(index) {
                        set_provider_key_value(
                            &mut submitted,
                            provider,
                            Some(draft.trim().to_string()),
                        );
                    }
                }
                Some(ProviderCredentialAction::Clear) => {
                    set_provider_key_value(&mut submitted, provider, Some(String::new()));
                }
                None => set_provider_key_value(&mut submitted, provider, None),
            }
        }
        match self.dispatch(AppCommand::SaveSettings(Box::new(submitted.clone()))) {
            DispatchOutcome::Accepted { .. } => {
                self.settings.save_epoch = Some(epoch);
                self.settings.saving = true;
                self.settings.save_error = None;
                // The staged keys are written by the credential broker as one
                // compensating unit; the document above only carries the
                // `*_configured` flags they imply.
                self.commit_provider_credentials();
            }
            outcome => {
                window::set_close_to_tray(self.shell.settings.close_to_tray);
                if let Some(reason) = outcome.rejection() {
                    self.settings.save_error = Some(rejection_text(reason));
                }
                self.shell.status = "Settings were not saved".to_string();
            }
        }
    }

    /// Ask the engine for a subscription account probe when the dialog opens
    /// on a CLI provider.
    pub(crate) fn probe_selected_subscription_account(&mut self) {
        let Some(provider) = crate::app::policy::subscription_auth_provider_for_setup(
            self.settings.provider_setup_index,
        ) else {
            return;
        };
        let _: SubscriptionAuthProvider = provider;
        self.begin_subscription_auth_operation(provider, SubscriptionOperation::Status);
    }

    // ---- provider setup ---------------------------------------------------------------

    /// Stage a provider credential for the current Settings dialog. Storage is
    /// untouched until the dialog's primary Save action succeeds; Cancel
    /// discards the transaction and every plaintext draft.
    pub(crate) fn submit_provider_key(&mut self, index: usize, store: bool) {
        let Some(provider) = ProviderKeyId::ALL.get(index).copied() else {
            return;
        };
        let Some(key) = self.settings.provider_key_drafts.get(index).cloned() else {
            return;
        };
        if store && key.trim().is_empty() {
            self.shell.status = "Enter an API key first".to_string();
            return;
        }
        if store {
            self.settings
                .credential_transaction
                .stage_store(provider, key.trim().to_string());
            set_provider_key_configured(&mut self.settings.draft, provider, true);
            self.shell.status = "API key staged · press Save to commit".to_string();
        } else {
            self.settings.credential_transaction.stage_clear(provider);
            set_provider_key_configured(&mut self.settings.draft, provider, false);
            self.shell.status = "API key removal staged · press Save to commit".to_string();
        }
        self.settings.provider_key_busy = false;
        self.settings.save_error = None;
    }

    /// Ask the engine for the selected provider's model list.
    ///
    /// The 400 ms debounce, the cancel-and-retry latch and the "keep the last
    /// catalog visible" rule all live in the engine's own refresh policy, so
    /// this is one dispatch per edit rather than a shell timer.
    pub(crate) fn request_provider_model_refresh(&mut self, forced: bool) {
        if self.shell.deterministic_visual
            || !self.settings.open
            || provider_setup_provider(self.settings.provider_setup_index)
                == Some(wfdiag_native_ai_provider::AIProvider::PhiSilica)
        {
            return;
        }
        let Some(provider) = provider_setup_provider(self.settings.provider_setup_index) else {
            return;
        };
        let draft = match provider_catalog_draft(
            self.settings.provider_setup_index,
            &self.settings.draft,
            &self.settings.provider_key_drafts,
        ) {
            Ok(Some(draft)) => draft,
            // The provider has no catalog at all; show an empty pane.
            Ok(None) => {
                if let Some(state) = self
                    .settings
                    .provider_catalogs
                    .get_mut(self.settings.provider_setup_index)
                {
                    *state = wfdiag_app::domain::catalog::CatalogState::default();
                }
                return;
            }
            // Discovery cannot even be attempted with the current inputs.
            Err(blocked) => {
                if let Some(state) = self
                    .settings
                    .provider_catalogs
                    .get_mut(self.settings.provider_setup_index)
                {
                    state.blocked(blocked);
                }
                return;
            }
        };
        let outcome = self.dispatch(AppCommand::RefreshModelCatalog {
            provider: provider.to_string(),
            draft_api_key: draft.api_key,
            draft_endpoint: draft.endpoint,
            draft_cli_path: draft.cli_path,
            forced,
        });
        if let Some(reason) = outcome.rejection()
            && let Some(state) = self
                .settings
                .provider_catalogs
                .get_mut(self.settings.provider_setup_index)
        {
            state.failed(rejection_text(reason));
        }
    }

    pub(crate) fn cancel_provider_model_request(&mut self) {
        if self.dispatch(AppCommand::CancelModelCatalog).is_accepted() && self.settings.open {
            self.shell.status = "Cancelling model discovery…".to_string();
        }
    }

    // ---- subscription CLIs ----------------------------------------------------------------

    pub(crate) fn begin_subscription_auth_operation(
        &mut self,
        provider: SubscriptionAuthProvider,
        operation: SubscriptionOperation,
    ) {
        if self.shell.deterministic_visual || !self.settings.open {
            return;
        }
        let wire = match provider {
            SubscriptionAuthProvider::Codex => "codex_cli",
            SubscriptionAuthProvider::ClaudeCode => "claude_code",
        };
        match self.dispatch(AppCommand::SubscriptionAuth {
            provider: wire.to_string(),
            operation,
        }) {
            DispatchOutcome::Accepted { .. } => {
                if let Some(state) = self
                    .settings
                    .subscription_auth_states
                    .get_mut(subscription_auth_state_index(provider))
                {
                    state.error = None;
                }
                self.shell.status = match operation {
                    SubscriptionOperation::Status => format!("Checking {provider} account…"),
                    SubscriptionOperation::SignIn => {
                        format!("Waiting for {provider} browser sign-in…")
                    }
                    SubscriptionOperation::SignOut => format!("Signing out of {provider}…"),
                };
            }
            outcome => {
                if let Some(reason) = outcome.rejection()
                    && let Some(state) = self
                        .settings
                        .subscription_auth_states
                        .get_mut(subscription_auth_state_index(provider))
                {
                    state.error = Some(rejection_text(reason));
                    state.operation = None;
                }
            }
        }
    }

    pub(crate) fn cancel_subscription_auth(&mut self) {
        if self
            .dispatch(AppCommand::CancelSubscriptionAuth)
            .is_accepted()
            && self.settings.open
        {
            self.shell.status = "Cancelling the subscription account action…".to_string();
        }
    }

    pub(crate) fn request_subscription_install(&mut self, provider: SubscriptionAuthProvider) {
        if self.shell.deterministic_visual || !self.settings.open {
            return;
        }
        let wire = match provider {
            SubscriptionAuthProvider::Codex => "codex_cli",
            SubscriptionAuthProvider::ClaudeCode => "claude_code",
        };
        // This only raises the confirmation. Nothing is installed until the
        // user answers, and the vendor bootstrap raises a second one of its own.
        let outcome = self.dispatch(AppCommand::InstallSubscriptionCli {
            provider: wire.to_string(),
        });
        if let Some(reason) = outcome.rejection() {
            self.shell.status = rejection_text(reason);
        }
    }

    pub(crate) fn answer_subscription_install_prompt(&mut self, accepted: bool) {
        match self.dispatch(AppCommand::ConfirmSubscriptionInstall { accepted }) {
            DispatchOutcome::Accepted { .. } if accepted => {
                self.settings.subscription_install_busy = true;
                self.shell.status = "Starting the subscription CLI installer…".to_string();
            }
            DispatchOutcome::Accepted { .. } => {}
            outcome => self.report_rejection(&outcome),
        }
    }

    pub(crate) fn cancel_subscription_install(&mut self) {
        if self
            .dispatch(AppCommand::CancelSubscriptionInstall)
            .is_accepted()
        {
            self.shell.status = "Cancelling the subscription CLI installer…".to_string();
        }
    }

    /// Commit the staged credential transaction alongside a settings save.
    pub(crate) fn commit_provider_credentials(&mut self) {
        if self.settings.credential_transaction.is_empty() {
            return;
        }
        let transaction = self.settings.credential_transaction.clone();
        self.settings.provider_key_busy = true;
        let outcome = self.dispatch(AppCommand::ProviderCredential(
            ProviderCredentialCommand::Commit(Box::new(transaction)),
        ));
        if outcome.rejection().is_some() {
            self.settings.provider_key_busy = false;
            self.report_rejection(&outcome);
        }
    }

    // ---- engine events ------------------------------------------------------

    /// What is left of the provider stream once the AI screen has taken the
    /// provider status it renders: the Settings dialog's own surfaces.
    pub(crate) fn apply_provider_event(&mut self, event: ProviderEvent) {
        match event {
            ProviderEvent::PreferenceRejected { reason } => {
                self.settings.save_error = Some(reason);
                self.shell.status = "Phi Silica preference was not changed".to_string();
            }
            ProviderEvent::ModelCatalog(event) => self.apply_model_catalog_event(event),
            ProviderEvent::Subscription(event) => self.apply_subscription_event(*event),
            _ => {}
        }
    }

    fn apply_model_catalog_event(&mut self, event: ModelCatalogEvent) {
        match event {
            ModelCatalogEvent::Started { provider } => {
                self.shell.status = format!(
                    "Loading {} models…",
                    provider_display_name(provider_from_wire(&provider))
                );
            }
            ModelCatalogEvent::Loaded { provider, catalog } => {
                let count = catalog.models.len();
                self.shell.status = format!(
                    "Loaded {count} {} model{}",
                    provider_display_name(provider_from_wire(&provider)),
                    if count == 1 { "" } else { "s" }
                );
            }
            ModelCatalogEvent::Failed { error, .. } => {
                self.shell.status = format!("Model discovery failed · {error}");
            }
            ModelCatalogEvent::Throttled { .. } | ModelCatalogEvent::Cancelled { .. } => {}
            _ => {}
        }
    }

    #[allow(clippy::too_many_lines)]
    fn apply_subscription_event(&mut self, event: SubscriptionEvent) {
        use wfdiag_native_ai_chat::workers::subscription_auth::SubscriptionAuthState;
        match event {
            SubscriptionEvent::Started { .. } | SubscriptionEvent::InstallStarted { .. } => {}
            SubscriptionEvent::Status { status } | SubscriptionEvent::Completed { status, .. } => {
                self.shell.status = match status.state {
                    SubscriptionAuthState::NotInstalled => {
                        format!("{} CLI was not detected", status.provider)
                    }
                    SubscriptionAuthState::SignedOut => {
                        format!("{} is installed · sign-in required", status.provider)
                    }
                    SubscriptionAuthState::SignedIn => {
                        format!("{} account is signed in", status.provider)
                    }
                    SubscriptionAuthState::Unknown => {
                        format!("{} account status could not be confirmed", status.provider)
                    }
                };
            }
            SubscriptionEvent::Failed {
                provider, error, ..
            } => {
                self.settings.subscription_auth_error = Some(error.clone());
                self.shell.status = format!("{provider} account action failed · {error}");
            }
            SubscriptionEvent::Cancelled { provider, .. } => {
                if self.settings.open {
                    self.shell.status = format!("{provider} account action cancelled");
                }
            }
            SubscriptionEvent::InstallProgress { progress } => {
                self.settings.subscription_install_busy = true;
                self.shell.status =
                    crate::app::policy::subscription_install_progress_label(*progress).to_string();
            }
            SubscriptionEvent::InstallFallbackRequired { .. } => {
                self.settings.subscription_install_busy = false;
                self.shell.status =
                    "winget could not finish · vendor installer approval required".to_string();
            }
            SubscriptionEvent::Installed { status } => {
                self.settings.subscription_install_busy = false;
                if self.settings.open {
                    let path = Some(status.path.to_string_lossy().into_owned());
                    match status.provider {
                        wfdiag_native_ai_chat::SubscriptionAuthProvider::Codex => {
                            self.settings.draft.codex_cli_path = path;
                        }
                        wfdiag_native_ai_chat::SubscriptionAuthProvider::ClaudeCode => {
                            self.settings.draft.claude_cli_path = path;
                        }
                    }
                    self.request_provider_model_refresh(false);
                }
                self.shell.status = format!(
                    "{} CLI installed · account sign-in was not started",
                    status.provider
                );
            }
            SubscriptionEvent::InstallFailed {
                provider, error, ..
            } => {
                self.settings.subscription_install_busy = false;
                self.shell.status = format!("{provider} CLI installation failed · {error}");
            }
            SubscriptionEvent::InstallCancelled { provider, .. } => {
                self.settings.subscription_install_busy = false;
                self.shell.status = format!("{provider} CLI installation cancelled");
            }
            _ => {}
        }
    }

    // ---- settings -----------------------------------------------------------

    pub(crate) fn apply_settings_event(&mut self, event: SettingsEvent) {
        match event {
            SettingsEvent::Loaded { settings } => {
                self.adopt_persisted_settings(&settings, true);
            }
            SettingsEvent::Saved { settings } => {
                self.settings.saving = false;
                let closes_dialog = self
                    .settings
                    .save_epoch
                    .take()
                    .is_some_and(|epoch| self.settings_dialog_is_current(epoch));
                self.adopt_persisted_settings(&settings, closes_dialog || !self.settings.open);
                self.settings.save_error = None;
                if closes_dialog {
                    self.settings.open = false;
                    self.cancel_provider_model_request();
                    self.cancel_subscription_auth();
                }
                self.shell.status = "Settings saved".to_string();
            }
            SettingsEvent::Updated { settings } => {
                self.shell.settings.cloud_fallback_policy = settings.cloud_fallback_policy;
                self.settings.draft.cloud_fallback_policy = settings.cloud_fallback_policy;
            }
            SettingsEvent::CredentialsCommitted => {}
            SettingsEvent::Failed { error } => {
                self.settings.saving = false;
                self.settings.save_epoch = None;
                window::set_close_to_tray(self.shell.settings.close_to_tray);
                self.settings.save_error = Some(error);
                self.shell.status = "Settings were not saved".to_string();
            }
        }
    }
}
