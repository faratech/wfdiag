//! Deterministic visual fixtures for QA and validation runs.

#![deny(unsafe_code)]

use crate::app::state::Page;
use crate::screens::processes::view::ProcessViewRow;
use wfdiag_native_projection::process_identity::ProcessIdentity;
use wfdiag_native_remediation::broker::{ActionRequest, current_action_catalog_fingerprint};
use wfdiag_native_remediation::remediation;
use wfdiag_native_remediation::runtime::{
    ActionItemRun, ActionItemStatus, ActionRunStatus, ActionRunSummary,
};
use wfdiag_native_system::SystemInfo;
use wfdiag_ui_core::SystemStats;

/// Deterministic Store 2.5.8 visual states used only by screenshot/QA automation.
/// The normal application path remains `Live` unless the environment variable is set.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum VisualState {
    #[default]
    Live,
    MonitorEmpty,
    ProcessesEmpty,
    HistoryEmpty,
    AiEmptyCompact,
    IssueToChat,
    AiConversationDesktop,
    AiConversationTopCompact,
    AiConversationBottomCompact,
    SettingsBottom,
    RemediationPartial,
}

impl VisualState {
    /// Pure parser for the visual-state knob (#186).
    ///
    /// The environment read that feeds this lives in `fixtures::knobs` and is
    /// compiled out without the `validation` feature; a shipping build calls
    /// this with `""` so the production default stays on one code path.
    pub(crate) fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().replace('_', "-").as_str() {
            "monitor-empty-desktop-dark" | "monitor-empty" => Self::MonitorEmpty,
            "processes-empty-desktop-dark" | "processes-empty" => Self::ProcessesEmpty,
            "history-empty-desktop-dark" | "history-empty" => Self::HistoryEmpty,
            "ai-empty-compact-dark" | "ai-empty-compact" => Self::AiEmptyCompact,
            "issue-to-chat-desktop-dark" | "issue-to-chat" => Self::IssueToChat,
            "ai-conversation-desktop-dark" | "ai-conversation-desktop" => {
                Self::AiConversationDesktop
            }
            "ai-conversation-top-compact-dark" | "ai-conversation-top-compact" => {
                Self::AiConversationTopCompact
            }
            "ai-conversation-bottom-compact-dark" | "ai-conversation-bottom-compact" => {
                Self::AiConversationBottomCompact
            }
            "settings-bottom-desktop-dark" | "settings-bottom" => Self::SettingsBottom,
            "remediation-partial-desktop-dark" | "remediation-partial" => Self::RemediationPartial,
            _ => Self::Live,
        }
    }

    pub(crate) fn default_page(self) -> Page {
        match self {
            Self::MonitorEmpty | Self::SettingsBottom => Page::Monitor,
            Self::ProcessesEmpty => Page::Processes,
            Self::HistoryEmpty => Page::History,
            Self::RemediationPartial => Page::Issues,
            Self::AiEmptyCompact
            | Self::IssueToChat
            | Self::AiConversationDesktop
            | Self::AiConversationTopCompact
            | Self::AiConversationBottomCompact => Page::Ai,
            Self::Live => Page::Diagnostics,
        }
    }

    pub(crate) fn default_size(self) -> (f64, f64) {
        match self {
            Self::MonitorEmpty | Self::ProcessesEmpty | Self::HistoryEmpty => (1440.0, 1000.0),
            Self::AiEmptyCompact
            | Self::AiConversationTopCompact
            | Self::AiConversationBottomCompact => (900.0, 800.0),
            Self::IssueToChat
            | Self::AiConversationDesktop
            | Self::SettingsBottom
            | Self::RemediationPartial => (1440.0, 900.0),
            Self::Live => (1200.0, 800.0),
        }
    }

    pub(crate) fn has_scan(self) -> bool {
        matches!(
            self,
            Self::IssueToChat
                | Self::AiConversationDesktop
                | Self::AiConversationTopCompact
                | Self::AiConversationBottomCompact
        )
    }

    pub(crate) fn is_conversation(self) -> bool {
        matches!(
            self,
            Self::IssueToChat
                | Self::AiConversationDesktop
                | Self::AiConversationTopCompact
                | Self::AiConversationBottomCompact
                | Self::RemediationPartial
        )
    }
}

