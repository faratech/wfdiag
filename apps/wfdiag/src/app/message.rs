//! The shell's message alphabet and the small payload enums it carries.

#![deny(unsafe_code)]

use crate::app::state::{
    AiMode, FixPlanActionSelection, HistoryTaskDiffProjection, SubscriptionInstallPrompt,
};
use crate::platform::save_picker::ValidatedSupportPackagePaths;
use crate::platform::window;
use wfdiag_native_ai_analysis::{AnalysisWorkerEvent, FixPlanWorkerEvent};
use wfdiag_native_ai_chat::workers::provider_setup::ProviderSetupWorkerEvent;
use wfdiag_native_ai_chat::workers::subscription_auth::SubscriptionAuthWorkerEvent;
use wfdiag_native_ai_chat::workers::subscription_install::SubscriptionInstallWorkerEvent;
use wfdiag_native_ai_chat::{ChatWorkerEvent, SubscriptionAuthProvider};
use wfdiag_native_ai_provider::AIProviderStatus;
use wfdiag_native_ai_report::{ReportGeneration, ReportWorkerEvent};
use wfdiag_native_diagnostics::{ScanKind, SharedScanEvidence};
use wfdiag_native_export::ExportCompleted;
use wfdiag_native_history::{ComparisonSummary, ScanSummary, TaskTrend};
use wfdiag_native_issues::IssueDetectionCompleted;
use wfdiag_native_issues::projection::{PendingIssueDetection, PreparedIssueDetection};
use wfdiag_native_monitor::{NetworkConnection, ProcessPage, ProcessSortKey};
use wfdiag_native_projection::process_identity::ProcessIdentity;
use wfdiag_native_remediation::runtime::{ActionRunEvent, ActionWorkerEvent};
use wfdiag_native_settings::SettingsEvent;
use wfdiag_native_system::SystemCompleted;
use wfdiag_native_update::UpdateInfo;
use wfdiag_native_update::policy::{AboutExternalAction, UpdateThrottle};
use wfdiag_ui_core::UiEvent;
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

/// Which history maintenance operation an acknowledgement completes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryAckKind {
    Label,
    Tags,
    Clear,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionReviewSurface {
    Review,
    RepairConfirmation,
}

