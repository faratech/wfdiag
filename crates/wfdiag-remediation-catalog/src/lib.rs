//! Canonical, UI-neutral remediation metadata for `WFDiag`.
//!
//! This crate contains no command lines, execution hooks, confirmation grants,
//! filesystem access, or UI-framework dependencies. Native issue detection and
//! every application shell can therefore share the exact same read-only action
//! projection while execution remains exclusively in the trusted action broker.

#![deny(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Safety tier of a cataloged remediation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationTier {
    OpenTool,
    AutoSafe,
    Repair,
}

/// Immutable metadata shared by issue detection, action previews, and shells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct RemediationMetadata {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub tier: RemediationTier,
    pub admin_required: bool,
    pub requires_restart: bool,
    pub long_running: bool,
    pub maintenance: bool,
    /// Read-only projection of the execution catalog's cancellation policy.
    /// The action broker still derives and enforces cancellation from `RunKind`.
    pub cancellable: bool,
}

impl RemediationMetadata {
    /// Only low-impact, non-elevated, non-restarting `AutoSafe` actions may be
    /// proposed as a batch.
    #[must_use]
    pub const fn batch_eligible(&self) -> bool {
        matches!(self.tier, RemediationTier::AutoSafe)
            && !self.admin_required
            && !self.requires_restart
            && !self.long_running
    }

    /// Produce the exact serialized remediation projection used by `WFDiag` 2.5.8.
    #[must_use]
    pub fn summary(&self) -> RemediationSummary {
        RemediationSummary {
            id: self.id.to_string(),
            label: self.label.to_string(),
            description: self.description.to_string(),
            tier: self.tier,
            admin_required: self.admin_required,
            requires_restart: self.requires_restart,
            long_running: self.long_running,
            maintenance: self.maintenance,
            batch_eligible: self.batch_eligible(),
            cancellable: self.cancellable,
        }
    }
}

/// Read-only remediation projection embedded in Issues and action previews.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct RemediationSummary {
    pub id: String,
    pub label: String,
    pub description: String,
    pub tier: RemediationTier,
    pub admin_required: bool,
    pub requires_restart: bool,
    pub long_running: bool,
    pub maintenance: bool,
    pub batch_eligible: bool,
    pub cancellable: bool,
}

pub const REMEDIATION_COUNT: usize = 17;