/// Closed, validation-build-only live fixtures. Unlike screenshot fixtures,
/// these exercise a deliberately tiny real native path. Production builds do
/// not read `LIVE_TEST_FIXTURE_ENV` (the knob is not even compiled), and the
/// action fixture permits only the one non-mutating catalog item named below.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveTestFixture {
    DeviceManager,
    ExportFallback,
    AdminRelaunch,
}

impl LiveTestFixture {
    /// Pure parser for the live-fixture knob (#186): the environment read
    /// that feeds it lives in `fixtures::knobs` behind the `validation`
    /// feature, and a shipping build calls this with `""` (always `None`).
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "device-manager" => Some(Self::DeviceManager),
            "export-fallback" => Some(Self::ExportFallback),
            "admin-relaunch" => Some(Self::AdminRelaunch),
            _ => None,
        }
    }

    pub(crate) const fn injects_scan(self) -> bool {
        true
    }

    pub(crate) fn permits_actions(self, actions: &[ActionRequest]) -> bool {
        self == Self::DeviceManager
            && !actions.is_empty()
            && actions
                .iter()
                .all(|action| action.remediation_id == "open_device_manager")
    }
}

pub(crate) fn remediation_partial_visual_run() -> ActionRunSummary {
    use remediation::{
        FixCompletionStatus, FixResult, RemediationStepResult, RemediationStepStatus,
    };

    let steps = vec![
        RemediationStepResult {
            action: "Reset Winsock catalog".to_string(),
            status: RemediationStepStatus::Succeeded,
            detail: Some("The Winsock catalog was reset.".to_string()),
        },
        RemediationStepResult {
            action: "Reset TCP/IP stack".to_string(),
            status: RemediationStepStatus::Failed,
            detail: Some("Access was denied while applying one step.".to_string()),
        },
    ];
    ActionRunSummary {
        run_id: "visual-partial-run".to_string(),
        proposal_id: "visual-partial-proposal".to_string(),
        authorization_id: "visual-partial-authorization".to_string(),
        status: ActionRunStatus::Partial,
        actions: vec![ActionItemRun {
            remediation_id: "network_reset".to_string(),
            label: "Reset network stack".to_string(),
            cancellable: true,
            status: ActionItemStatus::Partial,
            result: Some(FixResult {
                success: false,
                message: "The remediation completed with partial results.".to_string(),
                actions_taken: steps.iter().map(|step| step.action.clone()).collect(),
                requires_restart: false,
                completion_status: FixCompletionStatus::Partial,
                steps,
            }),
            error: None,
        }],
        current_index: None,
        approved_at_ms: 1_780_000_000_000,
        completed_at_ms: Some(1_780_000_001_000),
        scan_fingerprint: "visual-partial-scan".to_string(),
        catalog_fingerprint: current_action_catalog_fingerprint(),
    }
}

pub(crate) fn fixture_258_system_info() -> SystemInfo {
    SystemInfo {
        computer_name: "ANDROMEDA".to_string(),
        os_version: "Windows 11 Professional (25H2)".to_string(),
        is_admin: false,
    }
}

pub(crate) fn fixture_system_stats() -> SystemStats {
    SystemStats {
        cpu_utilization: 10.3,
        per_cpu_utilization: vec![10.3; 12],
        cpu_frequency: 2_980,
        memory_total_gb: 63.5,
        memory_used_gb: 51.4,
        memory_available_gb: 12.1,
        memory_utilization: 81.0,
        swap_total_gb: 8.0,
        swap_used_gb: 1.0,
        swap_utilization: 12.5,
        storage_used_percent: 64.3,
        disk_utilization: 64.3,
        disk_read_bytes: 0,
        disk_write_bytes: 0,
        disks: Vec::new(),
        network_upload_kb: 150.0,
        network_download_kb: 679.44,
        gpu_available: true,
        gpu_name: Some("Adreno 741".to_string()),
        gpu_utilization: Some(23.9),
        gpu_memory_used_mb: 2.17 * 1024.0,
        gpu_memory_total_mb: 8.12 * 1024.0,
        npu_available: true,
        npu_name: Some(
            "Snapdragon(R) X Elite - X1E80100 - Qualcomm(R) Hexagon(TM) NPU".to_string(),
        ),
        npu_utilization: Some(0.0),
        npu_memory_used_mb: 0.0,
        npu_memory_total_mb: 0.0,
        top_processes: Vec::new(),
        timestamp: 0,
    }
}

