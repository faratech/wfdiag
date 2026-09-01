//! Strict, execution-free AI fix-plan prompt and response validation.
//!
//! This module is deliberately colocated with the canonical issue and
//! remediation metadata. A model response can only project an already
//! detected issue's one catalog-mapped remediation; it cannot invent an
//! action, alter catalog metadata, or reach remediation execution.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wfdiag_remediation_catalog::{RemediationMetadata, RemediationTier};

use crate::Issue;

pub const MAX_FIX_PLAN_ENTRIES: usize = 8;
pub const MAX_FIX_PLAN_RATIONALE_CHARS: usize = 300;
pub const MAX_FIX_PLAN_NOTES_CHARS: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixPlanEntry {
    pub issue_id: String,
    pub remediation_id: String,
    pub rationale: String,
    pub tier: RemediationTier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFixPlan {
    pub entries: Vec<FixPlanEntry>,
    pub notes: String,
}

/// Build the catalog-ID-only planning prompt shared by every application
/// shell. `max_data_chars` includes the truncation marker when one is needed.
#[must_use]
pub fn build_fix_plan_prompt(
    issues: &[Issue],
    catalog: &[RemediationMetadata],
    max_data_chars: usize,
) -> String {
    let issue_lines = issues
        .iter()
        .filter(|issue| issue.detected)
        .map(|issue| {
            let remediation_note = issue.remediation.as_ref().map_or_else(
                || "no vetted remediation available for this issue".to_string(),
                |remediation| format!("allowed remediation id: {}", remediation.id),
            );
            format!(
                "- {} [{:?}] {}: {} ({})",
                issue.id, issue.severity, issue.title, issue.description, remediation_note
            )
        })
        .collect::<Vec<_>>();
    let remediation_lines = catalog
        .iter()
        .map(|remediation| {
            format!(
                "- {} ({:?}{}): {} — {}",
                remediation.id,
                remediation.tier,
                if remediation.requires_restart {
                    ", requires restart"
                } else {
                    ""
                },
                remediation.label,
                remediation.description
            )
        })
        .collect::<Vec<_>>();

    let data = truncate_to_chars(
        &format!(
            "DETECTED ISSUES (each lists the one remediation id allowed for it):\n{}\n\n\
             REMEDIATION CATALOG (for context/descriptions only — not a free menu):\n{}",
            issue_lines.join("\n"),
            remediation_lines.join("\n")
        ),
        max_data_chars,
    );

    format!(
        "You are planning repairs for a Windows PC using ONLY this app's vetted remediations.\n\n\
         {data}\n\n\
         Respond with ONLY this JSON (no prose, no code fences):\n\
         {{\"entries\": [{{\"issue_id\": \"...\", \"remediation_id\": \"...\", \"rationale\": \"one sentence\"}}], \"notes\": \"one short paragraph\"}}\n\n\
         Rules:\n\
         - For each issue, use ONLY the exact remediation id listed as its \"allowed remediation id\" above — never a different catalog id, even one that looks relevant.\n\
         - If an issue has no vetted remediation available, leave it out and mention it in notes.\n\
         - Order entries most-important-first.\n\
         - At most {MAX_FIX_PLAN_ENTRIES} entries."
    )
}

/// Parse a model response through the catalog/evidence boundary.
///
/// Markdown fences are tolerated, but an entry survives only when both IDs
/// are app-owned, the issue is currently detected, and that issue's immutable
/// remediation mapping exactly matches the proposed remediation. Malformed or
/// hostile output degrades to an empty or shorter plan and never to an action.
#[must_use]
pub fn parse_fix_plan(
    text: &str,
    detected: &[Issue],
    catalog: &[RemediationMetadata],
) -> ParsedFixPlan {
    let json_slice = match (text.find('{'), text.rfind('}')) {
        (Some(start), Some(end)) if end > start => &text[start..=end],
        _ => return unusable_plan(),
    };
    let Ok(value) = serde_json::from_str::<Value>(json_slice) else {
        return unusable_plan();
    };

    let notes = value["notes"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(MAX_FIX_PLAN_NOTES_CHARS)
        .collect();
    let mut entries = Vec::new();
    let mut seen = HashSet::new();

    for entry in value["entries"].as_array().cloned().unwrap_or_default() {
        if entries.len() >= MAX_FIX_PLAN_ENTRIES {
            break;
        }
        let (Some(issue_id), Some(remediation_id)) =
            (entry["issue_id"].as_str(), entry["remediation_id"].as_str())
        else {
            continue;
        };
        let Some(spec) = catalog
            .iter()
            .find(|remediation| remediation.id == remediation_id)
        else {
            continue;
        };
        let Some(issue) = detected
            .iter()
            .find(|issue| issue.detected && issue.id == issue_id)
        else {
            continue;
        };
        if issue
            .remediation
            .as_ref()
            .is_none_or(|remediation| remediation.id != remediation_id)
        {
            continue;
        }
        if !seen.insert((issue_id.to_string(), remediation_id.to_string())) {
            continue;
        }

        entries.push(FixPlanEntry {
            issue_id: issue_id.to_string(),
            remediation_id: remediation_id.to_string(),
            rationale: entry["rationale"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(MAX_FIX_PLAN_RATIONALE_CHARS)
                .collect(),
            tier: spec.tier,
        });
    }

    ParsedFixPlan { entries, notes }
}

fn unusable_plan() -> ParsedFixPlan {
    ParsedFixPlan {
        entries: Vec::new(),
        notes: "The AI did not return a usable plan.".to_string(),
    }
}

fn truncate_to_chars(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }

    let suffix = format!("… [truncated from {total} chars]");
    let suffix_chars = suffix.chars().count();
    if suffix_chars >= max_chars {
        return suffix.chars().take(max_chars).collect();
    }
    let mut bounded = text
        .chars()
        .take(max_chars - suffix_chars)
        .collect::<String>();
    bounded.push_str(&suffix);
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueSeverity, IssueStatus, remediation_catalog, remediation_summaries};

    fn issue(id: &str, remediation_id: Option<&str>) -> Issue {
        Issue {
            id: id.to_string(),
            category: "Test".to_string(),
            severity: IssueSeverity::Warning,
            status: IssueStatus::Detected,
            title: id.to_string(),
            description: format!("{id} description"),
            recommendation: "fix it".to_string(),
            detected: true,
            source_tasks: None,
            remediation: remediation_id.and_then(|wanted| {
                remediation_summaries()
                    .into_iter()
                    .find(|summary| summary.id == wanted)
            }),
        }
    }

    #[test]
    fn prompt_is_bounded_and_exposes_only_detected_issue_mappings() {
        let mut clear = issue("clear_issue", Some("flush_dns"));
        clear.detected = false;
        let prompt = build_fix_plan_prompt(
            &[issue("disk_issue", Some("open_disk_cleanup")), clear],
            remediation_catalog(),
            10_000,
        );
        assert!(prompt.contains("disk_issue"));
        assert!(prompt.contains("allowed remediation id: open_disk_cleanup"));
        assert!(!prompt.contains("clear_issue"));
        assert!(prompt.contains("Respond with ONLY this JSON"));

        let unicode = issue(&"🪟".repeat(2_000), Some("open_disk_cleanup"));
        let bounded = build_fix_plan_prompt(&[unicode], remediation_catalog(), 220);
        let data = bounded
            .split("\n\nRespond with ONLY")
            .next()
            .expect("prompt has a data prefix");
        assert!(data.contains("truncated"));
    }

    #[test]
    fn parser_accepts_fences_and_exact_catalog_pair() {
        let parsed = parse_fix_plan(
            r#"```json
            {"entries":[{"issue_id":"disk_issue","remediation_id":"open_disk_cleanup","rationale":"Review disk usage."}],"notes":"One action."}
            ```"#,
            &[issue("disk_issue", Some("open_disk_cleanup"))],
            remediation_catalog(),
        );
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].remediation_id, "open_disk_cleanup");
        assert_eq!(parsed.entries[0].tier, RemediationTier::OpenTool);
        assert_eq!(parsed.notes, "One action.");
    }

    #[test]
    fn parser_drops_unknown_undetected_mismatched_unmapped_and_duplicate_ids() {
        let mut undetected = issue("not_now", Some("flush_dns"));
        undetected.detected = false;
        let parsed = parse_fix_plan(
            r#"{"entries":[
                {"issue_id":"temp_files","remediation_id":"clear_temp_files","rationale":"valid"},
                {"issue_id":"temp_files","remediation_id":"clear_temp_files","rationale":"duplicate"},
                {"issue_id":"temp_files","remediation_id":"flush_dns","rationale":"wrong mapping"},
                {"issue_id":"temp_files","remediation_id":"format_c_drive","rationale":"invented"},
                {"issue_id":"unmapped","remediation_id":"flush_dns","rationale":"no issue mapping"},
                {"issue_id":"not_now","remediation_id":"flush_dns","rationale":"stale"},
                {"issue_id":"missing","remediation_id":"flush_dns","rationale":"not detected"}
            ],"notes":"bounded"}"#,
            &[
                issue("temp_files", Some("clear_temp_files")),
                issue("unmapped", None),
                undetected,
            ],
            remediation_catalog(),
        );
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].rationale, "valid");
    }

    #[test]
    fn parser_caps_entries_and_unicode_text_without_panicking() {
        let detected = (0..12)
            .map(|index| issue(&format!("issue{index}"), Some("flush_dns")))
            .collect::<Vec<_>>();
        let entries = (0..12)
            .map(|index| {
                format!(
                    r#"{{"issue_id":"issue{index}","remediation_id":"flush_dns","rationale":"{}"}}"#,
                    "🪟".repeat(500)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let response = format!(
            r#"{{"entries":[{entries}],"notes":"{}"}}"#,
            "🪟".repeat(2_000)
        );
        let parsed = parse_fix_plan(&response, &detected, remediation_catalog());
        assert_eq!(parsed.entries.len(), MAX_FIX_PLAN_ENTRIES);
        assert_eq!(
            parsed.entries[0].rationale.chars().count(),
            MAX_FIX_PLAN_RATIONALE_CHARS
        );
        assert_eq!(parsed.notes.chars().count(), MAX_FIX_PLAN_NOTES_CHARS);
    }

    #[test]
    fn malformed_output_degrades_to_no_references() {
        for response in ["not JSON", "{broken", r#"{"entries":"wrong shape"}"#] {
            let parsed = parse_fix_plan(
                response,
                &[issue("temp_files", Some("clear_temp_files"))],
                remediation_catalog(),
            );
            assert!(parsed.entries.is_empty());
        }
    }
}
