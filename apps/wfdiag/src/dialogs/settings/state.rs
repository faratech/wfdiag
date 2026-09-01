//! The Settings dialog's own state and message alphabet.
//!
//! The draft is the dialog's — it is what the user is editing and has not
//! committed. The persisted document lives in
//! [`crate::app::shell::ShellState::settings`]; persistence, validation and
//! the credential transaction all belong to the engine.

#![deny(unsafe_code)]

use crate::app::consts::PROVIDER_SETUP_LABELS;
use wfdiag_app::domain::catalog::CatalogState;
use wfdiag_app::domain::subscriptions::{AccountState, InstallPrompt};
use wfdiag_native_ai_chat::workers::subscription_install::SubscriptionInstallProgress;
use wfdiag_native_settings::{AppSettings, ProviderCredentialTransaction, ProviderKeyId};

/// Everything the Settings dialog renders.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct SettingsDialog {
    pub(crate) open: bool,
    pub(crate) draft: AppSettings,
    pub(crate) epoch: u64,
    /// The dialog epoch of the Save that is still being persisted.
    pub(crate) save_epoch: Option<u64>,
    pub(crate) loading: bool,
    pub(crate) saving: bool,
    pub(crate) error: Option<String>,
    pub(crate) save_error: Option<String>,

    // ---- provider setup ---------------------------------------------------
    pub(crate) provider_key_drafts: [String; ProviderKeyId::ALL.len()],
    pub(crate) credential_transaction: ProviderCredentialTransaction,
    pub(crate) provider_key_busy: bool,
    pub(crate) provider_setup_index: usize,
    pub(crate) provider_setup_error: Option<String>,
    pub(crate) provider_catalogs: Vec<CatalogState>,

    // ---- subscription CLIs ------------------------------------------------
    pub(crate) subscription_auth_error: Option<String>,
    pub(crate) subscription_auth_states: Vec<AccountState>,
    pub(crate) subscription_install_prompt: Option<InstallPrompt>,
    pub(crate) subscription_install_progress: Option<SubscriptionInstallProgress>,
    pub(crate) subscription_install_error: Option<String>,
    pub(crate) subscription_install_busy: bool,
}

impl SettingsDialog {
    /// The first frame's dialog, before anything is edited.
    pub(crate) fn new(defaults: AppSettings, open: bool, provider_setup_index: usize) -> Self {
        Self {
            open,
            draft: defaults,
            epoch: u64::from(open),
            save_epoch: None,
            loading: false,
            saving: false,
            error: None,
            save_error: None,
            provider_key_drafts: Default::default(),
            credential_transaction: ProviderCredentialTransaction::new(),
            provider_key_busy: false,
            provider_setup_index,
            provider_setup_error: None,
            provider_catalogs: (0..PROVIDER_SETUP_LABELS.len())
                .map(|_| CatalogState::default())
                .collect(),
            subscription_auth_error: None,
            subscription_auth_states: vec![AccountState::default(), AccountState::default()],
            subscription_install_prompt: None,
            subscription_install_progress: None,
            subscription_install_error: None,
            subscription_install_busy: false,
        }
    }
}