pub(crate) fn fixture_monitor_empty_stats() -> SystemStats {
    SystemStats {
        cpu_utilization: 71.2,
        per_cpu_utilization: vec![71.2; 12],
        cpu_frequency: 2_980,
        memory_total_gb: 63.5,
        memory_used_gb: 51.8,
        memory_available_gb: 11.7,
        memory_utilization: 81.6,
        swap_total_gb: 8.0,
        swap_used_gb: 1.0,
        swap_utilization: 12.5,
        storage_used_percent: 0.0,
        disk_utilization: 0.0,
        disk_read_bytes: 0,
        disk_write_bytes: 0,
        disks: Vec::new(),
        network_upload_kb: 0.0,
        network_download_kb: 0.0,
        gpu_available: true,
        gpu_name: Some("Adreno 741".to_string()),
        gpu_utilization: Some(0.0),
        gpu_memory_used_mb: 2.21 * 1024.0,
        gpu_memory_total_mb: 8.12 * 1024.0,
        npu_available: true,
        npu_name: Some(
            "Snapdragon(R) X Elite - X1E80100 - Qualcomm(R) Hexagon(TM) NPU".to_string(),
        ),
        npu_utilization: Some(0.0),
        npu_memory_used_mb: 0.0,
        npu_memory_total_mb: 0.0,
        top_processes: Vec::new(),
        timestamp: 0,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ProcessFixture258 {
    pub(crate) name: &'static str,
    pub(crate) pid: u32,
    pub(crate) cpu: f64,
    pub(crate) memory: &'static str,
    pub(crate) memory_percent: f64,
    pub(crate) status: &'static str,
    pub(crate) threads: u32,
}

pub(crate) const PROCESS_ROWS_258: [ProcessFixture258; 19] = [
    ProcessFixture258 {
        name: "firefox.exe",
        pid: 36840,
        cpu: 2.3,
        memory: "127.9 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 40,
    },
    ProcessFixture258 {
        name: "vmmemWSL",
        pid: 13884,
        cpu: 2.3,
        memory: "14.81 GB",
        memory_percent: 23.3,
        status: "Running",
        threads: 104,
    },
    ProcessFixture258 {
        name: "WorkloadsSessionHost.exe",
        pid: 21428,
        cpu: 2.2,
        memory: "48.3 MB",
        memory_percent: 0.1,
        status: "Running",
        threads: 8,
    },
    ProcessFixture258 {
        name: "System",
        pid: 4,
        cpu: 1.2,
        memory: "10.6 MB",
        memory_percent: 0.0,
        status: "Running",
        threads: 597,
    },
    ProcessFixture258 {
        name: "Taskmgr.exe",
        pid: 18736,
        cpu: 0.9,
        memory: "208.1 MB",
        memory_percent: 0.3,
        status: "Running",
        threads: 24,
    },
    ProcessFixture258 {
        name: "dwm.exe",
        pid: 2692,
        cpu: 0.9,
        memory: "194.4 MB",
        memory_percent: 0.3,
        status: "Running",
        threads: 18,
    },
    ProcessFixture258 {
        name: "msedgewebview2.exe",
        pid: 30728,
        cpu: 0.6,
        memory: "131.6 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 18,
    },
    ProcessFixture258 {
        name: "firefox.exe",
        pid: 10892,
        cpu: 0.6,
        memory: "26.1 MB",
        memory_percent: 0.0,
        status: "Running",
        threads: 11,
    },
    ProcessFixture258 {
        name: "Paltalk.exe",
        pid: 24468,
        cpu: 0.6,
        memory: "345.9 MB",
        memory_percent: 0.5,
        status: "Running",
        threads: 89,
    },
    ProcessFixture258 {
        name: "firefox.exe",
        pid: 33984,
        cpu: 0.5,
        memory: "310.8 MB",
        memory_percent: 0.5,
        status: "Running",
        threads: 53,
    },
    ProcessFixture258 {
        name: "WindowsTerminal.exe",
        pid: 25396,
        cpu: 0.4,
        memory: "175.4 MB",
        memory_percent: 0.3,
        status: "Running",
        threads: 21,
    },
    ProcessFixture258 {
        name: "WorkloadsSessionHost.exe",
        pid: 22104,
        cpu: 0.4,
        memory: "150.8 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 25,
    },
    ProcessFixture258 {
        name: "audiodg.exe",
        pid: 2832,
        cpu: 0.4,
        memory: "32.3 MB",
        memory_percent: 0.0,
        status: "Running",
        threads: 35,
    },
    ProcessFixture258 {
        name: "msedgewebview2.exe",
        pid: 39200,
        cpu: 0.3,
        memory: "326.1 MB",
        memory_percent: 0.5,
        status: "Running",
        threads: 17,
    },
    ProcessFixture258 {
        name: "svchost.exe",
        pid: 9852,
        cpu: 0.3,
        memory: "104.6 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 9,
    },
    ProcessFixture258 {
        name: "MsMpEng.exe",
        pid: 5772,
        cpu: 0.3,
        memory: "513.6 MB",
        memory_percent: 0.8,
        status: "Running",
        threads: 62,
    },
    ProcessFixture258 {
        name: "Discord.exe",
        pid: 21100,
        cpu: 0.2,
        memory: "736.4 MB",
        memory_percent: 1.1,
        status: "Running",
        threads: 592,
    },
    ProcessFixture258 {
        name: "SystemSettings.exe",
        pid: 19340,
        cpu: 0.2,
        memory: "96.8 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 21,
    },
    ProcessFixture258 {
        name: "PhoneExperienceHost.exe",
        pid: 17364,
        cpu: 0.2,
        memory: "142.7 MB",
        memory_percent: 0.2,
        status: "Running",
        threads: 32,
    },
];

impl ProcessFixture258 {
    pub(crate) fn virtual_memory(self) -> &'static str {
        match self.pid {
            13884 => "18.42 GB",
            21100 => "2.31 GB",
            5772 => "1.27 GB",
            33984 | 36840 => "1.14 GB",
            _ => "684.0 MB",
        }
    }

    pub(crate) fn handles(self) -> u32 {
        self.threads.saturating_mul(14).saturating_add(173)
    }

    pub(crate) fn cpu_time_secs(self) -> u64 {
        (self.pid as u64 % 1_200).saturating_add(self.threads as u64 * 3)
    }

    pub(crate) fn read(self) -> &'static str {
        match self.pid {
            4 => "6.81 GB",
            13884 => "2.07 GB",
            5772 => "1.42 GB",
            _ => "148.3 MB",
        }
    }

    pub(crate) fn written(self) -> &'static str {
        match self.pid {
            4 => "1.72 GB",
            13884 => "614.6 MB",
            21100 => "284.2 MB",
            _ => "42.1 MB",
        }
    }
}

