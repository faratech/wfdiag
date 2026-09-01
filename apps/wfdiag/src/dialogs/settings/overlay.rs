//! The Settings dialog's overlay: its status line, its content, and the
//! subscription-install confirmation that belongs to it.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::{Message, SettingsDialogAction};
use crate::app::policy::{
    phi_preference_gate, subscription_auth_provider_for_setup, subscription_auth_state_index,
};
use crate::app::screen::ShellEnv;
use crate::dialogs::settings::msg::SettingsMsg;
use crate::dialogs::settings::state::SettingsDialog;
use crate::dialogs::settings::view::settings_dialog;
use crate::fixtures::visual::VisualState;
use wfdiag_app::domain::subscriptions::InstallPrompt;
use wfdiag_native_ai_chat::SubscriptionAuthProvider;
use wfdiag_native_ai_chat::workers::subscription_install::SubscriptionInstallFallbackReason;
use wfdiag_native_ai_provider::AIProviderStatus;
use wfdiag_native_diagnostics::DiagnosticTask;
use windows_reactor::*;

impl SettingsDialog {
    /// The dialog itself, and the install prompt it can raise.
    ///
    /// Returns `(dialog, install_prompt)`: the shell hosts the first in the
    /// root grid and the second in the permanent overlay host.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn view(
        &self,
        env: &ShellEnv<'_>,
        provider_status: Option<&AIProviderStatus>,
        status_loading: bool,
        engine_running: bool,
        catalog: &[DiagnosticTask],
        vc: &mut ViewContext<WfdiagShell>,
    ) -> (View, View) {
        let settings_status = if self.loading {
            Some(("Loading settings…".to_string(), false))
        } else if self.saving {
            Some(("Saving settings…".to_string(), false))
        } else if let Some(error) = self.save_error.as_ref() {
            Some((format!("Settings were not saved: {error}"), true))
        } else {
            self.error.as_ref().map(|error| {
                (
                    format!("Settings persistence is unavailable: {error}"),
                    true,
                )
            })
        };
        let settings_editable = !self.loading && !self.saving && !self.subscription_install_busy;
        let settings_can_save = settings_editable && (env.deterministic_visual || engine_running);
        let settings_phi_gate = phi_preference_gate(provider_status, status_loading);
        let dialog: View = if self.open {
            let epoch = self.epoch;
            let subscription_provider =
                subscription_auth_provider_for_setup(self.provider_setup_index)
                    .unwrap_or(SubscriptionAuthProvider::Codex);
            let subscription_state = subscription_auth_provider_for_setup(
                self.provider_setup_index,
            )
            .and_then(|provider| {
                self.subscription_auth_states
                    .get(subscription_auth_state_index(provider))
            });
            let subscription_install_active = self.subscription_install_busy
                && subscription_auth_provider_for_setup(self.provider_setup_index)
                    == Some(subscription_provider);
            settings_dialog(
                env.palette,
                env.theme,
                env.visual_state == VisualState::SettingsBottom,
                &self.draft,
                &settings_phi_gate,
                env.deterministic_visual,
                self.provider_setup_index,
                self.provider_catalogs.get(self.provider_setup_index),
                subscription_state,
                self.subscription_auth_error.as_deref(),
                subscription_install_active,
                self.subscription_install_progress.as_ref(),
                self.subscription_install_error.as_deref(),
                settings_editable,
                settings_can_save,
                self.saving,
                settings_status,
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::ThemeSelectionChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::ExportFormatSelectionChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::AutoSaveChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::NotificationsChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::ScanOnStartupChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::CloseToTrayChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::MaxConcurrentTasksChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::AiEnabledChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::PreferredAiProviderSelectionChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::CloudFallbackSelectionChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::NetworkGroundingChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::CodexCliPathChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::CodexModelSelectionChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::ProviderSetupSelectionChanged(value),
                    })
                }),
                vc.callback(move |value| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::ProviderModelSelectionChanged(value),
                    })
                }),
                vc.message(Message::Settings(SettingsMsg::RefreshProviderModels)),
                vc.message(Message::Settings(SettingsMsg::CancelProviderModels)),
                vc.message(Message::Settings(SettingsMsg::RefreshSubscriptionAuth(
                    subscription_provider,
                ))),
                vc.message(Message::Settings(SettingsMsg::StartSubscriptionSignIn(
                    subscription_provider,
                ))),
                vc.message(Message::Settings(SettingsMsg::StartSubscriptionSignOut(
                    subscription_provider,
                ))),
                vc.message(Message::Settings(SettingsMsg::CancelSubscriptionAuth)),
                vc.message(Message::Settings(SettingsMsg::RequestSubscriptionInstall(
                    subscription_provider,
                ))),
                vc.message(Message::Settings(SettingsMsg::CancelSubscriptionInstall)),
                vc.callback(move |(field, value)| {
                    Message::Settings(SettingsMsg::Dialog {
                        epoch,
                        action: SettingsDialogAction::ProviderTextChanged(field, value),
                    })
                }),
                vc.message(Message::Settings(SettingsMsg::Dialog {
                    epoch,
                    action: SettingsDialogAction::Cancel,
                })),
                vc.message(Message::Settings(SettingsMsg::Dialog {
                    epoch,
                    action: SettingsDialogAction::Save,
                })),
                &self.provider_key_drafts,
                [
                    env.settings.open_ai_api_key_set,
                    env.settings.anthropic_api_key_set,
                    env.settings.gemini_api_key_set,
                    env.settings.deepseek_api_key_set,
                    env.settings.custom_api_key_set,
                ],
                self.provider_key_busy,
                vc.callback(move |(index, value)| {
                    Message::Settings(SettingsMsg::ProviderKeyDraftChanged(index, value))
                }),
                vc.callback(|value| Message::Settings(SettingsMsg::StoreProviderKey(value))),
                vc.callback(|value| Message::Settings(SettingsMsg::ClearProviderKey(value))),
                catalog,
                vc.callback(|value| Message::Settings(SettingsMsg::ToggleQuickScanTask(value))),
            )
        } else {
            View::empty()
        };

        let install_dialog: View = if let Some(prompt) = self.subscription_install_prompt {
            let (title, primary, body) = match prompt {
                InstallPrompt::Winget { provider } => {
                    let cli = match provider {
                        SubscriptionAuthProvider::Codex => "OpenAI Codex CLI",
                        SubscriptionAuthProvider::ClaudeCode => "Anthropic Claude Code CLI",
                    };
                    (
                        format!("Install {cli}?"),
                        "Install with winget",
                        format!(
                            "WFDiag will ask Windows Package Manager to install the official {cli} package. The installer runs only after this confirmation. After installation, WFDiag verifies the absolute executable path and does not sign in automatically."
                        ),
                    )
                }
                InstallPrompt::VendorFallback { provider, reason } => {
                    let cli = match provider {
                        SubscriptionAuthProvider::Codex => "OpenAI Codex CLI",
                        SubscriptionAuthProvider::ClaudeCode => "Anthropic Claude Code CLI",
                    };
                    let reason = match reason {
                        SubscriptionInstallFallbackReason::WingetUnavailable => {
                            "Windows Package Manager is unavailable"
                        }
                        SubscriptionInstallFallbackReason::WingetFailed => {
                            "Windows Package Manager could not complete the installation"
                        }
                        SubscriptionInstallFallbackReason::ExplicitApprovalMissing => {
                            "the vendor fallback has not been approved"
                        }
                    };
                    (
                        "Run the vendor PowerShell installer?".to_string(),
                        "Run vendor installer",
                        format!(
                            "{reason}. This is a separate approval to download and execute the current {cli} PowerShell bootstrap maintained by the vendor. Vendor scripts can change over time. WFDiag contains the process tree, applies a timeout, verifies the installed executable, and still does not sign in automatically."
                        ),
                    )
                }
            };
            let on_closed = vc.callback(move |result| {
                Message::Settings(SettingsMsg::SubscriptionInstallPromptClosed { prompt, result })
            });
            ContentDialog::new()
                .title(title)
                .is_open(true)
                .primary_button_text(primary)
                .secondary_button_text("Cancel")
                .on_closed(on_closed)
                .content(
                    Border::new()
                        .width(430.0)
                        .padding(Thickness::new(18.0, 14.0, 18.0, 14.0))
                        .background(env.palette.card_strong)
                        .content(
                            TextBlock::new()
                                .text(body)
                                .font_size(12.5)
                                .is_text_selection_enabled(true)
                                .text_wrapping(TextWrapping::Wrap),
                        ),
                )
        } else {
            View::empty()
        };
        (dialog, install_dialog)
    }
}
