pub use wfdiag_remediation_catalog::{RemediationSummary, RemediationTier};

/// Default resolver for direct pure-detector calls.
///
/// Production shells use `detect_all_with` or `IssueRuntime`, supplying the
/// canonical remediation summaries from `wfdiag-remediation-catalog`.
#[cfg(test)]
pub(crate) fn summary(remediation_id: &str) -> Option<RemediationSummary> {
    wfdiag_remediation_catalog::summary(remediation_id)
}
