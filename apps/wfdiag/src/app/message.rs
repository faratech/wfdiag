//! The shell's message alphabet.
//!
//! Every variant is either a **user intent**, a **UI-side timer/dialog
//! callback**, or one drained batch of engine facts. There is no longer one
//! variant per engine worker: worker output arrives as a single
//! [`Message::App`] batch of [`AppEvent`]s drained from
//! [`wfdiag_app::AppService`], which has already applied every staleness
//! guard.
//!
//! The alphabet is one level deep on purpose. Every message except the four
//! Reactor-level ones belongs to exactly one screen or dialog, and names that
//! owner: `Message::Processes(..)` is routed to `ProcessesScreen::update` and
//! can touch nothing else. That is what keeps [`crate::app::WfdiagShell`] a
//! dispatcher rather than a god object.

#![deny(unsafe_code)]

use crate::app::native_msg::NativeMsg;
use crate::app::shell_msg::ShellMsg;
use crate::dialogs::about::state::AboutMsg;
use crate::dialogs::action_review::state::ActionReviewMsg;
use crate::dialogs::export::msg::ExportMsg;
use crate::dialogs::palette::msg::PaletteMsg;
use crate::dialogs::settings::msg::SettingsMsg;
use crate::dialogs::shortcuts_help::state::ShortcutHelpMsg;
use crate::dialogs::update_notice::state::UpdateNoticeMsg;
use crate::screens::ai::state::AiMsg;
use crate::screens::diagnostics::state::DiagnosticsMsg;
use crate::screens::history::state::HistoryMsg;
use crate::screens::issues::state::IssuesMsg;
use crate::screens::monitor::state::MonitorMsg;
use crate::screens::processes::state::ProcessesMsg;
use wfdiag_app::AppEvent;
use windows_reactor::*;

#[derive(Clone)]
pub(crate) enum SettingsDialogAction {
    ThemeSelectionChanged(Option<usize>),
    ExportFormatSelectionChanged(Option<usize>),
    AiEnabledChanged(bool),
    PreferredAiProviderSelectionChanged(Option<usize>),
    CloudFallbackSelectionChanged(Option<usize>),
    NetworkGroundingChanged(bool),
    CodexCliPathChanged(String),
    CodexModelSelectionChanged(Option<usize>),
    ProviderSetupSelectionChanged(Option<usize>),
    ProviderModelSelectionChanged(Option<usize>),
    ProviderTextChanged(usize, String),
    ScanOnStartupChanged(bool),
    CloseToTrayChanged(bool),
    MaxConcurrentTasksChanged(Option<f64>),
    AutoSaveChanged(bool),
    NotificationsChanged(bool),
    Cancel,
    Save,
}

impl SettingsDialogAction {
    pub(crate) fn changes_draft(&self) -> bool {
        !matches!(
            self,
            Self::ThemeSelectionChanged(None)
                | Self::ExportFormatSelectionChanged(None)
                | Self::PreferredAiProviderSelectionChanged(None)
                | Self::CloudFallbackSelectionChanged(None)
                | Self::CodexModelSelectionChanged(None)
                | Self::ProviderModelSelectionChanged(None)
                | Self::ProviderSetupSelectionChanged(_)
                | Self::MaxConcurrentTasksChanged(None)
                | Self::Cancel
                | Self::Save
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryChangeKind {
    Regressed,
    Recovered,
    Changed,
}

impl HistoryChangeKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Regressed => "regressed",
            Self::Recovered => "recovered",
            Self::Changed => "changed",
        }
    }
}

#[derive(Clone)]
pub(crate) enum Message {
    /// One coalesced native-window wake. The UI thread drains the application
    /// service and every remaining UI-owned signal source in response.
    NativeSignalReady,
    /// First-publication handoff. Reactor commits native window commands
    /// before running view effects, so this is the earliest deterministic
    /// point where Win32 lifecycle/shortcut integration can discover the HWND.
    WindowHookBootstrap,
    WindowSize(WindowSize),
    ColorSchemeChanged(ColorScheme),
    /// One Win32 lifecycle, instance, global-shortcut or tray signal.
    Native(NativeMsg),
    /// One drained batch of engine facts. Every staleness comparison already
    /// happened inside `AppService::drain`, so these are applied in order with
    /// no further guards.
    App(Vec<AppEvent>),
    /// One chrome message: navigation, the pane, the theme, refresh.
    Shell(ShellMsg),
    Diagnostics(DiagnosticsMsg),
    Monitor(MonitorMsg),
    Processes(ProcessesMsg),
    Ai(AiMsg),
    Issues(IssuesMsg),
    History(HistoryMsg),
    Settings(SettingsMsg),
    About(AboutMsg),
    UpdateNotice(UpdateNoticeMsg),
    Export(ExportMsg),
    Palette(PaletteMsg),
    Shortcuts(ShortcutHelpMsg),
    ActionReview(ActionReviewMsg),
}
