//! The Settings dialog: its draft, its epochs, and its one Save command.
//!
//! The draft is the shell's — it is what the user is editing and has not
//! committed. Persistence, validation and the credential transaction all
//! belong to the engine, so Save is a single
//! [`AppCommand::SaveSettings`] and the answer arrives as a
//! [`wfdiag_app::SettingsEvent`].

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::{
    AI_PROVIDER_IDS, CODEX_MODEL_IDS, PROVIDER_SETUP_LABELS, SETTINGS_MAX_CONCURRENT_TASKS,
};
use crate::app::message::SettingsDialogAction;
use crate::app::policy::{
    configured_provider_setup_index, phi_preference_gate, provider_setup_index_for_provider,
    rejection_text, set_provider_key_value, set_provider_setup_model,
    settings_dialog_callback_is_current, validate_phi_preference, window_theme_from_setting,
    window_theme_setting,
};
use crate::platform::window;
use wfdiag_app::{AppCommand, DispatchOutcome, SubscriptionOperation};
use wfdiag_native_ai_chat::SubscriptionAuthProvider;
use wfdiag_native_ai_provider::{AIProvider, AIProviderPreference, parse_provider_preference};
use wfdiag_native_settings::{
    AppSettings, CloudFallbackPolicy, ProviderCredentialAction, ProviderKeyId,
};
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn next_settings_dialog_epoch(&mut self) -> u64 {
        self.settings_dialog_epoch = self.settings_dialog_epoch.wrapping_add(1);
        if self.settings_dialog_epoch == 0 {
            self.settings_dialog_epoch = 1;
        }
        self.settings_dialog_epoch
    }

    pub(crate) fn settings_dialog_is_current(&self, epoch: u64) -> bool {
        settings_dialog_callback_is_current(self.settings_open, self.settings_dialog_epoch, epoch)
    }

    /// Adopt a document the engine persisted or reloaded.
    pub(crate) fn adopt_persisted_settings(&mut self, settings: &AppSettings, adopt_draft: bool) {
        self.settings_snapshot = settings.clone();
        self.provider_setup_index = configured_provider_setup_index(settings);
        self.pane_open = !settings.nav_rail_collapsed;
        window::set_close_to_tray(settings.close_to_tray);
        if adopt_draft {
            self.settings_draft = settings.clone();
            self.theme = window_theme_from_setting(&settings.theme);
        }
        self.provider_key_drafts = Default::default();
        self.provider_credential_transaction.discard();
        self.provider_key_busy = false;
        self.settings_error = None;
    }

    /// Persist a settings change the shell made outside the dialog (the nav
    /// rail and the theme toggle).
    pub(crate) fn persist_shell_settings(&mut self, submitted: AppSettings) -> bool {
        if self.deterministic_visual {
            self.settings_snapshot = submitted.clone();
            self.settings_draft = submitted;
            return true;
        }
        if self.settings_loading || self.settings_saving || self.settings_save_epoch.is_some() {
            self.status = "Settings are already being saved…".to_string();
            return false;
        }
        match self.dispatch(AppCommand::SaveSettings(Box::new(submitted.clone()))) {
            DispatchOutcome::Accepted { .. } => {
                self.settings_save_epoch = Some(self.settings_dialog_epoch);
                self.settings_saving = true;
                self.settings_save_error = None;
                true
            }
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    self.status = format!("Settings were not saved · {}", rejection_text(reason));
                }
                false
            }
        }
    }

    pub(crate) fn open_settings(&mut self) {
        if self.settings_open || self.settings_saving || self.about_open {
            return;
        }
        self.next_settings_dialog_epoch();
        self.settings_draft = self.settings_snapshot.clone();
        self.provider_setup_index = configured_provider_setup_index(&self.settings_snapshot);
        self.provider_key_drafts = Default::default();
        self.provider_credential_transaction.discard();
        self.provider_key_busy = false;
        self.theme = window_theme_from_setting(&self.settings_snapshot.theme);
        self.settings_save_error = None;
        self.subscription_install_error = None;
        self.settings_open = true;
        let _ = self.dispatch(AppCommand::RequestProviderStatus);
        self.probe_selected_subscription_account();
        self.request_provider_model_refresh(false);
    }

    pub(crate) fn cancel_settings(&mut self, epoch: u64) {
        if !self.settings_dialog_is_current(epoch) || self.settings_saving {
            return;
        }
        self.settings_draft = self.settings_snapshot.clone();
        self.provider_key_drafts = Default::default();
        self.provider_credential_transaction.discard();
        self.provider_key_busy = false;
        self.theme = window_theme_from_setting(&self.settings_snapshot.theme);
        window::set_close_to_tray(self.settings_snapshot.close_to_tray);
        self.settings_save_error = None;
        self.settings_open = false;
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
        if !self.settings_loading && !self.settings_saving && action.changes_draft() {
            self.settings_save_error = None;
        }

        match action {
            SettingsDialogAction::Cancel => self.cancel_settings(epoch),
            SettingsDialogAction::Save => self.request_settings_save(epoch),
            _ if self.settings_loading || self.settings_saving => {}
            SettingsDialogAction::ThemeSelectionChanged(Some(index)) => {
                self.theme = match index {
                    0 => WindowTheme::System,
                    1 => WindowTheme::Light,
                    _ => WindowTheme::Dark,
                };
                self.settings_draft.theme = window_theme_setting(self.theme).to_string();
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
                self.settings_draft.export_format = match index {
                    1 => "json",
                    2 => "html",
                    _ => "text",
                }
                .to_string();
            }
            SettingsDialogAction::AiEnabledChanged(value) => {
                self.settings_draft.ai_enabled = value;
                if value {
                    let _ = self.dispatch(AppCommand::RequestProviderStatus);
                }
            }
            SettingsDialogAction::PreferredAiProviderSelectionChanged(Some(index)) => {
                if let Some(provider) = AI_PROVIDER_IDS.get(index) {
                    let phi_gate = phi_preference_gate(
                        self.ai_provider_status.as_ref(),
                        self.ai_status_loading,
                    );
                    if let Err(reason) = validate_phi_preference(provider, &phi_gate) {
                        self.settings_save_error = Some(reason);
                        self.status = "Phi Silica preference was not changed".to_string();
                        return;
                    }
                    self.settings_draft.preferred_ai_provider = (*provider).to_string();
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
                            self.provider_setup_index = setup_index;
                            self.request_provider_model_refresh(false);
                        }
                    }
                }
            }
            SettingsDialogAction::CloudFallbackSelectionChanged(Some(index)) => {
                self.settings_draft.cloud_fallback_policy = match index {
                    1 => CloudFallbackPolicy::Allow,
                    2 => CloudFallbackPolicy::Never,
                    _ => CloudFallbackPolicy::Ask,
                };
            }
            SettingsDialogAction::NetworkGroundingChanged(value) => {
                self.settings_draft.network_grounding_enabled = value;
            }
            SettingsDialogAction::CodexCliPathChanged(value) => {
                self.settings_draft.codex_cli_path =
                    if value.is_empty() { None } else { Some(value) };
                self.request_provider_model_refresh(false);
            }
            SettingsDialogAction::CodexModelSelectionChanged(Some(index)) => match index {
                0 => self.settings_draft.codex_model = None,
                1..=3 => {
                    self.settings_draft.codex_model =
                        CODEX_MODEL_IDS.get(index - 1).map(ToString::to_string);
                }
                _ => {}
            },
            SettingsDialogAction::ProviderSetupSelectionChanged(Some(index)) => {
                if index < PROVIDER_SETUP_LABELS.len() {
                    self.provider_setup_index = index;
                    self.probe_selected_subscription_account();
                    self.request_provider_model_refresh(false);
                }
            }
            SettingsDialogAction::ProviderModelSelectionChanged(Some(index)) => {
                let model = if index == 0 {
                    None
                } else {
                    self.provider_catalogs
                        .get(self.provider_setup_index)
                        .and_then(|state| state.catalog.as_ref())
                        .and_then(|catalog| catalog.models.get(index - 1))
                        .map(|model| model.id.clone())
                };
                if index == 0 || model.is_some() {
                    set_provider_setup_model(
                        self.provider_setup_index,
                        &mut self.settings_draft,
                        model,
                    );
                }
            }
            SettingsDialogAction::ProviderTextChanged(field, value) => {
                let value = (!value.trim().is_empty()).then(|| value.trim().to_string());
                let refresh_catalog = matches!(field, 1 | 3 | 5 | 11);
                match field {
                    0 => self.settings_draft.phi_silica_laf_token = value,
                    1 => self.settings_draft.local_ai_endpoint = value,
                    2 => self.settings_draft.local_ai_model = value,
                    3 => self.settings_draft.ollama_endpoint = value,
                    4 => self.settings_draft.ollama_model = value,
                    5 => self.settings_draft.claude_cli_path = value,
                    6 => self.settings_draft.claude_model = value,
                    7 => self.settings_draft.open_ai_model = value,
                    8 => self.settings_draft.anthropic_model = value,
                    9 => self.settings_draft.gemini_model = value,
                    10 => self.settings_draft.deepseek_model = value,
                    11 => self.settings_draft.custom_endpoint = value,
                    12 => self.settings_draft.custom_model = value,
                    _ => {}
                }
                if refresh_catalog {
                    self.request_provider_model_refresh(false);
                }
            }
            SettingsDialogAction::ScanOnStartupChanged(value) => {
                self.settings_draft.scan_on_startup = value;
            }
            SettingsDialogAction::CloseToTrayChanged(value) => {
                self.settings_draft.close_to_tray = value;
            }
            SettingsDialogAction::MaxConcurrentTasksChanged(Some(value)) => {
                if value.is_finite() {
                    self.settings_draft.max_concurrent_tasks =
                        (value.round() as u32).clamp(1, SETTINGS_MAX_CONCURRENT_TASKS);
                }
            }
            SettingsDialogAction::AutoSaveChanged(value) => {
                self.settings_draft.auto_save = value;
            }
            SettingsDialogAction::NotificationsChanged(value) => {
                self.settings_draft.show_notifications = value;
            }
        }
    }

    pub(crate) fn request_settings_save(&mut self, epoch: u64) {
        if !self.settings_dialog_is_current(epoch) {
            return;
        }
        let phi_gate =
            phi_preference_gate(self.ai_provider_status.as_ref(), self.ai_status_loading);
        if let Err(reason) =
            validate_phi_preference(&self.settings_draft.preferred_ai_provider, &phi_gate)
        {
            self.settings_save_error = Some(reason);
            self.status = "Settings were not saved · Phi Silica is not ready".to_string();
            return;
        }
        if self.deterministic_visual {
            self.settings_snapshot = self.settings_draft.clone();
            crate::app::policy::strip_provider_key_values(&mut self.settings_snapshot);
            self.settings_draft = self.settings_snapshot.clone();
            self.provider_key_drafts = Default::default();
            self.provider_credential_transaction.discard();
            window::set_close_to_tray(self.settings_snapshot.close_to_tray);
            self.settings_open = false;
            self.settings_save_error = None;
            self.status = "Settings saved".to_string();
            return;
        }
        if self.settings_loading || self.settings_saving {
            return;
        }

        // The staged credential transaction is committed as one compensating
        // unit by the engine; the document itself carries only the
        // `*_configured` flags the staged actions imply.
        let mut submitted = self.settings_draft.clone();
        for (index, provider) in ProviderKeyId::ALL.into_iter().enumerate() {
            match self.provider_credential_transaction.staged_action(provider) {
                Some(ProviderCredentialAction::Store) => {
                    if let Some(draft) = self.provider_key_drafts.get(index) {
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
                self.settings_save_epoch = Some(epoch);
                self.settings_saving = true;
                self.settings_save_error = None;
                // The staged keys are written by the credential broker as one
                // compensating unit; the document above only carries the
                // `*_configured` flags they imply.
                self.commit_provider_credentials();
            }
            outcome => {
                window::set_close_to_tray(self.settings_snapshot.close_to_tray);
                if let Some(reason) = outcome.rejection() {
                    self.settings_save_error = Some(rejection_text(reason));
                }
                self.status = "Settings were not saved".to_string();
            }
        }
    }

    /// Ask the engine for a subscription account probe when the dialog opens
    /// on a CLI provider.
    pub(crate) fn probe_selected_subscription_account(&mut self) {
        let Some(provider) =
            crate::app::policy::subscription_auth_provider_for_setup(self.provider_setup_index)
        else {
            return;
        };
        let _: SubscriptionAuthProvider = provider;
        self.begin_subscription_auth_operation(provider, SubscriptionOperation::Status);
    }
}
