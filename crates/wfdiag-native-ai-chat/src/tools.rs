//! Provider-neutral text and JSON envelopes for the bounded chat tools.
//!
//! Every function here is pure: the host maps its own domain values (scan
//! results, detected issues, remediation catalog entries) onto the small
//! borrowed views below and receives exactly the bounded, model-visible text
//! both shells send today. Keeping the wording, ordering, and JSON envelope
//! shapes in this crate makes them testable without a Windows host.

use serde_json::{Value, json};

use crate::{
    BoundedToolCatalog, DiagnosticToolDescriptor, RemediationToolDescriptor, ScanCoverage,
};

/// Upper bound on the compact argument summary shown in tool activity.
const TOOL_ARGS_SUMMARY_CHARS: usize = 80;

/// Build the bounded catalog from the host's diagnostic tasks (id and
/// description) and the ids of its vetted remediations.
#[must_use]
pub fn tool_catalog<'a>(
    tasks: impl IntoIterator<Item = (&'a str, &'a str)>,
    remediation_ids: impl IntoIterator<Item = &'a str>,
) -> BoundedToolCatalog {
    BoundedToolCatalog::new(
        tasks
            .into_iter()
            .map(|(id, description)| DiagnosticToolDescriptor {
                id: id.to_string(),
                description: description.to_string(),
            })
            .collect(),
        remediation_ids
            .into_iter()
            .map(|id| RemediationToolDescriptor { id: id.to_string() })
            .collect(),
    )
}

/// Compact, user-visible summary of a tool call's arguments. The model's
/// free-text `reason` is deliberately excluded: it is prompt content, not an
/// argument the user needs to see in the activity row.
#[must_use]
pub fn summarize_tool_arguments(arguments: &Value) -> String {
    let summary = arguments.as_object().map_or_else(
        || arguments.to_string(),
        |map| {
            map.iter()
                .filter_map(|(key, value)| {
                    if key == "reason" {
                        return None;
                    }
                    let value = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_string);
                    Some(format!("{key}: {value}"))
                })
                .collect::<Vec<_>>()
                .join(", ")
        },
    );
    summary.chars().take(TOOL_ARGS_SUMMARY_CHARS).collect()
}

/// Scan breadth as the host classified it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanTextKind {
    Quick,
    Full,
    Targeted,
}

impl ScanTextKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Full => "full",
            Self::Targeted => "targeted",
        }
    }
}

/// One collected task result as the model sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskResultText<'a> {
    pub task_id: &'a str,
    pub success: bool,
    pub output: &'a str,
    pub error: Option<&'a str>,
}

/// The host's current scan projected onto the fields the prompt text uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanText<'a> {
    pub kind: ScanTextKind,
    pub running: bool,
    pub selected_tasks: usize,
    /// Whole minutes since the scan started, as measured by the host clock.
    pub age_minutes: u64,
    pub results: Vec<TaskResultText<'a>>,
}

/// Classify how much evidence one scan actually covers.
#[must_use]
pub fn scan_coverage(scan: Option<&ScanText<'_>>) -> ScanCoverage {
    let Some(scan) = scan else {
        return ScanCoverage::None;
    };
    if scan.running || scan.results.len() < scan.selected_tasks {
        return ScanCoverage::InProgress;
    }
    match scan.kind {
        ScanTextKind::Quick => ScanCoverage::Quick,
        ScanTextKind::Full => ScanCoverage::Full,
        ScanTextKind::Targeted => ScanCoverage::Targeted,
    }
}

/// Deterministic scan evidence: a machine-readable scope line followed by one
/// line per task, ordered by task id so repeated turns are byte-identical.
#[must_use]
pub fn scan_summary_text(overview: Option<&str>, scan: Option<&ScanText<'_>>) -> String {
    let overview = overview.map_or_else(String::new, |overview| {
        format!("SYSTEM OVERVIEW\n{overview}\n")
    });
    let Some(scan) = scan else {
        return format!(
            "{overview}SCAN_SCOPE kind=none state=empty selected=0 completed=0. No scan data is available yet."
        );
    };
    let kind = scan.kind.as_str();
    let state = if scan.running { "running" } else { "complete" };
    let collected = scan.results.iter().filter(|result| result.success).count();
    let failures = scan.results.len().saturating_sub(collected);
    let age_minutes = scan.age_minutes;
    let mut lines = vec![format!(
        "{overview}SCAN_SCOPE kind={kind} state={state} selected={} completed={}. Scan from {age_minutes} minute(s) ago: {collected} collected, {failures} collection failures.",
        scan.selected_tasks,
        scan.results.len(),
    )];
    let mut results = scan.results.clone();
    results.sort_by(|left, right| left.task_id.cmp(right.task_id));
    lines.extend(results.into_iter().map(|result| {
        if result.success {
            format!("{}: COLLECTED — {}", result.task_id, result.output)
        } else {
            format!(
                "{}: COLLECTION ERROR ({})",
                result.task_id,
                result.error.unwrap_or("unknown error")
            )
        }
    }));
    lines.join("\n")
}

