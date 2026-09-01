//! Settings dialog and persistence orchestration.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::{
    AI_PROVIDER_IDS, CODEX_MODEL_IDS, PROVIDER_SETUP_LABELS, SETTINGS_MAX_CONCURRENT_TASKS,
};
use crate::app::message::SettingsDialogAction;
use crate::app::policy::{
    apply_startup_scan_preference, configured_provider_setup_index,
    normalize_provider_preference_for_runtime, phi_preference_gate,
    provider_setup_index_for_provider, set_provider_key_configured, set_provider_key_value,
    set_provider_setup_model, settings_ai_status_probe_needed, settings_dialog_callback_is_current,
    strip_provider_key_values, take_matching_pending_settings_save,
    update_history_retention_policy, validate_phi_preference, window_theme_from_setting,
    window_theme_setting,
};
use crate::app::state::PendingSettingsSave;
use crate::platform::window;
use wfdiag_native_ai_provider::{AIProvider, AIProviderPreference, parse_provider_preference};
use wfdiag_native_issues::projection::advance_nonzero_generation;
use wfdiag_native_settings::{
    AppSettings, CloudFallbackPolicy, ProviderCredentialAction, ProviderKeyId, SettingsCommand,
    SettingsEvent,
};
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn next_settings_request_id(&mut self) -> u64 {
        self.settings_request_id = self.settings_request_id.wrapping_add(1);
        if self.settings_request_id == 0 {
            self.settings_request_id = 1;
        }
        self.settings_request_id
    }

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

    pub(crate) fn resume_settings_wait(&mut self, _context: &ComponentContext<Self>) {
        self.settings_wait = None;
    }

    pub(crate) fn persist_shell_settings(
        &mut self,
        submitted: AppSettings,
        context: &ComponentContext<Self>,
    ) -> bool {
        if self.deterministic_visual {
            self.settings_snapshot = submitted.clone();
            self.settings_draft = submitted;
            return true;
        }
        if self.settings_loading || self.settings_saving || self.settings_pending_save.is_some() {
            self.status = "Settings are already being saved…".to_string();
            return false;
        }
        if self.settings_runtime.is_none() {
            self.status = "Native settings persistence is unavailable".to_string();
            return false;
        }
        let Some(request_id) = advance_nonzero_generation(&mut self.settings_request_id) else {
            self.status = "Native settings request identity was exhausted".to_string();
            return false;
        };
        let command = SettingsCommand::Save {
            request_id,
            settings: Box::new(submitted.clone()),
        };
        if let Err(error) = self
            .settings_runtime
            .as_ref()
            .expect("settings runtime availability checked above")
            .send(command)
        {
            self.status = format!("Settings were not saved · {error}");
            return false;
        }
        self.settings_pending_save = Some(PendingSettingsSave {
            request_id,
            dialog_epoch: self.settings_dialog_epoch,
            submitted,
        });
        self.settings_saving = true;
        self.settings_save_error = None;
        self.resume_settings_wait(context);
        true
    }

    pub(crate) fn open_settings(&mut self, context: &ComponentContext<Self>) {
        if self.settings_open || self.settings_saving || self.about_open {
            return;
        }
        self.next_settings_dialog_epoch();
        self.settings_draft = self.settings_snapshot.clone();
        self.provider_setup_index = configured_provider_setup_index(&self.settings_snapshot);
        self.provider_key_drafts = Default::default();
        self.provider_credential_transaction.discard();
        self.provider_key_pending = None;
        self.provider_key_busy = false;
        self.theme = window_theme_from_setting(&self.settings_snapshot.theme);
        self.settings_save_error = None;
        self.subscription_install_prompt = None;
        self.subscription_install_error = None;
        self.settings_open = true;
        if settings_ai_status_probe_needed(
            self.settings_open,
            self.ai_provider_status.is_some(),
            self.ai_status_loading,
        ) {
            self.request_ai_provider_status_for_settings(context);
        }
        self.schedule_provider_model_refresh(context);
    }

    pub(crate) fn cancel_settings(&mut self, epoch: u64) {
        if !self.settings_dialog_is_current(epoch) || self.settings_saving {
            return;
        }
        self.settings_draft = self.settings_snapshot.clone();
        self.provider_key_drafts = Default::default();
        self.provider_credential_transaction.discard();
        self.provider_key_pending = None;
        self.provider_key_busy = false;
        self.theme = window_theme_from_setting(&self.settings_snapshot.theme);
        window::set_close_to_tray(self.settings_snapshot.close_to_tray);
        self.settings_save_error = None;
        self.settings_open = false;
        self.cancel_provider_model_request();
        self.cancel_subscription_auth();
        self.subscription_install_prompt = None;
        self.cancel_subscription_install();
    }

    pub(crate) fn apply_settings_dialog_action(
        &mut self,
        epoch: u64,
        action: SettingsDialogAction,
        context: &ComponentContext<Self>,
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
                if value
                    && settings_ai_status_probe_needed(
                        self.settings_open,
                        self.ai_provider_status.is_some(),
                        self.ai_status_loading,
                    )
                {
                    self.request_ai_provider_status_for_settings(context);
                }
            }
            SettingsDialogAction::PreferredAiProviderSelectionChanged(Some(index)) => {
                if let Some(provider) = AI_PROVIDER_IDS.get(index) {
                    let phi_gate = phi_preference_gate(
                        self.ai_provider_status.as_ref(),
                        self.ai_status_loading,
                    );
                    if let Err(reason) = validate_phi_preference(provider, &phi_gate) {
                        self.settings_save_error = Some(reason.clone());
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
                            self.schedule_provider_model_refresh(context);
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
                self.schedule_provider_model_refresh(context);
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
                    self.schedule_provider_model_refresh(context);
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
                    self.schedule_provider_model_refresh(context);
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
            self.settings_save_error = Some(reason.clone());
            self.status = "Settings were not saved · Phi Silica is not ready".to_string();
            return;
        }
        if self.deterministic_visual {
            self.settings_snapshot = self.settings_draft.clone();
            strip_provider_key_values(&mut self.settings_snapshot);
            self.settings_draft = self.settings_snapshot.clone();
            self.provider_key_drafts = Default::default();
            self.provider_credential_transaction.discard();
            window::set_close_to_tray(self.settings_snapshot.close_to_tray);
            self.settings_open = false;
            self.settings_save_error = None;
            self.status = "Settings saved".to_string();
            self.cancel_provider_model_request();
            self.cancel_subscription_auth();
            return;
        }
        if self.settings_loading || self.settings_saving {
            return;
        }
        if self.settings_runtime.is_none() {
            window::set_close_to_tray(self.settings_snapshot.close_to_tray);
            self.settings_save_error =
                Some("Native settings persistence is unavailable".to_string());
            return;
        }

        let request_id = self.next_settings_request_id();
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
        self.settings_pending_save = Some(PendingSettingsSave {
            request_id,
            dialog_epoch: epoch,
            submitted: submitted.clone(),
        });
        self.settings_saving = true;
        self.settings_save_error = None;
        let command = SettingsCommand::Save {
            request_id,
            settings: Box::new(submitted),
        };
        if let Err(error) = self
            .settings_runtime
            .as_ref()
            .expect("settings runtime availability checked above")
            .send(command)
        {
            self.settings_pending_save = None;
            self.settings_saving = false;
            window::set_close_to_tray(self.settings_snapshot.close_to_tray);
            self.settings_save_error = Some(error.to_string());
            self.status = "Settings were not saved".to_string();
        }
    }

    pub(crate) fn apply_settings_event(
        &mut self,
        event: SettingsEvent,
        context: &ComponentContext<Self>,
    ) {
        self.settings_wait = None;
        let mut worker_stopped = false;
        match event {
            SettingsEvent::Loaded { request_id, result } => {
                if self.settings_load_request_id != Some(request_id) {
                    self.resume_settings_wait(context);
                    return;
                }
                self.settings_load_request_id = None;
                self.settings_loading = false;
                match result {
                    Ok(mut settings) => {
                        normalize_provider_preference_for_runtime(&mut settings);
                        apply_startup_scan_preference(
                            &mut self.startup_scan_gate,
                            settings.scan_on_startup,
                        );
                        let provider_preference = settings.preferred_ai_provider.clone();
                        self.provider_setup_index = configured_provider_setup_index(&settings);
                        self.theme = window_theme_from_setting(&settings.theme);
                        self.pane_open = !settings.nav_rail_collapsed;
                        update_history_retention_policy(&self.history_retention_policy, &settings);
                        self.settings_snapshot = settings.clone();
                        self.settings_draft = settings;
                        self.provider_key_drafts = Default::default();
                        self.provider_credential_transaction.discard();
                        window::set_close_to_tray(self.settings_snapshot.close_to_tray);
                        self.settings_error = None;
                        self.ai_settings_ready = true;
                        self.sync_ai_provider_preference(&provider_preference, context);
                        if self.settings_open {
                            if settings_ai_status_probe_needed(
                                true,
                                self.ai_provider_status.is_some(),
                                self.ai_status_loading,
                            ) {
                                self.request_ai_provider_status_for_settings(context);
                            }
                            self.schedule_provider_model_refresh(context);
                        }
                    }
                    Err(error) => {
                        apply_startup_scan_preference(&mut self.startup_scan_gate, false);
                        self.ai_settings_ready = false;
                        self.ai_provider_status = None;
                        self.ai_status_loading = false;
                        self.ai_status_error = Some("AI settings could not be loaded".to_string());
                        self.settings_error = Some(error.to_string());
                        self.status = "Settings could not be loaded".to_string();
                    }
                }
                self.maybe_begin_startup_scan(context);
            }
            SettingsEvent::Saved { request_id, result } => {
                let Some(pending) = take_matching_pending_settings_save(
                    &mut self.settings_pending_save,
                    request_id,
                ) else {
                    self.resume_settings_wait(context);
                    return;
                };
                self.settings_saving = false;
                match result {
                    Ok(()) => {
                        let provider_preference = pending.submitted.preferred_ai_provider.clone();
                        let closes_current_dialog =
                            self.settings_dialog_is_current(pending.dialog_epoch);
                        update_history_retention_policy(
                            &self.history_retention_policy,
                            &pending.submitted,
                        );
                        let mut committed = pending.submitted;
                        strip_provider_key_values(&mut committed);
                        self.settings_snapshot = committed.clone();
                        self.pane_open = !self.settings_snapshot.nav_rail_collapsed;
                        window::set_close_to_tray(self.settings_snapshot.close_to_tray);
                        if closes_current_dialog || !self.settings_open {
                            self.settings_draft = committed;
                            self.theme = window_theme_from_setting(&self.settings_draft.theme);
                        }
                        self.provider_key_drafts = Default::default();
                        self.provider_credential_transaction.discard();
                        self.provider_key_pending = None;
                        self.provider_key_busy = false;
                        self.settings_error = None;
                        self.settings_save_error = None;
                        if closes_current_dialog {
                            self.settings_open = false;
                            self.cancel_provider_model_request();
                            self.cancel_subscription_auth();
                        }
                        self.status = "Settings saved".to_string();
                        self.sync_ai_provider_preference(&provider_preference, context);
                    }
                    Err(error) => {
                        window::set_close_to_tray(self.settings_snapshot.close_to_tray);
                        self.settings_save_error = Some(error.to_string());
                        self.status = "Settings were not saved".to_string();
                    }
                }
            }
            SettingsEvent::ProviderKeyStored { request_id, result } => {
                let Some(pending) = self
                    .provider_key_pending
                    .filter(|pending| pending.request_id == request_id && pending.store)
                else {
                    self.resume_settings_wait(context);
                    return;
                };
                self.provider_key_pending = None;
                self.provider_key_busy = false;
                let succeeded = result.is_ok();
                self.status = match result {
                    Ok(()) => "API key saved".to_string(),
                    Err(error) => format!("Credential change failed: {error}"),
                };
                if succeeded {
                    set_provider_key_configured(
                        &mut self.settings_snapshot,
                        pending.provider,
                        true,
                    );
                    set_provider_key_configured(&mut self.settings_draft, pending.provider, true);
                    if let Some(draft) = self.provider_key_drafts.get_mut(pending.index) {
                        draft.clear();
                    }
                    self.schedule_provider_model_refresh(context);
                }
            }
            SettingsEvent::ProviderKeyCleared { request_id, result } => {
                let Some(pending) = self
                    .provider_key_pending
                    .filter(|pending| pending.request_id == request_id && !pending.store)
                else {
                    self.resume_settings_wait(context);
                    return;
                };
                self.provider_key_pending = None;
                self.provider_key_busy = false;
                let succeeded = result.is_ok();
                self.status = match result {
                    Ok(()) => "API key cleared".to_string(),
                    Err(error) => format!("Credential change failed: {error}"),
                };
                if succeeded {
                    set_provider_key_configured(
                        &mut self.settings_snapshot,
                        pending.provider,
                        false,
                    );
                    set_provider_key_configured(&mut self.settings_draft, pending.provider, false);
                    if let Some(draft) = self.provider_key_drafts.get_mut(pending.index) {
                        draft.clear();
                    }
                    self.schedule_provider_model_refresh(context);
                }
            }
            SettingsEvent::ProviderCredentialsCommitted { request_id, result } => {
                // The staged Settings-dialog transaction integration owns
                // request correlation. Until one is pending, ignore a stale
                // completion rather than mutating credential indicators.
                let _ = (request_id, result);
            }
            SettingsEvent::Updated { request_id, result } => {
                let Some(pending) = self
                    .cloud_fallback_policy_update
                    .take()
                    .filter(|pending| pending.request_id == request_id)
                else {
                    self.resume_settings_wait(context);
                    return;
                };
                match result {
                    Ok(settings) => {
                        self.settings_snapshot.cloud_fallback_policy =
                            settings.cloud_fallback_policy;
                        self.settings_draft.cloud_fallback_policy = settings.cloud_fallback_policy;
                        match pending.policy {
                            CloudFallbackPolicy::Allow => {
                                self.continue_chat_fallback(pending.consent, context);
                            }
                            CloudFallbackPolicy::Never => {
                                let logical_request_id = pending.consent.attempt.logical_request_id;
                                self.finish_chat_attempt_failure(
                                    logical_request_id,
                                    "The local provider failed, and cloud fallback was declined."
                                        .to_string(),
                                );
                            }
                            CloudFallbackPolicy::Ask => {
                                self.cloud_fallback_consent = Some(pending.consent);
                                self.status =
                                    "Choose whether this request may use a cloud provider"
                                        .to_string();
                            }
                        }
                    }
                    Err(error) => {
                        self.cloud_fallback_consent = Some(pending.consent);
                        self.status = format!("Cloud fallback preference was not saved · {error}");
                    }
                }
            }
            SettingsEvent::Stopped => worker_stopped = true,
        }

        if worker_stopped {
            apply_startup_scan_preference(&mut self.startup_scan_gate, false);
            self.settings_loading = false;
            self.settings_saving = false;
            self.settings_load_request_id = None;
            self.settings_pending_save = None;
            self.provider_key_pending = None;
            self.provider_key_busy = false;
            if let Some(pending) = self.cloud_fallback_policy_update.take() {
                self.cloud_fallback_consent = Some(pending.consent);
            }
            window::set_close_to_tray(self.settings_snapshot.close_to_tray);
            self.settings_error = Some("Native settings worker stopped".to_string());
            self.settings_save_error = None;
            self.settings_receiver = None;
            self.settings_runtime = None;
            self.status = "Native settings persistence stopped".to_string();
        } else {
            self.resume_settings_wait(context);
        }
    }
}
