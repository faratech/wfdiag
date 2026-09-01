//! The value types the root component stores as fields.
//!
//! Everything here is **view state**: what is on screen, what the user is
//! editing, and which overlay is open. Engine state — results, issues, history,
//! provider status, staged remediations — lives in
//! [`wfdiag_app::AppSnapshot`] and is projected into these types after every
//! drain. Nothing in this module holds a request id or a worker handle.

#![deny(unsafe_code)]

use crate::platform::save_picker::{ValidatedExportPath, ValidatedSupportPackagePaths};
use crate::widgets::icons::FaIcon;
use std::collections::VecDeque;
use wfdiag_app::domain::ai_intent::PendingAiIntent;
use wfdiag_native_ai_analysis::GroundingTrace;
use wfdiag_native_ai_chat::{ChatToolHistory, ProviderUse};
use wfdiag_native_ai_provider::AIProvider;
use wfdiag_native_history::TaskDiffDetail;
use wfdiag_native_projection::json_diff::{JsonDifference, find_json_differences};
use wfdiag_native_remediation::broker::ActionRequest;
use wfdiag_ui_core::SystemStats;

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AiMode {
    #[default]
    Assistant,
    ScanReport,
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
        self.intent.is_some_and(PendingAiIntent::is_chat)
    }

    pub(crate) fn is_report(self) -> bool {
        self.intent.is_some_and(PendingAiIntent::is_report)
    }
}

/// The Full Scan the assistant asked for, awaiting the user's answer.
///
/// This consent surface stays with the shell: the engine only reports that the
/// model asked ([`wfdiag_app::ChatEvent::FullScanRequested`]) and never starts
/// a scan on its own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FullScanConsent {
    pub(crate) source_scan_id: String,
    pub(crate) reason: String,
    pub(crate) original_prompt: String,
}

/// The local-to-cloud question, projected for rendering.
///
/// The decision itself is [`wfdiag_app::AppCommand::CloudFallbackDecision`];
/// this is only what the panel prints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CloudFallbackConsent {
    pub(crate) candidate: AIProvider,
    pub(crate) local_provider: AIProvider,
    pub(crate) reason: String,
    pub(crate) saving: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChatDisplayRole {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub(crate) struct ChatDisplayMessage {
    pub(crate) turn: u64,
    pub(crate) role: ChatDisplayRole,
    pub(crate) text: String,
    pub(crate) provider_use: Option<ProviderUse>,
    pub(crate) finish_reason: Option<String>,
    pub(crate) terminal_message: Option<String>,
    pub(crate) tools: ChatToolHistory,
    pub(crate) proposals: Vec<String>,
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

#[derive(Clone, Debug, Default)]
pub(crate) struct IssuePrioritizationDisplay {
    pub(crate) text: Option<String>,
    pub(crate) provider_use: Option<ProviderUse>,
    pub(crate) cached: bool,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
}

/// The catalog ids one fix-plan row (or its "review together" batch) names,
/// pinned to the fingerprints the plan was generated against.
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

    /// Whether this page consumes one-second telemetry.
    pub(crate) const fn consumes_live_telemetry(self) -> bool {
        matches!(self, Self::Monitor | Self::Processes)
    }
}

/// Reactor presentation state for one expanded History task.
///
/// The native history contract remains the source of the raw payloads. The
/// structured JSON projection is computed once, when the engine's task-diff
/// event arrives, so rebuilding the WinUI tree never reparses large output.
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

impl From<TaskDiffDetail> for HistoryTaskDiffProjection {
    fn from(detail: TaskDiffDetail) -> Self {
        let differences = find_json_differences(&detail.previous_output, &detail.current_output);
        Self {
            detail,
            differences,
        }
    }
}

/// What the shell must do with a rendered export payload once it arrives.
///
/// The destination was chosen before the render was requested (the picker
/// answers first, #140), so this is a pure "where does the payload go".
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

    #[cfg(feature = "validation")]
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

    #[test]
    fn only_the_live_surfaces_consume_telemetry() {
        for page in Page::ALL {
            assert_eq!(
                page.consumes_live_telemetry(),
                matches!(page, Page::Monitor | Page::Processes),
                "{page:?}"
            );
        }
    }
}
