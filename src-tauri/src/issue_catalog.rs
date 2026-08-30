//! Compatibility adapter from the shipping backend to the canonical native
//! issue catalog.

use crate::diagnostics::TaskResult;
use crate::timestamp::Timestamp;
use std::collections::HashMap;

pub use wfdiag_native_issues::{Issue, IssueSeverity, IssueStatus, RemediationSummary, catalog};

/// Shipping detection inputs retain the established timestamp type. The
/// adapter converts it to the portable detector clock without copying the
/// diagnostic result map.
pub struct DetectCtx<'a> {
    pub results: &'a HashMap<String, TaskResult>,
    pub now: Timestamp,
    pub temp_file_count: Option<usize>,
}

pub fn detect_all(ctx: &DetectCtx<'_>) -> Vec<Issue> {
    detect_all_with(ctx, &crate::remediation::summary)
}

pub fn detect_all_with(
    ctx: &DetectCtx<'_>,
    remediation_summary: &dyn Fn(&str) -> Option<RemediationSummary>,
) -> Vec<Issue> {
    let portable = wfdiag_native_issues::DetectCtx {
        results: ctx.results,
        now: wfdiag_native_issues::Timestamp::from_secs(ctx.now.secs),
        temp_file_count: ctx.temp_file_count,
    };
    wfdiag_native_issues::detect_all_with(&portable, remediation_summary)
}
