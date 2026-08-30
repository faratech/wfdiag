//! Native issue detection and a UI-framework-neutral worker boundary.
//!
//! The issue catalog and every detector live in this crate. Diagnostic
//! evidence, the clock, the temporary-file count, and read-only remediation
//! summaries are injected; detection never probes the environment itself.

#![deny(unsafe_code)]

mod diagnostics;
#[allow(dead_code, clippy::all, clippy::pedantic)]
#[path = "../../../src-tauri/src/error.rs"]
mod error;
#[allow(clippy::all, clippy::pedantic)]
pub mod issue_catalog;
#[allow(clippy::all, clippy::pedantic)]
pub mod issue_detector;
mod remediation;
mod runtime;
#[allow(dead_code, clippy::all, clippy::pedantic)]
#[path = "../../../src-tauri/src/timestamp.rs"]
pub mod timestamp;

pub use diagnostics::{DiagnosticTask, TaskResult};
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