/// Severity as rendered into issue evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueTextSeverity {
    Critical,
    Warning,
    Info,
    Ok,
}

impl IssueTextSeverity {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::Warning => "WARNING",
            Self::Info => "INFO",
            Self::Ok => "OK",
        }
    }
}

/// Detection outcome for one catalog check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueTextStatus {
    /// The rule fired: the issue is present.
    Detected,
    /// The rule could not be evaluated (missing or skipped evidence).
    Unverified,
    /// The rule evaluated cleanly; healthy checks are never listed.
    Ok,
}

/// One catalog issue projected onto the fields the evidence text uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IssueText<'a> {
    pub id: &'a str,
    pub remediation_id: Option<&'a str>,
    pub severity: IssueTextSeverity,
    pub status: IssueTextStatus,
    pub title: &'a str,
    pub description: &'a str,
    pub recommendation: &'a str,
}

/// Rule-based issue evidence: detected issues first, then the checks that
/// could not be verified. Healthy checks are omitted entirely.
#[must_use]
pub fn detected_issues_text(issues: &[IssueText<'_>]) -> String {
    if issues.is_empty() {
        return "Detected issues: none. No rule-based issue evidence is available for this scan."
            .to_string();
    }
    let mut detected = Vec::new();
    let mut unknown = Vec::new();
    for issue in issues {
        match issue.status {
            IssueTextStatus::Detected => detected.push(format!(
                "Issue ID: {} | Remediation ID: {} | Severity: {} | {} — {} | Recommendation: {}",
                issue.id,
                issue.remediation_id.unwrap_or("none"),
                issue.severity.as_str(),
                issue.title,
                issue.description,
                issue.recommendation,
            )),
            IssueTextStatus::Unverified => unknown.push(format!(
                "Issue ID: {} | Status: UNKNOWN | {} — {}",
                issue.id, issue.title, issue.description
            )),
            IssueTextStatus::Ok => {}
        }
    }
    let mut sections = vec![if detected.is_empty() {
        "Detected issues: none.".to_string()
    } else {
        format!(
            "{} issue(s) detected:\n{}",
            detected.len(),
            detected.join("\n")
        )
    }];
    if !unknown.is_empty() {
        sections.push(format!(
            "{} check(s) could not be verified:\n{}",
            unknown.len(),
            unknown.join("\n")
        ));
    }
    sections.join("\n\n")
}

/// Answer a `request_full_scan` call. A Full Scan is only ever *requested*:
/// the returned envelope asks the UI to confirm and starts nothing.
pub fn request_full_scan_envelope(coverage: ScanCoverage, reason: &str) -> Result<String, String> {
    match coverage {
        ScanCoverage::Quick | ScanCoverage::Targeted => serde_json::to_string(&json!({
            "kind": "scan_request",
            "scanKind": "full",
            "reason": reason,
            "notice": "Confirmation requested only. The Full Scan has not started."
        }))
        .map_err(|error| format!("Could not serialize the Full Scan request: {error}")),
        ScanCoverage::None => Ok(
            "No scan evidence is available. Complete the automatic Quick Scan first.".to_string(),
        ),
        ScanCoverage::InProgress => {
            Ok("A scan is still in progress. Wait for it to finish.".to_string())
        }
        ScanCoverage::Full => {
            Ok("The current session already contains a Full Scan. Use its evidence.".to_string())
        }
    }
}

/// One vetted remediation the model may stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StageableRemediation<'a> {
    pub id: &'a str,
    pub label: &'a str,
    /// Always-available maintenance entries may be staged without an issue.
    pub maintenance: bool,
}

/// One *detected* issue and the remediation the catalog maps it to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StageableIssue<'a> {
    pub id: &'a str,
    pub remediation_id: Option<&'a str>,
}