/// The single canonical metadata catalog, in the shipping display order.
pub static REMEDIATIONS: [RemediationMetadata; REMEDIATION_COUNT] = [
    RemediationMetadata {
        id: "open_defrag",
        label: "Open Optimize Drives",
        description: "Opens the Windows Optimize Drives tool (dfrgui.exe).",
        tier: RemediationTier::OpenTool,
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: false,
        cancellable: false,
    },
    RemediationMetadata {
        id: "open_disk_cleanup",
        label: "Open Disk Cleanup",
        description: "Opens Windows Disk Cleanup (cleanmgr.exe) to pick what to remove.",
        tier: RemediationTier::OpenTool,
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: false,
        cancellable: false,
    },
    RemediationMetadata {
        id: "open_task_manager",
        label: "Open Task Manager",
        description: "Opens Task Manager (taskmgr.exe) to inspect processes and startup apps.",
        tier: RemediationTier::OpenTool,
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: false,
        cancellable: false,
    },
    RemediationMetadata {
        id: "open_windows_update",
        label: "Open Windows Update",
        description: "Opens Settings > Windows Update (ms-settings:windowsupdate).",
        tier: RemediationTier::OpenTool,
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: false,
        cancellable: false,
    },
    RemediationMetadata {
        id: "open_security_center",
        label: "Open Windows Security",
        description: "Opens the Windows Security app (windowsdefender://).",
        tier: RemediationTier::OpenTool,
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: false,
        cancellable: false,
    },
    RemediationMetadata {
        id: "open_device_manager",
        label: "Open Device Manager",
        description: "Opens Device Manager to inspect flagged devices.",
        tier: RemediationTier::OpenTool,
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: false,
        cancellable: false,
    },
    RemediationMetadata {
        id: "open_system_protection",
        label: "Open System Protection",
        description: "Opens System Protection settings (SystemPropertiesProtection.exe) to manage restore points.",
        tier: RemediationTier::OpenTool,
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: false,
        cancellable: false,
    },
    RemediationMetadata {
        id: "flush_dns",
        label: "Flush DNS cache",
        description: "Runs 'ipconfig /flushdns' to clear cached DNS lookups.",
        tier: RemediationTier::AutoSafe,
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: true,
        cancellable: true,
    },
    RemediationMetadata {
        id: "clear_icon_cache",
        label: "Rebuild icon & thumbnail cache",
        description: "Deletes IconCache.db and Explorer thumbnail caches; they rebuild on next sign-in.",
        tier: RemediationTier::AutoSafe,
        admin_required: false,
        requires_restart: true,
        long_running: false,
        maintenance: true,
        cancellable: true,
    },
    RemediationMetadata {
        id: "empty_recycle_bin",
        label: "Empty Recycle Bin",
        description: "Permanently removes the current contents of the Recycle Bin using the Windows Shell API.",
        tier: RemediationTier::Repair,
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: true,
        cancellable: false,
    },
    RemediationMetadata {
        id: "clear_temp_files",
        label: "Clean temp files",
        description: "Permanently deletes files and folders in the user temp directory; locked items are skipped.",
        tier: RemediationTier::Repair,
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: true,
        cancellable: true,
    },
    RemediationMetadata {
        id: "start_critical_services",
        label: "Start stopped core services",
        description: "Runs 'sc start' for wuauserv, BITS, Spooler, Themes and AudioSrv.",
        tier: RemediationTier::AutoSafe,
        admin_required: true,
        requires_restart: false,
        long_running: false,
        maintenance: false,
        cancellable: true,
    },
    RemediationMetadata {
        id: "windows_update_reset",
        label: "Reset Windows Update",
        description: "Stops the Windows Update service, clears the SoftwareDistribution download cache, and restarts the service.",
        tier: RemediationTier::Repair,
        admin_required: true,
        requires_restart: false,
        long_running: false,
        maintenance: true,
        cancellable: false,
    },
    RemediationMetadata {
        id: "dism_restorehealth",
        label: "Repair Windows image (DISM)",
        description: "Runs 'DISM /Online /Cleanup-Image /RestoreHealth' to repair the Windows component store. Can take 10-30 minutes.",
        tier: RemediationTier::Repair,
        admin_required: true,
        requires_restart: false,
        long_running: true,
        maintenance: true,
        cancellable: false,
    },
    RemediationMetadata {
        id: "sfc_scannow",
        label: "System File Checker",
        description: "Runs 'sfc /scannow' to verify and repair protected system files. Can take 5-15 minutes.",
        tier: RemediationTier::Repair,
        admin_required: true,
        requires_restart: false,
        long_running: true,
        maintenance: true,
        cancellable: false,
    },
    RemediationMetadata {
        id: "network_reset",
        label: "Reset network stack",
        description: "Runs 'netsh winsock reset' and 'netsh int ip reset'. Requires a restart to take effect.",
        tier: RemediationTier::Repair,
        admin_required: true,
        requires_restart: true,
        long_running: false,
        maintenance: true,
        cancellable: true,
    },
    RemediationMetadata {
        id: "restart_system",
        label: "Restart Windows (60s)",
        description: "Schedules a restart in 60 seconds via 'shutdown /r /t 60'. Cancel with 'shutdown /a'.",
        tier: RemediationTier::Repair,
        admin_required: false,
        requires_restart: true,
        long_running: false,
        maintenance: false,
        cancellable: false,
    },
];

pub const OPEN_DEFRAG: &RemediationMetadata = &REMEDIATIONS[0];
pub const OPEN_DISK_CLEANUP: &RemediationMetadata = &REMEDIATIONS[1];
pub const OPEN_TASK_MANAGER: &RemediationMetadata = &REMEDIATIONS[2];
pub const OPEN_WINDOWS_UPDATE: &RemediationMetadata = &REMEDIATIONS[3];
pub const OPEN_SECURITY_CENTER: &RemediationMetadata = &REMEDIATIONS[4];
pub const OPEN_DEVICE_MANAGER: &RemediationMetadata = &REMEDIATIONS[5];
pub const OPEN_SYSTEM_PROTECTION: &RemediationMetadata = &REMEDIATIONS[6];
pub const FLUSH_DNS: &RemediationMetadata = &REMEDIATIONS[7];
pub const CLEAR_ICON_CACHE: &RemediationMetadata = &REMEDIATIONS[8];
pub const EMPTY_RECYCLE_BIN: &RemediationMetadata = &REMEDIATIONS[9];
pub const CLEAR_TEMP_FILES: &RemediationMetadata = &REMEDIATIONS[10];
pub const START_CRITICAL_SERVICES: &RemediationMetadata = &REMEDIATIONS[11];
pub const WINDOWS_UPDATE_RESET: &RemediationMetadata = &REMEDIATIONS[12];
pub const DISM_RESTOREHEALTH: &RemediationMetadata = &REMEDIATIONS[13];
pub const SFC_SCANNOW: &RemediationMetadata = &REMEDIATIONS[14];
pub const NETWORK_RESET: &RemediationMetadata = &REMEDIATIONS[15];
pub const RESTART_SYSTEM: &RemediationMetadata = &REMEDIATIONS[16];

