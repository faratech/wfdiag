//! Routing the Settings dialog's messages.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::subscription_auth_provider_for_setup;
use crate::dialogs::settings::msg::SettingsMsg;
use windows_reactor::*;

impl WfdiagShell {
    /// One Settings-dialog message.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn route_settings(
        &mut self,
        message: SettingsMsg,
        _context: &ComponentContext<Self>,
    ) {
        match message {
            SettingsMsg::ProviderKeyDraftChanged(index, value) => {
                if let Some(draft) = self.settings.provider_key_drafts.get_mut(index) {
                    *draft = value;
                    let setup_uses_key = matches!(
                        (self.settings.provider_setup_index, index),
                        (5, 0) | (6, 1) | (7, 2) | (8, 3) | (9, 4)
                    );
                    if setup_uses_key {
                        self.request_provider_model_refresh(false);
                    }
                }
            }
            SettingsMsg::StoreProviderKey(index) => self.submit_provider_key(index, true),
            SettingsMsg::ClearProviderKey(index) => {
                if let Some(draft) = self.settings.provider_key_drafts.get_mut(index) {
                    draft.clear();
                }
                self.submit_provider_key(index, false);
                self.request_provider_model_refresh(false);
            }
            SettingsMsg::ToggleQuickScanTask(task_id) => {
                let tasks = self
                    .settings
                    .draft
                    .quick_scan_tasks
                    .get_or_insert_with(Vec::new);
                match tasks.iter().position(|existing| existing == &task_id) {
                    Some(position) => {
                        tasks.remove(position);
                    }
                    None => tasks.push(task_id.clone()),
                }
                self.shell.status = format!(
                    "Quick Scan customization: {} tasks selected · press Save to apply",
                    tasks.len()
                );
            }
            SettingsMsg::Open => self.open_settings(),
            SettingsMsg::Dialog { epoch, action } => {
                self.apply_settings_dialog_action(epoch, action);
            }
            SettingsMsg::RefreshProviderModels => {
                if let Some(provider) =
                    subscription_auth_provider_for_setup(self.settings.provider_setup_index)
                {
                    self.begin_subscription_auth_operation(
                        provider,
                        wfdiag_app::SubscriptionOperation::Status,
                    );
                }
                self.request_provider_model_refresh(true);
            }
            SettingsMsg::CancelProviderModels => self.cancel_provider_model_request(),
            SettingsMsg::RefreshSubscriptionAuth(provider) => {
                self.begin_subscription_auth_operation(
                    provider,
                    wfdiag_app::SubscriptionOperation::Status,
                );
            }
            SettingsMsg::StartSubscriptionSignIn(provider) => {
                self.begin_subscription_auth_operation(
                    provider,
                    wfdiag_app::SubscriptionOperation::SignIn,
                );
            }
            SettingsMsg::StartSubscriptionSignOut(provider) => {
                self.begin_subscription_auth_operation(
                    provider,
                    wfdiag_app::SubscriptionOperation::SignOut,
                );
            }
            SettingsMsg::CancelSubscriptionAuth => self.cancel_subscription_auth(),
            SettingsMsg::RequestSubscriptionInstall(provider) => {
                self.request_subscription_install(provider);
            }
            SettingsMsg::SubscriptionInstallPromptClosed { prompt, result } => {
                if self.settings.subscription_install_prompt != Some(prompt) {
                    return;
                }
                self.answer_subscription_install_prompt(result == ContentDialogResult::Primary);
            }
            SettingsMsg::CancelSubscriptionInstall => self.cancel_subscription_install(),
        }
    }
}