/// Answer a `stage_remediation` call. Nothing is executed here: the envelope
/// only asks the UI to present the proposal for the user's exact approval, and
/// the model may only name a catalog id that is mapped to a detected issue (or
/// an always-available maintenance entry).
///
/// `detected_issues` must contain detected issues only.
pub fn stage_remediation_envelope(
    remediations: &[StageableRemediation<'_>],
    detected_issues: &[StageableIssue<'_>],
    remediation_id: &str,
    issue_id: Option<&str>,
) -> Result<String, String> {
    let remediation = remediations
        .iter()
        .find(|remediation| remediation.id == remediation_id)
        .ok_or_else(|| format!("Unknown remediation '{remediation_id}'"))?;
    if let Some(issue_id) = issue_id {
        let issue = detected_issues
            .iter()
            .find(|issue| issue.id == issue_id)
            .ok_or_else(|| format!("Detected issue '{issue_id}' is not present"))?;
        if issue.remediation_id != Some(remediation_id) {
            return Err(format!(
                "Remediation '{remediation_id}' is not mapped to issue '{issue_id}'"
            ));
        }
    } else if !remediation.maintenance {
        return Err(format!(
            "Remediation '{remediation_id}' requires its detected issue id"
        ));
    }
    serde_json::to_string(&json!({
        "kind": "staged_action_proposal",
        "proposal": {
            "remediationId": remediation_id,
            "issueId": issue_id,
            "label": remediation.label,
        },
        "notice": "Staged only. Awaiting the user's exact approval; nothing was executed."
    }))
    .map_err(|error| format!("Could not serialize staged proposal: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(kind: ScanTextKind, running: bool) -> ScanText<'static> {
        ScanText {
            kind,
            running,
            selected_tasks: 1,
            age_minutes: 3,
            results: vec![TaskResultText {
                task_id: "os_info",
                success: true,
                output: "Windows 11",
                error: None,
            }],
        }
    }

    #[test]
    fn catalog_exposes_exactly_the_canonical_ten_tools() {
        let catalog = tool_catalog(
            [("os_info", "Collect Windows version information")],
            ["open_disk_cleanup"],
        );
        let names = catalog
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "run_diagnostic",
                "search_windows_knowledge",
                "get_scan_summary",
                "request_full_scan",
                "get_detected_issues",
                "compare_with_previous_scan",
                "get_live_stats",
                "list_remediations",
                "list_scan_history",
                "stage_remediation",
            ]
        );
        assert!(!names.iter().any(|name| name == "get_system_overview"));
    }

    #[test]
    fn argument_summary_drops_the_reason_and_stays_bounded() {
        assert_eq!(
            summarize_tool_arguments(&json!({"task_id": "os_info", "reason": "hidden"})),
            "task_id: os_info"
        );
        let long = summarize_tool_arguments(&json!({"query": "x".repeat(200)}));
        assert_eq!(long.chars().count(), TOOL_ARGS_SUMMARY_CHARS);
    }

    #[test]
    fn scan_summary_is_ordered_and_reports_scope() {
        let mut evidence = scan(ScanTextKind::Quick, false);
        evidence.selected_tasks = 2;
        evidence.results.insert(
            0,
            TaskResultText {
                task_id: "zz_last",
                success: false,
                output: "",
                error: Some("access denied"),
            },
        );
        let text = scan_summary_text(Some("Windows 11 · ARM64"), Some(&evidence));

        assert!(text.starts_with("SYSTEM OVERVIEW\nWindows 11 · ARM64\nSCAN_SCOPE kind=quick"));
        assert!(text.contains("state=complete selected=2 completed=2"));
        assert!(text.contains("1 collected, 1 collection failures"));
        let os_info = text.find("os_info: COLLECTED").expect("collected line");
        let failure = text
            .find("zz_last: COLLECTION ERROR (access denied)")
            .expect("failure line");
        assert!(os_info < failure, "task lines must be ordered by task id");

        assert_eq!(
            scan_summary_text(None, None),
            "SCAN_SCOPE kind=none state=empty selected=0 completed=0. No scan data is available yet."
        );
    }

    #[test]
    fn coverage_treats_partial_and_running_scans_as_in_progress() {
        assert_eq!(scan_coverage(None), ScanCoverage::None);
        assert_eq!(
            scan_coverage(Some(&scan(ScanTextKind::Quick, true))),
            ScanCoverage::InProgress
        );
        let mut partial = scan(ScanTextKind::Full, false);
        partial.selected_tasks = 5;
        assert_eq!(scan_coverage(Some(&partial)), ScanCoverage::InProgress);
        assert_eq!(
            scan_coverage(Some(&scan(ScanTextKind::Full, false))),
            ScanCoverage::Full
        );
        assert_eq!(
            scan_coverage(Some(&scan(ScanTextKind::Targeted, false))),
            ScanCoverage::Targeted
        );
    }

    #[test]
    fn issue_text_separates_detected_from_unverified_and_hides_healthy() {
        let issues = [
            IssueText {
                id: "low_disk_space",
                remediation_id: Some("open_disk_cleanup"),
                severity: IssueTextSeverity::Warning,
                status: IssueTextStatus::Detected,
                title: "Low disk space",
                description: "C: has little free space",
                recommendation: "Review Disk Cleanup",
            },
            IssueText {
                id: "tpm_ready",
                remediation_id: None,
                severity: IssueTextSeverity::Info,
                status: IssueTextStatus::Unverified,
                title: "TPM",
                description: "Not collected",
                recommendation: "Retry",
            },
            IssueText {
                id: "secure_boot",
                remediation_id: None,
                severity: IssueTextSeverity::Ok,
                status: IssueTextStatus::Ok,
                title: "Secure Boot",
                description: "Enabled",
                recommendation: "None",
            },
        ];
        let text = detected_issues_text(&issues);

        assert!(text.starts_with("1 issue(s) detected:"));
        assert!(text.contains("Remediation ID: open_disk_cleanup | Severity: WARNING"));
        assert!(text.contains("1 check(s) could not be verified:"));
        assert!(text.contains("Issue ID: tpm_ready | Status: UNKNOWN"));
        assert!(!text.contains("secure_boot"));
        assert_eq!(
            detected_issues_text(&[]),
            "Detected issues: none. No rule-based issue evidence is available for this scan."
        );
    }

    #[test]
    fn full_scan_request_only_confirms_from_narrower_coverage() {
        let envelope = request_full_scan_envelope(ScanCoverage::Quick, "need driver evidence")
            .expect("quick coverage should request confirmation");
        let envelope = serde_json::from_str::<Value>(&envelope).unwrap();
        assert_eq!(envelope["kind"], "scan_request");
        assert_eq!(envelope["scanKind"], "full");
        assert!(
            envelope["notice"]
                .as_str()
                .unwrap()
                .contains("has not started")
        );

        for coverage in [
            ScanCoverage::None,
            ScanCoverage::InProgress,
            ScanCoverage::Full,
        ] {
            let answer = request_full_scan_envelope(coverage, "reason").unwrap();
            assert!(serde_json::from_str::<Value>(&answer).is_err());
        }
    }

    #[test]
    fn staging_requires_a_catalog_id_mapped_to_a_detected_issue() {
        let remediations = [
            StageableRemediation {
                id: "open_disk_cleanup",
                label: "Open Disk Cleanup",
                maintenance: false,
            },
            StageableRemediation {
                id: "run_sfc",
                label: "Run System File Checker",
                maintenance: true,
            },
        ];
        let issues = [StageableIssue {
            id: "low_disk_space",
            remediation_id: Some("open_disk_cleanup"),
        }];

        let staged = stage_remediation_envelope(
            &remediations,
            &issues,
            "open_disk_cleanup",
            Some("low_disk_space"),
        )
        .expect("mapped remediation should stage");
        let staged = serde_json::from_str::<Value>(&staged).unwrap();
        assert_eq!(staged["kind"], "staged_action_proposal");
        assert_eq!(staged["proposal"]["remediationId"], "open_disk_cleanup");
        assert_eq!(staged["proposal"]["label"], "Open Disk Cleanup");
        assert!(
            staged["notice"]
                .as_str()
                .unwrap()
                .contains("nothing was executed")
        );

        assert!(
            stage_remediation_envelope(&remediations, &issues, "run_sfc", None).is_ok(),
            "maintenance entries stage without an issue"
        );
        assert_eq!(
            stage_remediation_envelope(&remediations, &issues, "powershell", None),
            Err("Unknown remediation 'powershell'".to_string())
        );
        assert_eq!(
            stage_remediation_envelope(&remediations, &issues, "open_disk_cleanup", None),
            Err("Remediation 'open_disk_cleanup' requires its detected issue id".to_string())
        );
        assert_eq!(
            stage_remediation_envelope(&remediations, &issues, "open_disk_cleanup", Some("absent")),
            Err("Detected issue 'absent' is not present".to_string())
        );
        assert_eq!(
            stage_remediation_envelope(&remediations, &issues, "run_sfc", Some("low_disk_space")),
            Err("Remediation 'run_sfc' is not mapped to issue 'low_disk_space'".to_string())
        );
    }
}
