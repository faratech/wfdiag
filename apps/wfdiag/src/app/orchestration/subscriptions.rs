//! Subscription CLI sign-in and install orchestration.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::{
    subscription_auth_completion_refreshes_models, subscription_auth_provider_for_setup,
    subscription_auth_state_index, subscription_install_progress_label,
};
use crate::app::state::{
    PendingSubscriptionAuth, PendingSubscriptionInstall, SubscriptionInstallPrompt,
};
use wfdiag_native_ai_chat::workers::subscription_auth::{
    SubscriptionAuthState, SubscriptionAuthWorkerEvent,
};
use wfdiag_native_ai_chat::workers::subscription_install::{
    SubscriptionInstallMethod, SubscriptionInstallWorkerEvent,
};
use wfdiag_native_ai_chat::{SubscriptionAuthOperation, SubscriptionAuthProvider};
use wfdiag_native_issues::projection::advance_nonzero_generation;
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn resume_subscription_auth_wait(&mut self, _context: &ComponentContext<Self>) {
        self.subscription_auth_wait = None;
    }

    pub(crate) fn begin_subscription_auth_operation(
        &mut self,
        provider: SubscriptionAuthProvider,
        operation: SubscriptionAuthOperation,
        context: &ComponentContext<Self>,
    ) {
        if self.deterministic_visual || !self.settings_open {
            return;
        }
        if self.subscription_auth_pending.is_some() {
            self.status = "A subscription account action is already active…".to_string();
            return;
        }
        let Some(operation_id) =
            advance_nonzero_generation(&mut self.subscription_auth_operation_id)
        else {
            self.status = "Subscription account request identity was exhausted".to_string();
            return;
        };
        let draft_cli_path = match provider {
            SubscriptionAuthProvider::Codex => self.settings_draft.codex_cli_path.clone(),
            SubscriptionAuthProvider::ClaudeCode => self.settings_draft.claude_cli_path.clone(),
        };
        let Some(runtime) = self.subscription_auth_runtime.as_ref() else {
            let error = self
                .subscription_auth_error
                .clone()
                .unwrap_or_else(|| "Subscription account controls are unavailable".to_string());
            if let Some(state) = self
                .subscription_auth_states
                .get_mut(subscription_auth_state_index(provider))
            {
                state.error = Some(error);
                state.operation = None;
            }
            return;
        };
        let queued = match operation {
            SubscriptionAuthOperation::Status => {
                runtime.request_status(operation_id, provider, draft_cli_path)
            }
            SubscriptionAuthOperation::SignIn => {
                runtime.start_sign_in(operation_id, provider, draft_cli_path)
            }
            SubscriptionAuthOperation::SignOut => {
                runtime.start_sign_out(operation_id, provider, draft_cli_path)
            }
        };
        if !queued {
            if let Some(state) = self
                .subscription_auth_states
                .get_mut(subscription_auth_state_index(provider))
            {
                state.error = Some("The subscription account queue is busy".to_string());
                state.operation = None;
            }
            return;
        }
        self.subscription_auth_pending = Some(PendingSubscriptionAuth {
            operation_id,
            provider,
            operation,
            dialog_epoch: self.settings_dialog_epoch,
        });
        if let Some(state) = self
            .subscription_auth_states
            .get_mut(subscription_auth_state_index(provider))
        {
            state.operation = Some(operation);
            state.error = None;
        }
        self.status = match operation {
            SubscriptionAuthOperation::Status => format!("Checking {provider} account…"),
            SubscriptionAuthOperation::SignIn => {
                format!("Waiting for {provider} browser sign-in…")
            }
            SubscriptionAuthOperation::SignOut => format!("Signing out of {provider}…"),
        };
        self.resume_subscription_auth_wait(context);
    }

    pub(crate) fn cancel_subscription_auth(&mut self) {
        let Some(pending) = self.subscription_auth_pending else {
            return;
        };
        if self
            .subscription_auth_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.cancel(pending.operation_id))
            && self.settings_open
        {
            self.status = format!("Cancelling {} account action…", pending.provider);
        }
    }

    pub(crate) fn apply_subscription_auth_event(
        &mut self,
        event: SubscriptionAuthWorkerEvent,
        context: &ComponentContext<Self>,
    ) {
        self.subscription_auth_wait = None;
        let operation_id = event.operation_id();
        let Some(pending) = self
            .subscription_auth_pending
            .filter(|pending| pending.operation_id == operation_id)
        else {
            self.resume_subscription_auth_wait(context);
            return;
        };
        match event {
            SubscriptionAuthWorkerEvent::Ack { .. } => {
                self.resume_subscription_auth_wait(context);
                return;
            }
            SubscriptionAuthWorkerEvent::StatusLoaded { status, .. }
            | SubscriptionAuthWorkerEvent::Completed { status, .. } => {
                self.subscription_auth_pending = None;
                if let Some(state) = self
                    .subscription_auth_states
                    .get_mut(subscription_auth_state_index(status.provider))
                {
                    state.status = Some(status.clone());
                    state.operation = None;
                    state.error = None;
                }
                self.status = match status.state {
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
                self.request_ai_provider_status(context);
                if self.settings_dialog_is_current(pending.dialog_epoch)
                    && subscription_auth_provider_for_setup(self.provider_setup_index)
                        == Some(status.provider)
                    && subscription_auth_completion_refreshes_models(pending.operation)
                {
                    self.schedule_provider_model_refresh(context);
                }
            }
            SubscriptionAuthWorkerEvent::Failed {
                provider, message, ..
            } => {
                self.subscription_auth_pending = None;
                if let Some(state) = self
                    .subscription_auth_states
                    .get_mut(subscription_auth_state_index(provider))
                {
                    state.operation = None;
                    state.error = Some(message.clone());
                }
                self.status = format!("{provider} account action failed · {message}");
                self.request_ai_provider_status(context);
            }
            SubscriptionAuthWorkerEvent::Cancelled { provider, .. } => {
                self.subscription_auth_pending = None;
                if let Some(state) = self
                    .subscription_auth_states
                    .get_mut(subscription_auth_state_index(provider))
                {
                    state.operation = None;
                }
                if self.settings_open {
                    self.status = format!("{provider} account action cancelled");
                }
            }
        }
        self.resume_subscription_auth_wait(context);
    }

    pub(crate) fn stop_subscription_auth_delivery(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.subscription_auth_wait = None;
        self.subscription_auth_pending = None;
        self.subscription_auth_receiver = None;
        self.subscription_auth_runtime = None;
        self.subscription_auth_error = Some(reason.clone());
        for state in &mut self.subscription_auth_states {
            state.operation = None;
            state.error = Some(reason.clone());
        }
    }

    pub(crate) fn resume_subscription_install_wait(&mut self, _context: &ComponentContext<Self>) {
        self.subscription_install_wait = None;
    }

    pub(crate) fn request_subscription_install(&mut self, provider: SubscriptionAuthProvider) {
        if self.deterministic_visual || !self.settings_open {
            return;
        }
        if self.subscription_install_pending.is_some()
            || self.subscription_auth_pending.is_some()
            || self.subscription_install_prompt.is_some()
        {
            self.status = "A subscription CLI action is already active…".to_string();
            return;
        }
        if self.subscription_install_runtime.is_none() {
            self.status = self
                .subscription_install_error
                .clone()
                .unwrap_or_else(|| "Subscription CLI installation is unavailable".to_string());
            return;
        }
        self.subscription_install_prompt = Some(SubscriptionInstallPrompt::Winget {
            provider,
            dialog_epoch: self.settings_dialog_epoch,
        });
        self.subscription_install_error = None;
    }

    pub(crate) fn begin_subscription_install(
        &mut self,
        prompt: SubscriptionInstallPrompt,
        context: &ComponentContext<Self>,
    ) {
        let (provider, method, dialog_epoch) = match prompt {
            SubscriptionInstallPrompt::Winget {
                provider,
                dialog_epoch,
            } => (provider, SubscriptionInstallMethod::Winget, dialog_epoch),
            SubscriptionInstallPrompt::VendorFallback {
                provider,
                dialog_epoch,
                ..
            } => (
                provider,
                SubscriptionInstallMethod::VendorPowerShell,
                dialog_epoch,
            ),
        };
        if !self.settings_dialog_is_current(dialog_epoch)
            || self.subscription_install_pending.is_some()
        {
            return;
        }
        let Some(request_id) =
            advance_nonzero_generation(&mut self.subscription_install_request_id)
        else {
            self.subscription_install_error =
                Some("Subscription installer request identity was exhausted".to_string());
            return;
        };
        let Some(runtime) = self.subscription_install_runtime.as_ref() else {
            self.subscription_install_error =
                Some("Subscription CLI installation is unavailable".to_string());
            return;
        };
        let queued = match method {
            SubscriptionInstallMethod::Winget => {
                runtime.install_with_winget(request_id, provider, true)
            }
            SubscriptionInstallMethod::VendorPowerShell => {
                runtime.install_with_vendor_fallback(request_id, provider, true, true)
            }
        };
        if !queued {
            self.subscription_install_error =
                Some("The subscription installer is already busy".to_string());
            return;
        }
        self.subscription_install_pending = Some(PendingSubscriptionInstall {
            request_id,
            provider,
            method,
            dialog_epoch,
        });
        self.subscription_install_progress = None;
        self.subscription_install_error = None;
        self.status = format!("Starting the {provider} CLI installer…");
        self.resume_subscription_install_wait(context);
    }

    pub(crate) fn cancel_subscription_install(&mut self) {
        let Some(pending) = self.subscription_install_pending else {
            return;
        };
        if self
            .subscription_install_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.cancel(pending.request_id))
        {
            self.status = format!("Cancelling the {} CLI installer…", pending.provider);
        }
    }

    pub(crate) fn apply_subscription_install_event(
        &mut self,
        event: SubscriptionInstallWorkerEvent,
        context: &ComponentContext<Self>,
    ) {
        self.subscription_install_wait = None;
        let Some(pending) = self
            .subscription_install_pending
            .filter(|pending| pending.request_id == event.request_id())
        else {
            self.resume_subscription_install_wait(context);
            return;
        };
        match event {
            SubscriptionInstallWorkerEvent::Ack { .. } => {
                self.status = format!("Preparing the {} CLI installer…", pending.provider);
                self.resume_subscription_install_wait(context);
                return;
            }
            SubscriptionInstallWorkerEvent::Progress { progress, .. } => {
                self.subscription_install_progress = Some(progress);
                self.status = subscription_install_progress_label(progress).to_string();
                self.resume_subscription_install_wait(context);
                return;
            }
            SubscriptionInstallWorkerEvent::VendorFallbackConfirmationRequired {
                provider,
                reason,
                ..
            } => {
                self.subscription_install_pending = None;
                self.subscription_install_progress = None;
                if self.settings_dialog_is_current(pending.dialog_epoch) {
                    self.subscription_install_prompt =
                        Some(SubscriptionInstallPrompt::VendorFallback {
                            provider,
                            reason,
                            dialog_epoch: pending.dialog_epoch,
                        });
                    self.status =
                        "winget could not finish · vendor installer approval required".to_string();
                }
            }
            SubscriptionInstallWorkerEvent::Installed { status, .. } => {
                self.subscription_install_pending = None;
                self.subscription_install_progress = None;
                if !status.path.is_absolute() {
                    self.subscription_install_error = Some(
                        "The installer did not return a verified absolute CLI path".to_string(),
                    );
                    self.status = "CLI installation verification failed".to_string();
                    return;
                }
                let auth_status = status.auth_status();
                if let Some(state) = self
                    .subscription_auth_states
                    .get_mut(subscription_auth_state_index(status.provider))
                {
                    state.status = Some(auth_status);
                    state.operation = None;
                    state.error = None;
                }
                if self.settings_dialog_is_current(pending.dialog_epoch) {
                    let path = Some(status.path.to_string_lossy().into_owned());
                    match status.provider {
                        SubscriptionAuthProvider::Codex => {
                            self.settings_draft.codex_cli_path = path;
                        }
                        SubscriptionAuthProvider::ClaudeCode => {
                            self.settings_draft.claude_cli_path = path;
                        }
                    }
                    self.schedule_provider_model_refresh(context);
                }
                self.subscription_install_error = None;
                self.status = format!(
                    "{} CLI installed · account sign-in was not started",
                    status.provider
                );
                self.request_ai_provider_status(context);
            }
            SubscriptionInstallWorkerEvent::Failed {
                provider, message, ..
            } => {
                self.subscription_install_pending = None;
                self.subscription_install_progress = None;
                self.subscription_install_error = Some(message.clone());
                self.status = format!("{provider} CLI installation failed · {message}");
            }
            SubscriptionInstallWorkerEvent::Cancelled { provider, .. } => {
                self.subscription_install_pending = None;
                self.subscription_install_progress = None;
                self.status = format!("{provider} CLI installation cancelled");
            }
        }
        self.resume_subscription_install_wait(context);
    }

    pub(crate) fn stop_subscription_install_delivery(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.subscription_install_wait = None;
        self.subscription_install_pending = None;
        self.subscription_install_progress = None;
        self.subscription_install_receiver = None;
        self.subscription_install_runtime = None;
        self.subscription_install_error = Some(reason.clone());
        self.status = reason;
    }
}