impl From<ProcessFixture258> for ProcessViewRow {
    fn from(process: ProcessFixture258) -> Self {
        Self {
            name: process.name.to_string(),
            pid: process.pid,
            start_time: ProcessIdentity::UNKNOWN_START_TIME,
            cpu: process.cpu,
            memory: process.memory.to_string(),
            memory_percent: process.memory_percent,
            virtual_memory: process.virtual_memory().to_string(),
            status: process.status.to_string(),
            threads: process.threads,
            handles: process.handles(),
            cpu_time_secs: process.cpu_time_secs(),
            read: process.read().to_string(),
            written: process.written().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::policy::{
        action_run_status_text, machine_card_accessibility_name, privilege_label,
    };

    #[test]
    fn remediation_partial_fixture_is_terminal_and_keeps_failed_step_detail() {
        let run = remediation_partial_visual_run();
        assert_eq!(run.status, ActionRunStatus::Partial);
        assert!(run.status.terminal());
        assert_eq!(run.actions.len(), 1);
        assert_eq!(run.actions[0].status, ActionItemStatus::Partial);
        let result = run.actions[0].result.as_ref().unwrap();
        assert_eq!(
            result.completion_status,
            remediation::FixCompletionStatus::Partial
        );
        assert!(result.steps.iter().any(|step| {
            step.status == remediation::RemediationStepStatus::Failed
                && step.detail.as_deref() == Some("Access was denied while applying one step.")
        }));
        assert!(
            action_run_status_text(&run)
                .starts_with("Remediation finished with partial results · 1 action reviewed")
        );
    }

    #[test]
    fn deterministic_machine_identity_remains_the_exact_store_2_5_8_fixture() {
        let system_info = fixture_258_system_info();
        assert_eq!(system_info.computer_name, "ANDROMEDA");
        assert_eq!(system_info.os_version, "Windows 11 Professional (25H2)");
        assert!(!system_info.is_admin);
        assert_eq!(privilege_label(system_info.is_admin), "Standard user");
        assert_eq!(
            machine_card_accessibility_name(&system_info, None, None),
            "Computer ANDROMEDA, Windows 11 Professional (25H2), Standard user"
        );
    }
}