#[derive(Clone)]
pub(crate) enum Message {
    /// One coalesced native-window wake. The UI thread drains every app-owned
    /// event channel in response, replacing the former permanent poll tasks.
    NativeSignalReady,
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
    UpdateStartupDue {
        throttle: Option<UpdateThrottle>,
    },
    UpdateStartupSkipped,
    UpdateDelayCancelled,
    UpdateDelayRejected,
    UpdateCheckFinished(Result<Option<UpdateInfo>, String>),
    UpdateCheckCancelled,
    UpdateCheckRejected,
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
    SettingsRuntimeEvent(Box<SettingsEvent>),
    SettingsWorkerStopped,
    ProviderModelsRefreshDue {
        dialog_epoch: u64,
        refresh_revision: u64,
        setup_index: usize,
    },
    ProviderModelsRefreshCancelled {
        refresh_revision: u64,
    },
    ProviderModelsRefreshRejected {
        refresh_revision: u64,
    },
    RefreshProviderModels,
    CancelProviderModels,
    ProviderSetupWorkerEventReceived(Box<ProviderSetupWorkerEvent>),
    ProviderSetupWorkerStopped,
    RefreshSubscriptionAuth(SubscriptionAuthProvider),
    StartSubscriptionSignIn(SubscriptionAuthProvider),
    StartSubscriptionSignOut(SubscriptionAuthProvider),
    CancelSubscriptionAuth,
    SubscriptionAuthWorkerEventReceived(Box<SubscriptionAuthWorkerEvent>),
    SubscriptionAuthWorkerStopped,
    RequestSubscriptionInstall(SubscriptionAuthProvider),
    SubscriptionInstallPromptClosed {
        prompt: SubscriptionInstallPrompt,
        result: ContentDialogResult,
    },
    CancelSubscriptionInstall,
    SubscriptionInstallWorkerEventReceived(Box<SubscriptionInstallWorkerEvent>),
    SubscriptionInstallWorkerStopped,
    SystemRuntimeCompleted(Box<SystemCompleted>),
    SystemWorkerStopped,
    SystemWaitCancelled,
    SystemWaitRejected,
    IssueRuntimeCompleted(Box<IssueDetectionCompleted>),
    IssueRequestPrepared(Box<PreparedIssueDetection>),
    IssueRequestPreparationCancelled(PendingIssueDetection),
    IssueRequestPreparationRejected(PendingIssueDetection),
    IssueWorkerStopped,
    IssueWaitCancelled,
    IssueWaitRejected,
    RequestQuickScan,
    RequestFullScan,
    CancelScan,
    DiagnosticSessionStarted {
        session_id: String,
        scan_kind: ScanKind,
        task_count: usize,
    },
    DiagnosticSessionStartFailed {
        error: String,
    },
    DiagnosticRunFinished {
        session_id: String,
        cancelled: bool,
        authoritative_results: Result<SharedScanEvidence, String>,
    },
    DiagnosticRunRejected,
    DiagnosticFinalizationElapsed {
        session_id: String,
    },
    DiagnosticFinalizationCancelled {
        session_id: String,
    },
    DiagnosticFinalizationRejected {
        session_id: String,
    },
    DiagnosticHistorySaveFinished {
        session_id: String,
        result: Result<(), String>,
    },
    DiagnosticHistorySaveWaitCancelled {
        session_id: String,
    },
    DiagnosticHistorySaveRejected {
        session_id: String,
    },
    DiagnosticCancelFinished {
        session_id: String,
        error: Option<String>,
    },
    DiagnosticCancelRejected {
        session_id: String,
    },
    DiagnosticBatch {
        events: Vec<UiEvent>,
        terminated: bool,
    },
    AiStatusFinished {
        request_id: u64,
        result: Result<Box<AIProviderStatus>, String>,
    },
    AiStatusCancelled {
        request_id: u64,
    },
    AiStatusRejected {
        request_id: u64,
    },
    ExportRuntimeCompleted(Box<ExportCompleted>),
    /// The rendered report file was written to the validated user path. The
    /// write happens on a background worker; the error is already a string.
    ExportFileSaved {
        request_id: u64,
        result: Box<Result<std::path::PathBuf, String>>,
    },
    SupportPackageSaved {
        request_id: u64,
        result: Box<Result<ValidatedSupportPackagePaths, String>>,
    },
    ExportWorkerStopped,
    ExportWaitCancelled,
    ExportWaitRejected,
    SetAiMode(AiMode),
    ToggleMonitoring,
    Refresh,
    ProcessFilterChanged(String),
    ProcessSort(ProcessSortKey),
    ProcessPrevious,
    ProcessNext,
    ProcessQueryFinished {
        request_id: u64,
        result: Result<ProcessPage, String>,
    },
    ProcessQueryDiscarded {
        request_id: u64,
    },
    ProcessQueryRejected {
        request_id: u64,
    },
    SelectProcess(Option<ProcessIdentity>),
    RefreshHistory,
    HistoryFilterChanged(String),
    SelectHistory(String),
    HistoryListFinished {
        request_id: u64,
        result: Result<Vec<ScanSummary>, String>,
    },
    HistoryCompareFinished {
        request_id: u64,
        result: Result<Box<ComparisonSummary>, String>,
    },
    ToggleHistoryTaskDetail(String),
    HistoryTaskDiffFinished {
        request_id: u64,
        task_id: String,
        result: Result<Box<HistoryTaskDiffProjection>, String>,
    },
    HistoryTaskDiffRejected {
        request_id: u64,
        task_id: String,
    },
    HistoryQueryRejected {
        request_id: u64,
        comparison: bool,
    },
    ChatInputChanged(String),
    UsePrompt(String),
    SendChat,
    /// Chat events arrive at token rate; they are batched per wake so a
    /// streamed answer costs one view rebuild per drain instead of one per
    /// token. Order within the batch is the worker's emission order.
    ChatWorkerEventsBatch(Vec<ChatWorkerEvent>),
    ChatWorkerStopped,
    ReportWorkerEventReceived(Box<ReportWorkerEvent>),
    ReportWorkerStopped,
    ReportGenerationPrepared {
        request_id: u64,
        generation: Box<ReportGeneration>,
    },
    ReportGenerationPreparationCancelled {
        request_id: u64,
    },
    ReportGenerationPreparationRejected {
        request_id: u64,
    },
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
    FixPlanWorkerEventReceived(Box<FixPlanWorkerEvent>),
    FixPlanWorkerStopped,
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
    AnalysisWorkerEventReceived(Box<AnalysisWorkerEvent>),
    AnalysisWorkerStopped,
    NetworkConnectionsFinished(Box<Result<Vec<NetworkConnection>, String>>),
    ToggleClearHistoryConfirm(bool),
    ClearHistoryConfirmed,
    BeginHistoryLabelEdit,
    CancelHistoryLabelEdit,
    HistoryLabelDraftChanged(String),
    SaveHistoryLabel,
    HistoryTagDraftChanged(String),
    SaveHistoryTags,
    HistoryAckFinished {
        kind: HistoryAckKind,
        result: Result<(), String>,
    },
    RequestHistoryTrends,
    HistoryTrendsFinished {
        request_id: u64,
        result: Box<Result<Vec<TaskTrend>, String>>,
    },
    ActionReviewDialogClosed {
        proposal_id: String,
        result: ContentDialogResult,
    },
    RepairDialogClosed {
        proposal_id: String,
        result: ContentDialogResult,
    },
    ActionWorkerEventReceived(Box<ActionWorkerEvent>),
    ActionWorkerStopped,
    InstanceWaitCancelled,
    WindowHookRetryReady,
    WindowHookRetryRejected,
    ActionRunEventReceived(Box<ActionRunEvent>),
    ActionRunStreamStopped,
    CancelActionRun,
    ActionRunExpandedChanged {
        run_id: String,
        expanded: bool,
    },
    RestartAsAdmin,
    RestartAsAdminFinished(Result<bool, String>),
    InstanceActivated,
    WindowLifecycleChanged(window::WindowLifecycleSnapshot),
    GlobalShortcut(window::GlobalShortcutEvent),
    TrayCommand(u8),
    BackendBatch {
        events: Vec<UiEvent>,
        terminated: bool,
    },
}
