//! Native issue detection and a UI-framework-neutral worker boundary.
//!
//! The issue catalog and every detector live in this crate. Diagnostic
//! evidence, the clock, the temporary-file count, and read-only remediation
//! summaries are injected; detection never probes the environment itself.

#![deny(unsafe_code)]

mod diagnostics;
mod fix_plan;
#[allow(clippy::all, clippy::pedantic)]
pub mod issue_catalog;
#[allow(clippy::all, clippy::pedantic)]
pub mod issue_detector;
pub mod projection;
mod remediation;
mod runtime;
/// UTC timestamps, re-exported from `wfdiag-native-core` where the
/// implementation now lives. Kept as a module because `issue_catalog.rs`,
/// `issue_detector.rs` and both shells spell it `…::timestamp::Timestamp`.
pub mod timestamp {
    pub use wfdiag_native_core::timestamp::*;
}

pub use diagnostics::{
    DiagnosticTask, ScanEvidence, SharedScanEvidence, SharedTaskResult, TaskResult,
    TaskResultLookup, ordered_scan_evidence,
};
pub use fix_plan::{
    FixPlanEntry, MAX_FIX_PLAN_ENTRIES, MAX_FIX_PLAN_NOTES_CHARS, MAX_FIX_PLAN_RATIONALE_CHARS,
    ParsedFixPlan, build_fix_plan_prompt, parse_fix_plan,
};
pub use issue_catalog::{
    DetectCtx, DetectFn, Detection, DetectionOutcome, Issue, IssueSeverity, IssueSpec, IssueStatus,
    catalog, detect_all_with,
};
pub use remediation::{RemediationSummary, RemediationTier};
pub use runtime::{
    IssueDetectionCompleted, IssueDetectionRequest, IssueRuntime, IssueRuntimeError,
};
pub use timestamp::Timestamp;
pub use wfdiag_remediation_catalog::{
    RemediationMetadata, catalog as remediation_catalog, find as find_remediation_metadata,
    summaries as remediation_summaries,
};