#[must_use]
pub fn catalog() -> &'static [RemediationMetadata] {
    &REMEDIATIONS
}

#[must_use]
pub fn find(remediation_id: &str) -> Option<&'static RemediationMetadata> {
    catalog()
        .iter()
        .find(|metadata| metadata.id == remediation_id)
}

#[must_use]
pub fn summary(remediation_id: &str) -> Option<RemediationSummary> {
    find(remediation_id).map(RemediationMetadata::summary)
}

#[must_use]
pub fn summaries() -> Vec<RemediationSummary> {
    catalog().iter().map(RemediationMetadata::summary).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_is_complete_unique_and_nonempty() {
        assert_eq!(catalog().len(), REMEDIATION_COUNT);
        let mut ids = HashSet::new();
        for metadata in catalog() {
            assert!(ids.insert(metadata.id), "duplicate id {}", metadata.id);
            assert!(!metadata.label.is_empty(), "{} missing label", metadata.id);
            assert!(
                !metadata.description.is_empty(),
                "{} missing description",
                metadata.id
            );
            assert!(std::ptr::eq(find(metadata.id).unwrap(), metadata));
        }
    }

    #[test]
    fn named_aliases_remain_bound_to_their_execution_ids() {
        let aliases = [
            (OPEN_DEFRAG, "open_defrag"),
            (OPEN_DISK_CLEANUP, "open_disk_cleanup"),
            (OPEN_TASK_MANAGER, "open_task_manager"),
            (OPEN_WINDOWS_UPDATE, "open_windows_update"),
            (OPEN_SECURITY_CENTER, "open_security_center"),
            (OPEN_DEVICE_MANAGER, "open_device_manager"),
            (OPEN_SYSTEM_PROTECTION, "open_system_protection"),
            (FLUSH_DNS, "flush_dns"),
            (CLEAR_ICON_CACHE, "clear_icon_cache"),
            (EMPTY_RECYCLE_BIN, "empty_recycle_bin"),
            (CLEAR_TEMP_FILES, "clear_temp_files"),
            (START_CRITICAL_SERVICES, "start_critical_services"),
            (WINDOWS_UPDATE_RESET, "windows_update_reset"),
            (DISM_RESTOREHEALTH, "dism_restorehealth"),
            (SFC_SCANNOW, "sfc_scannow"),
            (NETWORK_RESET, "network_reset"),
            (RESTART_SYSTEM, "restart_system"),
        ];

        assert_eq!(aliases.len(), REMEDIATION_COUNT);
        for (metadata, expected_id) in aliases {
            assert_eq!(metadata.id, expected_id);
            assert!(std::ptr::eq(find(expected_id).unwrap(), metadata));
        }
    }

    #[test]
    fn summaries_preserve_the_shipping_wire_contract() {
        let value = serde_json::to_value(summaries()).unwrap();
        assert_eq!(value.as_array().unwrap().len(), REMEDIATION_COUNT);
        assert_eq!(
            value,
            serde_json::json!([
                {"id":"open_defrag","label":"Open Optimize Drives","description":"Opens the Windows Optimize Drives tool (dfrgui.exe).","tier":"open_tool","admin_required":false,"requires_restart":false,"long_running":false,"maintenance":false,"batch_eligible":false,"cancellable":false},
                {"id":"open_disk_cleanup","label":"Open Disk Cleanup","description":"Opens Windows Disk Cleanup (cleanmgr.exe) to pick what to remove.","tier":"open_tool","admin_required":false,"requires_restart":false,"long_running":false,"maintenance":false,"batch_eligible":false,"cancellable":false},
                {"id":"open_task_manager","label":"Open Task Manager","description":"Opens Task Manager (taskmgr.exe) to inspect processes and startup apps.","tier":"open_tool","admin_required":false,"requires_restart":false,"long_running":false,"maintenance":false,"batch_eligible":false,"cancellable":false},
                {"id":"open_windows_update","label":"Open Windows Update","description":"Opens Settings > Windows Update (ms-settings:windowsupdate).","tier":"open_tool","admin_required":false,"requires_restart":false,"long_running":false,"maintenance":false,"batch_eligible":false,"cancellable":false},
                {"id":"open_security_center","label":"Open Windows Security","description":"Opens the Windows Security app (windowsdefender://).","tier":"open_tool","admin_required":false,"requires_restart":false,"long_running":false,"maintenance":false,"batch_eligible":false,"cancellable":false},
                {"id":"open_device_manager","label":"Open Device Manager","description":"Opens Device Manager to inspect flagged devices.","tier":"open_tool","admin_required":false,"requires_restart":false,"long_running":false,"maintenance":false,"batch_eligible":false,"cancellable":false},
                {"id":"open_system_protection","label":"Open System Protection","description":"Opens System Protection settings (SystemPropertiesProtection.exe) to manage restore points.","tier":"open_tool","admin_required":false,"requires_restart":false,"long_running":false,"maintenance":false,"batch_eligible":false,"cancellable":false},
                {"id":"flush_dns","label":"Flush DNS cache","description":"Runs 'ipconfig /flushdns' to clear cached DNS lookups.","tier":"auto_safe","admin_required":false,"requires_restart":false,"long_running":false,"maintenance":true,"batch_eligible":true,"cancellable":true},
                {"id":"clear_icon_cache","label":"Rebuild icon & thumbnail cache","description":"Deletes IconCache.db and Explorer thumbnail caches; they rebuild on next sign-in.","tier":"auto_safe","admin_required":false,"requires_restart":true,"long_running":false,"maintenance":true,"batch_eligible":false,"cancellable":true},
                {"id":"empty_recycle_bin","label":"Empty Recycle Bin","description":"Permanently removes the current contents of the Recycle Bin using the Windows Shell API.","tier":"repair","admin_required":false,"requires_restart":false,"long_running":false,"maintenance":true,"batch_eligible":false,"cancellable":false},
                {"id":"clear_temp_files","label":"Clean temp files","description":"Permanently deletes files and folders in the user temp directory; locked items are skipped.","tier":"repair","admin_required":false,"requires_restart":false,"long_running":false,"maintenance":true,"batch_eligible":false,"cancellable":true},
                {"id":"start_critical_services","label":"Start stopped core services","description":"Runs 'sc start' for wuauserv, BITS, Spooler, Themes and AudioSrv.","tier":"auto_safe","admin_required":true,"requires_restart":false,"long_running":false,"maintenance":false,"batch_eligible":false,"cancellable":true},
                {"id":"windows_update_reset","label":"Reset Windows Update","description":"Stops the Windows Update service, clears the SoftwareDistribution download cache, and restarts the service.","tier":"repair","admin_required":true,"requires_restart":false,"long_running":false,"maintenance":true,"batch_eligible":false,"cancellable":false},
                {"id":"dism_restorehealth","label":"Repair Windows image (DISM)","description":"Runs 'DISM /Online /Cleanup-Image /RestoreHealth' to repair the Windows component store. Can take 10-30 minutes.","tier":"repair","admin_required":true,"requires_restart":false,"long_running":true,"maintenance":true,"batch_eligible":false,"cancellable":false},
                {"id":"sfc_scannow","label":"System File Checker","description":"Runs 'sfc /scannow' to verify and repair protected system files. Can take 5-15 minutes.","tier":"repair","admin_required":true,"requires_restart":false,"long_running":true,"maintenance":true,"batch_eligible":false,"cancellable":false},
                {"id":"network_reset","label":"Reset network stack","description":"Runs 'netsh winsock reset' and 'netsh int ip reset'. Requires a restart to take effect.","tier":"repair","admin_required":true,"requires_restart":true,"long_running":false,"maintenance":true,"batch_eligible":false,"cancellable":true},
                {"id":"restart_system","label":"Restart Windows (60s)","description":"Schedules a restart in 60 seconds via 'shutdown /r /t 60'. Cancel with 'shutdown /a'.","tier":"repair","admin_required":false,"requires_restart":true,"long_running":false,"maintenance":false,"batch_eligible":false,"cancellable":false}
            ])
        );
    }
}
