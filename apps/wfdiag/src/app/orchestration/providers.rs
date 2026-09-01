//! AI provider status, model catalogue, and key orchestration.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::{
    AiWorkerKind, ai_worker_enabled, provider_catalog_request_for_draft, provider_display_name,
    provider_setup_provider, set_provider_key_configured,
};
use crate::app::state::{PendingProviderCatalogRequest, ProviderCatalogUiState};
use crate::app::tasks::{
    spawn_ai_preference_status_wait, spawn_ai_status_wait, spawn_provider_model_refresh_delay,
};
use wfdiag_native_ai_chat::workers::provider_setup::ProviderSetupWorkerEvent;
use wfdiag_native_ai_provider::AIProvider;
use wfdiag_native_issues::projection::advance_nonzero_generation;
use wfdiag_native_settings::{ProviderKeyId, SettingsService};
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn ai_worker_startup_settings(
        &self,
        worker: AiWorkerKind,
    ) -> Result<SettingsService, String> {
        if self.deterministic_visual {
            return Err(format!(
                "Visual fixture mode does not start the native {} worker",
                worker.display_name()
            ));
        }
        if !ai_worker_enabled(self.ai_worker_policy.as_os_str(), worker) {
            return Err(format!(
                "Native {} is disabled by the worker-isolation policy",
                worker.display_name()
            ));
        }
        self.ai_worker_settings.clone().ok_or_else(|| {
            format!(
                "Native {} cannot start because native settings are unavailable",
                worker.display_name()
            )
        })
    }

    pub(crate) fn ai_worker_available(&self, worker: AiWorkerKind) -> bool {
        !self.deterministic_visual
            && self.ai_worker_settings.is_some()
            && ai_worker_enabled(self.ai_worker_policy.as_os_str(), worker)
    }

    pub(crate) fn next_ai_status_request_id(&mut self) -> u64 {
        self.ai_status_request_id = self.ai_status_request_id.wrapping_add(1);
        if self.ai_status_request_id == 0 {
            self.ai_status_request_id = 1;
        }
        self.ai_status_request_id
    }

    pub(crate) fn request_ai_provider_status(&mut self, context: &ComponentContext<Self>) {
        self.request_ai_provider_status_with_disabled_policy(context, false);
    }

    pub(crate) fn request_ai_provider_status_for_settings(
        &mut self,
        context: &ComponentContext<Self>,
    ) {
        self.request_ai_provider_status_with_disabled_policy(context, true);
    }

    pub(crate) fn request_ai_provider_status_with_disabled_policy(
        &mut self,
        context: &ComponentContext<Self>,
        probe_while_disabled: bool,
    ) {
        if self.deterministic_visual {
            return;
        }

        if let Some(task) = self.ai_status_task.take() {
            task.cancel();
        }
        let request_id = self.next_ai_status_request_id();
        if !self.ai_settings_ready {
            self.ai_provider_status = None;
            self.ai_status_loading = self.settings_loading;
            self.ai_status_error =
                (!self.settings_loading).then(|| "AI settings could not be loaded".to_string());
            return;
        }
        if !probe_while_disabled && !self.settings_snapshot.ai_enabled {
            self.ai_provider_status = None;
            self.ai_status_loading = false;
            self.ai_status_error = None;
            return;
        }
        let Some(runtime) = self.ai_provider_runtime.as_ref() else {
            self.ai_provider_status = None;
            self.ai_status_loading = false;
            self.ai_status_error = Some("Native AI provider discovery is unavailable".to_string());
            return;
        };
        match runtime.request_status() {
            Ok(reply) => {
                self.ai_status_loading = true;
                self.ai_status_error = None;
                self.ai_status_task = Some(spawn_ai_status_wait(context, request_id, reply));
            }
            Err(error) => {
                self.ai_provider_status = None;
                self.ai_status_loading = false;
                self.ai_status_error = Some(error.to_string());
            }
        }
    }

    pub(crate) fn sync_ai_provider_preference(
        &mut self,
        preference: &str,
        context: &ComponentContext<Self>,
    ) {
        if self.deterministic_visual {
            return;
        }
        if !self.settings_snapshot.ai_enabled {
            if let Some(task) = self.ai_status_task.take() {
                task.cancel();
            }
            self.next_ai_status_request_id();
            self.ai_provider_status = None;
            self.ai_status_loading = false;
            self.ai_status_error = None;
            return;
        }
        if let Some(task) = self.ai_status_task.take() {
            task.cancel();
        }
        let request_id = self.next_ai_status_request_id();
        let Some(runtime) = self.ai_provider_runtime.as_ref() else {
            self.ai_provider_status = None;
            self.ai_status_loading = false;
            self.ai_status_error = Some("Native AI provider discovery is unavailable".to_string());
            return;
        };
        match runtime.request_set_preference_and_status(preference.to_string()) {
            Ok(reply) => {
                self.ai_provider_status = None;
                self.ai_status_loading = true;
                self.ai_status_error = None;
                self.ai_status_task =
                    Some(spawn_ai_preference_status_wait(context, request_id, reply));
            }
            Err(error) => {
                self.ai_provider_status = None;
                self.ai_status_loading = false;
                self.ai_status_error = Some(error.to_string());
            }
        }
    }

    /// Stage a provider credential for the current Settings dialog. Storage
    /// is untouched until the dialog's primary Save action succeeds; Cancel
    /// discards the transaction and every plaintext draft.
    pub(crate) fn submit_provider_key(&mut self, index: usize, store: bool) {
        let Some(provider) = ProviderKeyId::ALL.get(index).copied() else {
            return;
        };
        let Some(key) = self.provider_key_drafts.get(index).cloned() else {
            return;
        };
        if store && key.trim().is_empty() {
            self.status = "Enter an API key first".to_string();
            return;
        }
        if store {
            self.provider_credential_transaction
                .stage_store(provider, key.trim().to_string());
            set_provider_key_configured(&mut self.settings_draft, provider, true);
            self.status = "API key staged · press Save to commit".to_string();
        } else {
            self.provider_credential_transaction.stage_clear(provider);
            set_provider_key_configured(&mut self.settings_draft, provider, false);
            self.status = "API key removal staged · press Save to commit".to_string();
        }
        self.provider_key_pending = None;
        self.provider_key_busy = false;
        self.settings_save_error = None;
    }

    pub(crate) fn resume_provider_setup_wait(&mut self, _context: &ComponentContext<Self>) {
        self.provider_setup_wait = None;
    }

    pub(crate) fn schedule_provider_model_refresh(&mut self, context: &ComponentContext<Self>) {
        if let Some(task) = self.provider_catalog_refresh_task.take() {
            task.cancel();
        }
        self.provider_catalog_refresh_revision =
            self.provider_catalog_refresh_revision.wrapping_add(1);
        if self.provider_catalog_refresh_revision == 0 {
            self.provider_catalog_refresh_revision = 1;
        }
        if self.deterministic_visual
            || !self.settings_open
            || provider_setup_provider(self.provider_setup_index) == Some(AIProvider::PhiSilica)
        {
            return;
        }
        self.provider_catalog_refresh_task = Some(spawn_provider_model_refresh_delay(
            context,
            self.settings_dialog_epoch,
            self.provider_catalog_refresh_revision,
            self.provider_setup_index,
        ));
    }

    pub(crate) fn begin_provider_model_refresh(
        &mut self,
        setup_index: usize,
        context: &ComponentContext<Self>,
    ) {
        if self.deterministic_visual
            || !self.settings_open
            || setup_index != self.provider_setup_index
        {
            return;
        }
        if let Some(pending) = self.provider_catalog_pending {
            self.provider_catalog_refresh_after_cancel = true;
            if let Some(runtime) = self.provider_setup_runtime.as_ref() {
                let _ = runtime.cancel(pending.request_id);
            }
            self.status = "Refreshing the provider model list…".to_string();
            return;
        }

        let request = match provider_catalog_request_for_draft(
            setup_index,
            &self.settings_draft,
            &self.provider_key_drafts,
        ) {
            Ok(Some(request)) => request,
            Ok(None) => {
                if let Some(state) = self.provider_catalogs.get_mut(setup_index) {
                    *state = ProviderCatalogUiState::default();
                }
                return;
            }
            Err(blocked) => {
                if let Some(state) = self.provider_catalogs.get_mut(setup_index) {
                    state.loading = false;
                    state.error = None;
                    state.blocked = Some(blocked);
                    state.stale = state.catalog.is_some();
                }
                return;
            }
        };
        let Some(runtime) = self.provider_setup_runtime.as_ref() else {
            let error = self
                .provider_setup_error
                .clone()
                .unwrap_or_else(|| "Native model discovery is unavailable".to_string());
            if let Some(state) = self.provider_catalogs.get_mut(setup_index) {
                state.loading = false;
                state.blocked = None;
                state.error = Some(error);
                state.stale = state.catalog.is_some();
            }
            return;
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.provider_catalog_request_id)
        else {
            if let Some(state) = self.provider_catalogs.get_mut(setup_index) {
                state.error = Some("Model discovery request identity was exhausted".to_string());
            }
            return;
        };
        let provider = request.provider;
        if !runtime.list_models(request_id, request) {
            if let Some(state) = self.provider_catalogs.get_mut(setup_index) {
                state.loading = false;
                state.error = Some("The native model discovery queue is busy".to_string());
                state.stale = state.catalog.is_some();
            }
            return;
        }
        self.provider_catalog_pending = Some(PendingProviderCatalogRequest {
            request_id,
            provider,
            setup_index,
            dialog_epoch: self.settings_dialog_epoch,
        });
        self.provider_catalog_refresh_after_cancel = false;
        if let Some(state) = self.provider_catalogs.get_mut(setup_index) {
            state.loading = true;
            state.error = None;
            state.blocked = None;
        }
        self.status = format!("Loading {} models…", provider_display_name(provider));
        self.resume_provider_setup_wait(context);
    }

    pub(crate) fn cancel_provider_model_request(&mut self) {
        self.provider_catalog_refresh_after_cancel = false;
        if let Some(task) = self.provider_catalog_refresh_task.take() {
            task.cancel();
        }
        if let Some(pending) = self.provider_catalog_pending
            && self
                .provider_setup_runtime
                .as_ref()
                .is_some_and(|runtime| runtime.cancel(pending.request_id))
            && self.settings_open
        {
            self.status = "Cancelling model discovery…".to_string();
        }
    }

    pub(crate) fn apply_provider_setup_event(
        &mut self,
        event: ProviderSetupWorkerEvent,
        context: &ComponentContext<Self>,
    ) {
        self.provider_setup_wait = None;
        let request_id = event.request_id();
        let Some(pending) = self
            .provider_catalog_pending
            .filter(|pending| pending.request_id == request_id)
        else {
            self.resume_provider_setup_wait(context);
            return;
        };
        match event {
            ProviderSetupWorkerEvent::Ack { provider, .. } => {
                if provider == pending.provider {
                    self.status = format!("Loading {} models…", provider_display_name(provider));
                }
                self.resume_provider_setup_wait(context);
                return;
            }
            ProviderSetupWorkerEvent::ModelsLoaded {
                provider, catalog, ..
            } => {
                self.provider_catalog_pending = None;
                if provider == pending.provider
                    && self.settings_dialog_is_current(pending.dialog_epoch)
                    && self.provider_setup_index == pending.setup_index
                    && let Some(state) = self.provider_catalogs.get_mut(pending.setup_index)
                {
                    let count = catalog.models.len();
                    state.catalog = Some(catalog);
                    state.loading = false;
                    state.error = None;
                    state.blocked = None;
                    state.stale = false;
                    self.status = format!(
                        "Loaded {count} {} model{}",
                        provider_display_name(provider),
                        if count == 1 { "" } else { "s" }
                    );
                }
            }
            ProviderSetupWorkerEvent::Failed {
                provider, message, ..
            } => {
                self.provider_catalog_pending = None;
                if provider == pending.provider
                    && self.settings_dialog_is_current(pending.dialog_epoch)
                    && let Some(state) = self.provider_catalogs.get_mut(pending.setup_index)
                {
                    state.loading = false;
                    state.error = Some(message.clone());
                    state.blocked = None;
                    state.stale = state.catalog.is_some();
                    self.status = format!("Model discovery failed · {message}");
                }
            }
            ProviderSetupWorkerEvent::Cancelled { .. } => {
                self.provider_catalog_pending = None;
                if let Some(state) = self.provider_catalogs.get_mut(pending.setup_index) {
                    state.loading = false;
                }
            }
        }
        let refresh_after_cancel = self.provider_catalog_refresh_after_cancel;
        self.provider_catalog_refresh_after_cancel = false;
        if refresh_after_cancel && self.settings_open {
            self.begin_provider_model_refresh(self.provider_setup_index, context);
        }
        self.resume_provider_setup_wait(context);
    }

    pub(crate) fn stop_provider_setup_delivery(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if let Some(task) = self.provider_catalog_refresh_task.take() {
            task.cancel();
        }
        self.provider_setup_wait = None;
        self.provider_catalog_pending = None;
        self.provider_setup_receiver = None;
        self.provider_setup_runtime = None;
        self.provider_setup_error = Some(reason.clone());
        if let Some(state) = self.provider_catalogs.get_mut(self.provider_setup_index) {
            state.loading = false;
            state.error = Some(reason);
            state.stale = state.catalog.is_some();
        }
    }
}
