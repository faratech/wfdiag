//! The shell's message alphabet and the small payload enums it carries.
//!
//! Every variant here is either a **user intent** or a **UI-side timer/dialog
//! callback**. There is no longer one variant per engine worker: worker output
//! arrives as a single [`Message::App`] batch of [`AppEvent`]s drained from
//! [`wfdiag_app::AppService`], which has already applied every staleness guard.
//! That is what removed the `*WorkerEventReceived` / `*Finished` / `*Rejected`
//! / `*WaitCancelled` / `*WorkerStopped` families.

#![deny(unsafe_code)]

use crate::app::state::{AiMode, FixPlanActionSelection};
use crate::platform::save_picker::{SavePickerReply, ValidatedSupportPackagePaths};
use crate::platform::window;
use wfdiag_app::AppEvent;
use wfdiag_app::domain::subscriptions::InstallPrompt;
use wfdiag_app::ports::monitor::ProcessSortKey;
use wfdiag_native_ai_chat::SubscriptionAuthProvider;
use wfdiag_native_projection::process_identity::ProcessIdentity;
use wfdiag_native_update::policy::AboutExternalAction;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaletteFocusAction {
    FocusQuery,
    RestorePrevious,
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

/// Which export payload a finished picker or write belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExportPickerKind {
    /// The single-file report export.
    File,
    /// The three-file support package.
    SupportPackage,
}

#[derive(Clone)]
pub(crate) enum Message {
    /// One coalesced native-window wake. The UI thread drains the application
    /// service and every remaining UI-owned signal source in response.
    NativeSignalReady,
    /// One drained batch of engine facts. Every staleness comparison already
    /// happened inside `AppService::drain`, so these are applied in order with
    /// no further guards.
    App(Vec<AppEvent>),
    /// First-publication handoff. Reactor commits native window commands
    /// before running view effects, so this is the earliest deterministic
    /// point where Win32 lifecycle/shortcut integration can discover the HWND.
    WindowHookBootstrap,
    Navigate(Option<String>),
    WindowSize(WindowSize),
    ColorSchemeChanged(ColorScheme),
    TogglePane,
    ToggleTheme,
    OpenAbout,
    AboutClosed {
        epoch: u64,
    },
    AboutExternalRequested {
        epoch: u64,
        action: AboutExternalAction,
    },
    AboutExternalFinished {
        epoch: u64,
        result: Result<(), String>,
    },
    AboutExternalRejected {
        epoch: u64,
    },
    UpdateNoticeClosed {
        epoch: u64,
    },
    UpdateNoticeExpired {
        epoch: u64,
        timer_generation: u64,
    },
    UpdateNoticePointerEntered {
        epoch: u64,
    },
    UpdateNoticePointerExited {
        epoch: u64,
    },
    UpdateNoticeTimerCancelled {
        epoch: u64,
        timer_generation: u64,
    },
    UpdateNoticeTimerRejected {
        epoch: u64,
        timer_generation: u64,
    },
    OpenSettings,
    SettingsDialog {
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
    RequestQuickScan,
    RequestFullScan,
    CancelScan,
    /// The off-UI-thread save picker answered (#140, #196). The epoch rejects
    /// an answer from a request the user has already superseded.
    ExportPickerFinished {
        epoch: u64,
        kind: ExportPickerKind,
        outcome: Box<SavePickerReply>,
    },
    ExportFileSaved {
        epoch: u64,
        result: Box<Result<std::path::PathBuf, String>>,
    },
    SupportPackageSaved {
        epoch: u64,
        result: Box<Result<ValidatedSupportPackagePaths, String>>,
    },
    SetAiMode(AiMode),
    ToggleMonitoring,
    Refresh,
    ProcessFilterChanged(String),
    ProcessSort(ProcessSortKey),
    ProcessPrevious,
    ProcessNext,
    /// The process-filter debounce elapsed. The engine is only asked for a
    /// page once the user stops typing.
    ProcessQueryDue {
        revision: u64,
    },
    ProcessQueryDebounceEnded {
        revision: u64,
    },
    SelectProcess(Option<ProcessIdentity>),
    RefreshHistory,
    HistoryFilterChanged(String),
    SelectHistory(String),
    ToggleHistoryTaskDetail(String),
    ChatInputChanged(String),
    UsePrompt(String),
    SendChat,
    GenerateReport,
    RegenerateReport,
    CancelReport,
    CancelPendingAiIntent,
    RetryPendingAiIntent,
    CopyReport,
    ExplainLatestScan,
    RunRemediation(String),
    AskAiAboutIssue(String),
    PrioritizeIssues,
    CancelIssuePrioritization,
    ProposeFixPlan,
    CancelFixPlan,
    ReviewFixPlanActions(FixPlanActionSelection),
    CancelChat,
    NewConversation,
    AllowCloudFallback,
    NeverCloudFallback,
    ApproveFullScan,
    DismissFullScan,
    TogglePalette,
    ClosePalette,
    PaletteFocusReady {
        epoch: u64,
        action: PaletteFocusAction,
    },
    PaletteFocusCancelled {
        epoch: u64,
    },
    PaletteFocusRejected {
        epoch: u64,
    },
    PaletteQueryChanged(String),
    PaletteActiveChanged(usize),
    PaletteCommand(String),
    ShowShortcutHelp,
    CloseShortcutHelp,
    ProviderKeyDraftChanged(usize, String),
    StoreProviderKey(usize),
    ClearProviderKey(usize),
    ToggleQuickScanTask(String),
    RequestNetworkConnections,
    DiagnosticFilterChanged(String),
    SetDiagnosticRaw(bool),
    SelectDiagnosticResult(String),
    AnalyzeSelectedDiagnostic,
    RetrySelectedDiagnosticAnalysis,
    CancelDiagnosticAnalysis,
    ToggleClearHistoryConfirm(bool),
    ClearHistoryConfirmed,
    BeginHistoryLabelEdit,
    CancelHistoryLabelEdit,
    HistoryLabelDraftChanged(String),
    SaveHistoryLabel,
    HistoryTagDraftChanged(String),
    SaveHistoryTags,
    RequestHistoryTrends,
    ActionReviewDialogClosed {
        proposal_id: String,
        result: ContentDialogResult,
    },
    RepairDialogClosed {
        proposal_id: String,
        result: ContentDialogResult,
    },
    CancelActionRun,
    ActionRunExpandedChanged {
        run_id: String,
        expanded: bool,
    },
    RestartAsAdmin,
    InstanceWaitCancelled,
    WindowHookRetryReady,
    WindowHookRetryRejected,
    InstanceActivated,
    WindowLifecycleChanged(window::WindowLifecycleSnapshot),
    GlobalShortcut(window::GlobalShortcutEvent),
    TrayCommand(u8),
}
