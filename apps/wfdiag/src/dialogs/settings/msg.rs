//! The Settings dialog's message alphabet.

#![deny(unsafe_code)]

use crate::app::message::SettingsDialogAction;
use wfdiag_app::domain::subscriptions::InstallPrompt;
use wfdiag_native_ai_chat::SubscriptionAuthProvider;
use windows_reactor::*;

/// Everything the Settings dialog can ask for.
#[derive(Clone)]
pub(crate) enum SettingsMsg {
    Open,
    /// One control changed, or the dialog was committed or cancelled.
    Dialog {
        epoch: u64,
        action: SettingsDialogAction,
    },
    RefreshProviderModels,
    CancelProviderModels,
    RefreshSubscriptionAuth(SubscriptionAuthProvider),
    StartSubscriptionSignIn(SubscriptionAuthProvider),
    StartSubscriptionSignOut(SubscriptionAuthProvider),
    CancelSubscriptionAuth,
    RequestSubscriptionInstall(SubscriptionAuthProvider),
    SubscriptionInstallPromptClosed {
        prompt: InstallPrompt,
        result: ContentDialogResult,
    },
    CancelSubscriptionInstall,
    ProviderKeyDraftChanged(usize, String),
    StoreProviderKey(usize),
    ClearProviderKey(usize),
    ToggleQuickScanTask(String),
}
