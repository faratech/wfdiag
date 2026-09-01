//! The value types the root component stores as fields.

#![deny(unsafe_code)]

use crate::ai::chat_tools::ChatToolSnapshot;
use crate::app::message::ActionReviewSurface;
use crate::app::policy::{
    merge_targeted_diagnostic_result, scan_concurrency_from_settings, scan_kind_history_tag,
};
use crate::platform::save_picker::{ValidatedExportPath, ValidatedSupportPackagePaths};
use crate::widgets::icons::FaIcon;
use std::collections::VecDeque;
use wfdiag_native_ai_analysis::{
    DiagnosticAnalysisGeneration, FixPlanGeneration, GroundingTrace, IssuePrioritizationGeneration,
};
use wfdiag_native_ai_chat::workers::provider_setup::ModelCatalog;
use wfdiag_native_ai_chat::workers::subscription_install::{
    SubscriptionInstallFallbackReason, SubscriptionInstallMethod,
};
use wfdiag_native_ai_chat::{
    ChatToolHistory, ProviderUse, SubscriptionAuthOperation, SubscriptionAuthProvider,
    SubscriptionAuthStatus,
};
use wfdiag_native_ai_provider::{AIProvider, AIProviderPreference, ProviderAvailability};
use wfdiag_native_diagnostics::{DiagnosticTask, ScanKind};
use wfdiag_native_history::TaskDiffDetail;
use wfdiag_native_projection::json_diff::{JsonDifference, find_json_differences};
use wfdiag_native_remediation::broker::{ActionProposal, ActionRequest};
use wfdiag_native_settings::{AppSettings, CloudFallbackPolicy, ProviderKeyId};
use wfdiag_ui_core::{DiagnosticTaskResult, SystemStats};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Page {
    Diagnostics,
    Monitor,
    Processes,
    Ai,
    Issues,
    History,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageTransition {
    pub(crate) previous: Page,
    pub(crate) next: Page,
}

impl PageTransition {
    pub(crate) fn between(previous: Page, next: Page) -> Option<Self> {
        (previous != next).then_some(Self { previous, next })
    }

    pub(crate) fn leaves_processes(self) -> bool {
        self.previous == Page::Processes && self.next != Page::Processes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AiMode {
    Assistant,
    ScanReport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PendingAiIntent {
    Report { force_refresh: bool },
    Chat { prompt: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingAiProviderGate {
    Ready,
    Waiting,
    Refresh,
    Disabled,
    Unavailable,
}

#[derive(Clone, Copy)]
pub(crate) struct AiPreparationUi<'a> {
    pub(crate) intent: Option<&'a PendingAiIntent>,
    pub(crate) error: Option<&'a str>,
    pub(crate) scan_busy: bool,
    pub(crate) scan_cancelling: bool,
    pub(crate) completed: usize,
    pub(crate) total: usize,
    pub(crate) current_task: Option<&'a str>,
}

impl AiPreparationUi<'_> {
    pub(crate) fn is_chat(self) -> bool {
        matches!(self.intent, Some(PendingAiIntent::Chat { .. }))
    }

    pub(crate) fn is_report(self) -> bool {
        matches!(self.intent, Some(PendingAiIntent::Report { .. }))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FullScanConsent {
    pub(crate) source_scan_id: String,
    pub(crate) reason: String,
    pub(crate) original_prompt: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChatDisplayRole {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub(crate) struct ChatDisplayMessage {
    pub(crate) request_id: u64,
    pub(crate) role: ChatDisplayRole,
    pub(crate) text: String,
    pub(crate) provider_use: Option<ProviderUse>,
    pub(crate) finish_reason: Option<String>,
    pub(crate) terminal_message: Option<String>,
    pub(crate) tools: ChatToolHistory,
    pub(crate) proposals: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ChatAttempt {
    pub(crate) logical_request_id: u64,
    pub(crate) prompt: String,
    pub(crate) tools: ChatToolSnapshot,
    pub(crate) preference: AIProviderPreference,
    pub(crate) availability: ProviderAvailability,
    pub(crate) tried: Vec<AIProvider>,
    pub(crate) initial_provider: AIProvider,
    pub(crate) current_provider: AIProvider,
    pub(crate) first_failure: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct CloudFallbackConsent {
    pub(crate) previous_request_id: u64,
    pub(crate) attempt: ChatAttempt,
    pub(crate) candidate: AIProvider,
    pub(crate) reason: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingCloudFallbackPolicyUpdate {
    pub(crate) request_id: u64,
    pub(crate) policy: CloudFallbackPolicy,
    pub(crate) consent: CloudFallbackConsent,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DiagnosticAnalysisDisplay {
    pub(crate) interpretation: Option<String>,
    pub(crate) provider_use: Option<ProviderUse>,
    pub(crate) grounding: Option<GroundingTrace>,
    pub(crate) cached: bool,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct DiagnosticAnalysisAttempt {
    pub(crate) generation: DiagnosticAnalysisGeneration,
    pub(crate) tried: Vec<AIProvider>,
    pub(crate) initial_provider: AIProvider,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingDiagnosticAnalysis {
    pub(crate) request_id: u64,
    pub(crate) attempt: DiagnosticAnalysisAttempt,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct IssuePrioritizationDisplay {
    pub(crate) text: Option<String>,
    pub(crate) provider_use: Option<ProviderUse>,
    pub(crate) grounding: Option<GroundingTrace>,
    pub(crate) cached: bool,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
    /// Epoch of the committed issue evidence this text describes. A result
    /// from an older detection pass is never rendered over newer evidence.
    pub(crate) committed_epoch: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct IssuePrioritizationAttempt {
    pub(crate) generation: IssuePrioritizationGeneration,
    pub(crate) tried: Vec<AIProvider>,
    pub(crate) initial_provider: AIProvider,
    pub(crate) committed_epoch: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingIssuePrioritization {
    pub(crate) request_id: u64,
    pub(crate) attempt: IssuePrioritizationAttempt,
}

#[derive(Clone, Debug)]
pub(crate) struct FixPlanAttempt {
    pub(crate) generation: FixPlanGeneration,
    pub(crate) tried: Vec<AIProvider>,
    pub(crate) initial_provider: AIProvider,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingFixPlan {
    pub(crate) request_id: u64,
    pub(crate) attempt: FixPlanAttempt,
}

#[derive(Clone, Debug)]
pub(crate) struct FixPlanActionSelection {
    pub(crate) actions: Vec<ActionRequest>,
    pub(crate) expected_scan_fingerprint: String,
    pub(crate) expected_catalog_fingerprint: String,
}

impl Page {
    pub(crate) const ALL: [Self; 6] = [
        Self::Diagnostics,
        Self::Monitor,
        Self::Processes,
        Self::Ai,
        Self::Issues,
        Self::History,
    ];

    pub(crate) fn tag(self) -> &'static str {
        match self {
            Self::Diagnostics => "diagnostics",
            Self::Monitor => "monitor",
            Self::Processes => "processes",
            Self::Ai => "ai",
            Self::Issues => "issues",
            Self::History => "history",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Diagnostics => "System Analysis",
            Self::Monitor => "Live Monitor",
            Self::Processes => "Processes",
            Self::Ai => "AI Analysis",
            Self::Issues => "Issues",
            Self::History => "History",
        }
    }

    pub(crate) fn nav_label(self) -> &'static str {
        match self {
            Self::Diagnostics => "Diagnostics",
            Self::Monitor => "Live Monitor",
            Self::Processes => "Processes",
            Self::Ai => "AI Analysis",
            Self::Issues => "Issues",
            Self::History => "History",
        }
    }

    pub(crate) fn subtitle(self) -> &'static str {
        match self {
            Self::Diagnostics => "Read-only diagnostics across hardware, storage, network and logs",
            Self::Monitor => "Real-time CPU, memory, disk, network and NPU telemetry",
            Self::Processes => "Running processes with live resource usage",
            Self::Ai => "Ask about this PC or turn the latest scan into a focused health report",
            Self::Issues => "Problems detected in the latest scan, with one-click fixes",
            Self::History => "Past scans — spot drift and regressions over time",
        }
    }

    pub(crate) fn icon(self) -> FaIcon {
        match self {
            Self::Diagnostics => FaIcon::Diagnostics,
            Self::Monitor => FaIcon::Monitor,
            Self::Processes => FaIcon::Processes,
            Self::Ai => FaIcon::Ai,
            Self::Issues => FaIcon::Issues,
            Self::History => FaIcon::History,
        }
    }

    pub(crate) fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|page| page.tag() == tag)
    }
}

/// Reactor presentation state for one expanded History task.
///
/// The native history contract remains the source of the raw payloads. The
/// structured JSON projection is computed once on the background completion
/// path so rebuilding the WinUI tree never reparses large diagnostic output.
#[derive(Clone, Debug)]
pub(crate) struct HistoryTaskDiffProjection {
    pub(crate) detail: TaskDiffDetail,
    pub(crate) differences: Option<Vec<JsonDifference>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HistoryTrendBadge {
    pub(crate) label: String,
    pub(crate) description: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingActionApproval {
    pub(crate) proposal: ActionProposal,
    pub(crate) return_surface: ActionReviewSurface,
}

impl From<TaskDiffDetail> for HistoryTaskDiffProjection {
    fn from(detail: TaskDiffDetail) -> Self {
        let differences = find_json_differences(&detail.previous_output, &detail.current_output);
        Self {
            detail,
            differences,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticSnapshot {
    pub(crate) results: Vec<DiagnosticTaskResult>,
    pub(crate) scan_kind: Option<ScanKind>,
    pub(crate) task_ids: Vec<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) duration_ms: u64,
    pub(crate) total: usize,
    pub(crate) completed: usize,
    pub(crate) errors: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TargetedDiagnosticOverlay {
    pub(crate) target_task_id: String,
    pub(crate) base: DiagnosticSnapshot,
    pub(crate) staged_result: Option<DiagnosticTaskResult>,
}

impl TargetedDiagnosticOverlay {
    pub(crate) fn for_committed_session(
        scan_kind: ScanKind,
        task_ids: &[String],
        base: DiagnosticSnapshot,
    ) -> Option<Self> {
        let [target_task_id] = task_ids else {
            return None;
        };
        (scan_kind == ScanKind::Targeted && base.session_id.is_some()).then(|| Self {
            target_task_id: target_task_id.clone(),
            base,
            staged_result: None,
        })
    }

    pub(crate) fn stage(&mut self, result: DiagnosticTaskResult) {
        if result.task_id == self.target_task_id {
            self.staged_result = Some(result);
        }
    }

    pub(crate) fn staged_counts(&self) -> (usize, usize) {
        self.staged_result
            .as_ref()
            .map_or((0, 0), |result| (1, usize::from(!result.success)))
    }

    pub(crate) fn rollback(self) -> DiagnosticSnapshot {
        self.base
    }

    pub(crate) fn commit(
        &self,
        replacement: DiagnosticTaskResult,
        catalog: &[DiagnosticTask],
    ) -> Result<DiagnosticSnapshot, String> {
        let results = merge_targeted_diagnostic_result(
            self.base.results.clone(),
            &self.target_task_id,
            replacement,
            self.base.session_id.as_deref(),
            catalog,
        )?;
        let mut task_ids = self.base.task_ids.clone();
        if !task_ids
            .iter()
            .any(|task_id| task_id == &self.target_task_id)
        {
            task_ids.push(self.target_task_id.clone());
        }
        let completed = results.len();
        let errors = results.iter().filter(|result| !result.success).count();
        Ok(DiagnosticSnapshot {
            results,
            scan_kind: self.base.scan_kind,
            total: task_ids.len(),
            task_ids,
            session_id: self.base.session_id.clone(),
            duration_ms: self.base.duration_ms,
            completed,
            errors,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PendingExportAction {
    ShareToWindowsForum,
    EmailReport,
    CopyDiagnosticReport,
    SupportPackage {
        paths: ValidatedSupportPackagePaths,
    },
    /// Write the rendered report to the user-chosen, policy-validated path.
    SaveToFile {
        path: ValidatedExportPath,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingExport {
    pub(crate) request_id: u64,
    pub(crate) action: PendingExportAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExportWriteKind {
    File,
    SupportPackage,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProviderCatalogUiState {
    pub(crate) catalog: Option<ModelCatalog>,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) blocked: Option<String>,
    pub(crate) stale: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingProviderCatalogRequest {
    pub(crate) request_id: u64,
    pub(crate) provider: AIProvider,
    pub(crate) setup_index: usize,
    pub(crate) dialog_epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingProviderKeyChange {
    pub(crate) request_id: u64,
    pub(crate) provider: ProviderKeyId,
    pub(crate) index: usize,
    pub(crate) store: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SubscriptionAuthUiState {
    pub(crate) status: Option<SubscriptionAuthStatus>,
    pub(crate) operation: Option<SubscriptionAuthOperation>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingSubscriptionAuth {
    pub(crate) operation_id: u64,
    pub(crate) provider: SubscriptionAuthProvider,
    pub(crate) operation: SubscriptionAuthOperation,
    pub(crate) dialog_epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubscriptionInstallPrompt {
    Winget {
        provider: SubscriptionAuthProvider,
        dialog_epoch: u64,
    },
    VendorFallback {
        provider: SubscriptionAuthProvider,
        reason: SubscriptionInstallFallbackReason,
        dialog_epoch: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingSubscriptionInstall {
    pub(crate) request_id: u64,
    pub(crate) provider: SubscriptionAuthProvider,
    pub(crate) method: SubscriptionInstallMethod,
    pub(crate) dialog_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticScanPolicy {
    pub(crate) auto_save: bool,
    pub(crate) max_concurrent_tasks: usize,
    pub(crate) history_tag: String,
}

impl DiagnosticScanPolicy {
    pub(crate) fn snapshot(
        settings: &AppSettings,
        scan_kind: ScanKind,
        targeted_rerun: bool,
    ) -> Self {
        Self {
            // A one-row rerun updates the currently committed scan in place.
            // Persisting it as a separate one-row history entry would both lose
            // the scan context and diverge from the shipping Tauri behavior.
            auto_save: settings.auto_save && !targeted_rerun,
            max_concurrent_tasks: scan_concurrency_from_settings(settings.max_concurrent_tasks),
            history_tag: scan_kind_history_tag(scan_kind).to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HistoryRetentionPolicy {
    pub(crate) retain_history: bool,
    pub(crate) history_limit: u32,
}

impl From<&AppSettings> for HistoryRetentionPolicy {
    fn from(settings: &AppSettings) -> Self {
        Self {
            retain_history: settings.retain_history,
            history_limit: settings.history_limit,
        }
    }
}

pub(crate) struct PendingSettingsSave {
    pub(crate) request_id: u64,
    pub(crate) dialog_epoch: u64,
    pub(crate) submitted: AppSettings,
}

pub(crate) const MONITOR_HISTORY_SAMPLES: usize = 60;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MonitorSample {
    pub(crate) cpu: f64,
    pub(crate) memory: f64,
    pub(crate) storage: f64,
    pub(crate) network_mb: f64,
    pub(crate) gpu: f64,
    pub(crate) npu: f64,
}

impl MonitorSample {
    pub(crate) fn from_stats(stats: &SystemStats) -> Self {
        Self {
            cpu: f64::from(stats.cpu_utilization),
            memory: f64::from(stats.memory_utilization),
            storage: f64::from(stats.storage_used_percent),
            network_mb: (stats.network_upload_kb + stats.network_download_kb) / 1024.0,
            gpu: f64::from(stats.gpu_utilization.unwrap_or_default()),
            npu: f64::from(stats.npu_utilization.unwrap_or_default()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MonitorMetric {
    Cpu,
    Memory,
    Storage,
    Network,
    Gpu,
    Npu,
}

#[derive(Debug, Default)]
pub(crate) struct MonitorHistory {
    pub(crate) samples: VecDeque<MonitorSample>,
}

impl MonitorHistory {
    pub(crate) fn push_stats(&mut self, stats: &SystemStats) {
        self.samples.push_back(MonitorSample::from_stats(stats));
        while self.samples.len() > MONITOR_HISTORY_SAMPLES {
            self.samples.pop_front();
        }
    }

    pub(crate) fn series(&self, metric: MonitorMetric) -> Vec<f64> {
        self.samples
            .iter()
            .map(|sample| match metric {
                MonitorMetric::Cpu => sample.cpu,
                MonitorMetric::Memory => sample.memory,
                MonitorMetric::Storage => sample.storage,
                MonitorMetric::Network => sample.network_mb,
                MonitorMetric::Gpu => sample.gpu,
                MonitorMetric::Npu => sample.npu,
            })
            .collect()
    }

    pub(crate) fn fixture_258() -> Self {
        let cpu = [10.3, 63.0, 20.0, 17.0, 10.3];
        let memory = [81.0; 5];
        let storage = [64.3; 5];
        let network = [0.60, 1.15, 0.55, 2.00, 0.81];
        let gpu = [21.0, 27.0, 23.0, 23.0, 23.9];
        let npu = [0.0; 5];
        let samples = (0..5)
            .map(|index| MonitorSample {
                cpu: cpu[index],
                memory: memory[index],
                storage: storage[index],
                network_mb: network[index],
                gpu: gpu[index],
                npu: npu[index],
            })
            .collect();
        Self { samples }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_transition_classifies_every_process_exit_and_ignores_noops() {
        for previous in Page::ALL {
            for next in Page::ALL {
                let transition = PageTransition::between(previous, next);
                if previous == next {
                    assert_eq!(transition, None);
                    continue;
                }
                let transition = transition.expect("different pages produce a transition");
                assert_eq!(transition.previous, previous);
                assert_eq!(transition.next, next);
                assert_eq!(
                    transition.leaves_processes(),
                    previous == Page::Processes && next != Page::Processes,
                    "{previous:?} -> {next:?}"
                );
            }
        }
    }
}
