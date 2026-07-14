//! Deterministic, budget-aware evidence packets for AI providers.
//!
//! This module is deliberately presentation-free. It converts a scan into a
//! compact set of provenance-bearing records and fits only complete records
//! into the caller's character budget. The user's current question and the
//! coverage/omission summary are never silently truncated.

use crate::diagnostics::TaskResult;
use crate::issue_catalog::{Issue, IssueSeverity, IssueStatus, catalog};
use crate::results_storage::{ComparisonResult, TaskChange};
use serde::Serialize;
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;

pub(crate) const EVIDENCE_SCHEMA_VERSION: &str = "wfdiag-evidence-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceKind {
    Issue,
    Diagnostic,
    Change,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceStatus {
    Detected,
    Unknown,
    Failed,
    Collected,
    Changed,
    Recovered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct EvidenceRecord {
    /// Packet-local reference used when an answer cites evidence (E1, E2...).
    pub id: String,
    pub source_id: String,
    pub kind: EvidenceKind,
    pub status: EvidenceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    pub title: String,
    pub value: String,
    pub source_tasks: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct CoverageSummary {
    pub collected_tasks: usize,
    pub failed_tasks: usize,
    pub detected_checks: usize,
    pub clear_checks: usize,
    pub unknown_checks: usize,
    pub total_checks: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct OmissionSummary {
    pub records: usize,
    pub detected: usize,
    pub unknown: usize,
    pub failed: usize,
    pub changes: usize,
    pub diagnostics: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct EvidencePacket {
    pub schema_version: &'static str,
    /// The exact current user question. It is never shortened to make room.
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_minutes: Option<u64>,
    pub coverage: CoverageSummary,
    pub records: Vec<EvidenceRecord>,
    pub omissions: OmissionSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactEvidence {
    pub packet: EvidencePacket,
    /// Stable line-oriented form intended for a model prompt.
    pub rendered: String,
}

pub(crate) struct EvidenceRequest<'a> {
    pub question: &'a str,
    pub scan_id: Option<&'a str>,
    pub captured_at: Option<&'a str>,
    pub age_minutes: Option<u64>,
    pub results: &'a HashMap<String, TaskResult>,
    pub issues: &'a [Issue],
    pub comparison: Option<&'a ComparisonResult>,
    /// Task/issue ids explicitly attached by the UI or selected by a tool.
    pub preferred_source_ids: &'a [String],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EvidencePolicy {
    pub max_chars: usize,
    pub max_record_chars: usize,
    pub max_records: usize,
    /// When false, successful diagnostics are considered only if explicitly
    /// requested, relevant to the question, or part of the small core set.
    pub include_collected_tasks: bool,
}

impl EvidencePolicy {
    pub(crate) fn compact(max_chars: usize) -> Self {
        Self {
            max_chars,
            // Tiny providers need room for at least one complete record after
            // the mandatory question/coverage metadata. Larger providers cap
            // each value to prevent one verbose diagnostic from dominating.
            max_record_chars: (max_chars / 5).clamp(72, 360),
            max_records: 20,
            include_collected_tasks: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EvidenceBuildError {
    EmptyQuestion,
    BudgetTooSmall {
        required_chars: usize,
        available_chars: usize,
    },
}

impl fmt::Display for EvidenceBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyQuestion => write!(f, "the current user question is empty"),
            Self::BudgetTooSmall {
                required_chars,
                available_chars,
            } => write!(
                f,
                "evidence budget is too small: at least {required_chars} characters are required, but only {available_chars} are available"
            ),
        }
    }
}

impl Error for EvidenceBuildError {}

#[derive(Debug, Clone)]
struct Candidate {
    source_id: String,
    kind: EvidenceKind,
    status: EvidenceStatus,
    severity: Option<String>,
    title: String,
    value: String,
    source_tasks: Vec<String>,
    priority: u8,
    relevance: usize,
    selectable: bool,
}

/// Build a deterministic evidence packet whose rendered form is guaranteed to
/// fit `policy.max_chars`. If even the exact question plus mandatory coverage
/// metadata cannot fit, this returns an error instead of losing the question.
pub(crate) fn build_compact_evidence(
    request: EvidenceRequest<'_>,
    policy: EvidencePolicy,
) -> Result<CompactEvidence, EvidenceBuildError> {
    if request.question.trim().is_empty() {
        return Err(EvidenceBuildError::EmptyQuestion);
    }

    let preferred: HashSet<&str> = request
        .preferred_source_ids
        .iter()
        .map(String::as_str)
        .collect();
    let question_terms = query_terms(request.question);
    let coverage = coverage_summary(request.results, request.issues);
    let mut candidates = build_candidates(&request, &preferred, &question_terms, policy);
    candidates.sort_by(|left, right| {
        (
            left.priority,
            Reverse(left.relevance),
            kind_rank(left.kind),
            left.source_id.as_str(),
            left.title.as_str(),
        )
            .cmp(&(
                right.priority,
                Reverse(right.relevance),
                kind_rank(right.kind),
                right.source_id.as_str(),
                right.title.as_str(),
            ))
    });

    let metadata = PacketMetadata {
        question: request.question,
        scan_id: request.scan_id,
        captured_at: request.captured_at,
        age_minutes: request.age_minutes,
        coverage: &coverage,
    };
    let selected = Vec::new();
    let minimal_omissions = omission_summary(&candidates, &selected);
    let minimal = render_packet(&metadata, &[], &minimal_omissions);
    let required_chars = char_count(&minimal);
    if required_chars > policy.max_chars {
        return Err(EvidenceBuildError::BudgetTooSmall {
            required_chars,
            available_chars: policy.max_chars,
        });
    }

    let mut selected_indices = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if selected_indices.len() >= policy.max_records || !candidate.selectable {
            continue;
        }
        let mut trial = selected_indices.clone();
        trial.push(index);
        let records = materialize_records(&candidates, &trial);
        let omissions = omission_summary(&candidates, &trial);
        let rendered = render_packet(&metadata, &records, &omissions);
        if char_count(&rendered) <= policy.max_chars {
            selected_indices = trial;
        }
    }

    let records = materialize_records(&candidates, &selected_indices);
    let omissions = omission_summary(&candidates, &selected_indices);
    let rendered = render_packet(&metadata, &records, &omissions);
    debug_assert!(char_count(&rendered) <= policy.max_chars);

    Ok(CompactEvidence {
        packet: EvidencePacket {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            question: request.question.to_string(),
            scan_id: request.scan_id.map(str::to_string),
            captured_at: request.captured_at.map(str::to_string),
            age_minutes: request.age_minutes,
            coverage,
            records,
            omissions,
        },
        rendered,
    })
}

fn coverage_summary(results: &HashMap<String, TaskResult>, issues: &[Issue]) -> CoverageSummary {
    let mut coverage = CoverageSummary {
        collected_tasks: results.values().filter(|result| result.success).count(),
        failed_tasks: results.values().filter(|result| !result.success).count(),
        total_checks: issues.len(),
        ..CoverageSummary::default()
    };
    for issue in issues {
        match issue.status {
            IssueStatus::Detected => coverage.detected_checks += 1,
            IssueStatus::Ok => coverage.clear_checks += 1,
            IssueStatus::Unknown | IssueStatus::Skipped => coverage.unknown_checks += 1,
        }
    }
    coverage
}

fn build_candidates(
    request: &EvidenceRequest<'_>,
    preferred: &HashSet<&str>,
    question_terms: &BTreeSet<String>,
    policy: EvidencePolicy,
) -> Vec<Candidate> {
    let issue_defaults: HashMap<&str, IssueSeverity> = catalog()
        .iter()
        .map(|spec| (spec.id, spec.default_severity))
        .collect();
    let mut candidates = Vec::new();

    for issue in request.issues {
        if issue.status == IssueStatus::Ok {
            continue;
        }
        let mut source_tasks = issue.source_tasks.clone().unwrap_or_default();
        source_tasks.sort();
        source_tasks.dedup();
        let explicitly_preferred = preferred.contains(issue.id.as_str())
            || source_tasks
                .iter()
                .any(|source| preferred.contains(source.as_str()));
        let severity = if matches!(issue.status, IssueStatus::Unknown | IssueStatus::Skipped) {
            issue_defaults
                .get(issue.id.as_str())
                .copied()
                .unwrap_or(IssueSeverity::Info)
        } else {
            issue.severity
        };
        let relevance = relevance_score(
            question_terms,
            &format!(
                "{} {} {} {} {}",
                issue.id, issue.category, issue.title, issue.description, issue.recommendation
            ),
        );
        let status = if issue.status == IssueStatus::Detected {
            EvidenceStatus::Detected
        } else {
            EvidenceStatus::Unknown
        };
        let priority = if explicitly_preferred {
            0
        } else {
            match (status, severity) {
                (EvidenceStatus::Detected, IssueSeverity::Critical) => 1,
                (EvidenceStatus::Detected, IssueSeverity::Warning) => 2,
                (EvidenceStatus::Detected, _) => 3,
                (EvidenceStatus::Unknown, IssueSeverity::Critical) => 2,
                (EvidenceStatus::Unknown, IssueSeverity::Warning) => 3,
                (EvidenceStatus::Unknown, _) => 4,
                _ => 5,
            }
        };
        let value = compact_plain_text(
            &format!(
                "{} Recommendation: {}",
                issue.description, issue.recommendation
            ),
            policy.max_record_chars,
        );
        candidates.push(Candidate {
            source_id: issue.id.clone(),
            kind: EvidenceKind::Issue,
            status,
            severity: Some(severity_name(severity).to_string()),
            title: compact_plain_text(&issue.title, 120),
            value,
            source_tasks,
            priority,
            relevance,
            selectable: true,
        });
    }

    let core_tasks = ["os_info", "logical_disk", "physical_memory", "processor"];
    let mut sorted_results: Vec<_> = request.results.iter().collect();
    sorted_results.sort_by_key(|(task_id, _)| task_id.as_str());
    for (task_id, result) in sorted_results {
        let explicitly_preferred = preferred.contains(task_id.as_str());
        let output_excerpt = relevance_excerpt(&result.output, 4_096);
        let relevance = relevance_score(
            question_terms,
            &format!(
                "{} {} {}",
                task_id,
                output_excerpt,
                result.error.as_deref().unwrap_or("")
            ),
        );
        let is_core = core_tasks.contains(&task_id.as_str());
        let status = if result.success {
            EvidenceStatus::Collected
        } else {
            EvidenceStatus::Failed
        };
        let priority = if explicitly_preferred {
            0
        } else if !result.success {
            1
        } else if relevance > 0 {
            5
        } else if is_core {
            6
        } else {
            8
        };
        let value = if result.success {
            compact_diagnostic_output(&result.output, question_terms, policy.max_record_chars)
        } else {
            compact_plain_text(
                result
                    .error
                    .as_deref()
                    .filter(|error| !error.trim().is_empty())
                    .unwrap_or(&result.output),
                policy.max_record_chars,
            )
        };
        candidates.push(Candidate {
            source_id: task_id.clone(),
            kind: EvidenceKind::Diagnostic,
            status,
            severity: None,
            title: task_id.replace('_', " "),
            value,
            source_tasks: vec![task_id.clone()],
            priority,
            relevance,
            selectable: !result.success
                || policy.include_collected_tasks
                || explicitly_preferred
                || relevance > 0
                || is_core,
        });
    }

    if let Some(comparison) = request.comparison {
        for change in &comparison.new_failures {
            candidates.push(change_candidate(
                change,
                EvidenceStatus::Failed,
                preferred,
                question_terms,
                policy.max_record_chars,
            ));
        }
        for change in &comparison.new_successes {
            candidates.push(change_candidate(
                change,
                EvidenceStatus::Recovered,
                preferred,
                question_terms,
                policy.max_record_chars,
            ));
        }
        for change in comparison
            .status_unchanged
            .iter()
            .filter(|change| change.output_changed)
        {
            candidates.push(change_candidate(
                change,
                EvidenceStatus::Changed,
                preferred,
                question_terms,
                policy.max_record_chars,
            ));
        }
    }

    candidates
}

fn change_candidate(
    change: &TaskChange,
    status: EvidenceStatus,
    preferred: &HashSet<&str>,
    question_terms: &BTreeSet<String>,
    max_chars: usize,
) -> Candidate {
    let relevance = relevance_score(
        question_terms,
        &format!(
            "{} {} {} {}",
            change.task_id,
            change.task_name,
            change.category,
            relevance_excerpt(&change.current_output, 4_096),
        ),
    );
    let explicitly_preferred = preferred.contains(change.task_id.as_str());
    let priority = if explicitly_preferred {
        0
    } else if status == EvidenceStatus::Failed {
        1
    } else {
        5
    };
    let side_budget = (max_chars / 3).max(24);
    let previous = compact_diagnostic_output(&change.previous_output, question_terms, side_budget);
    let current = compact_diagnostic_output(&change.current_output, question_terms, side_budget);
    let value = compact_plain_text(
        &format!(
            "success:{}→{} before:[{}] after:[{}]",
            change.previous_success, change.current_success, previous, current
        ),
        max_chars,
    );
    Candidate {
        source_id: format!("change:{}", change.task_id),
        kind: EvidenceKind::Change,
        status,
        severity: None,
        title: compact_plain_text(&change.task_name, 120),
        value,
        source_tasks: vec![change.task_id.clone()],
        priority,
        relevance,
        selectable: true,
    }
}

fn materialize_records(candidates: &[Candidate], selected: &[usize]) -> Vec<EvidenceRecord> {
    selected
        .iter()
        .enumerate()
        .map(|(record_index, candidate_index)| {
            let candidate = &candidates[*candidate_index];
            EvidenceRecord {
                id: format!("E{}", record_index + 1),
                source_id: candidate.source_id.clone(),
                kind: candidate.kind,
                status: candidate.status,
                severity: candidate.severity.clone(),
                title: candidate.title.clone(),
                value: candidate.value.clone(),
                source_tasks: candidate.source_tasks.clone(),
            }
        })
        .collect()
}

fn omission_summary(candidates: &[Candidate], selected: &[usize]) -> OmissionSummary {
    let selected: HashSet<usize> = selected.iter().copied().collect();
    let mut omissions = OmissionSummary::default();
    for (index, candidate) in candidates.iter().enumerate() {
        if selected.contains(&index) {
            continue;
        }
        omissions.records += 1;
        match candidate.status {
            EvidenceStatus::Detected => omissions.detected += 1,
            EvidenceStatus::Unknown => omissions.unknown += 1,
            EvidenceStatus::Failed => omissions.failed += 1,
            EvidenceStatus::Changed | EvidenceStatus::Recovered => omissions.changes += 1,
            EvidenceStatus::Collected => {}
        }
        if candidate.kind == EvidenceKind::Diagnostic {
            omissions.diagnostics += 1;
        }
    }
    omissions
}

struct PacketMetadata<'a> {
    question: &'a str,
    scan_id: Option<&'a str>,
    captured_at: Option<&'a str>,
    age_minutes: Option<u64>,
    coverage: &'a CoverageSummary,
}

fn render_packet(
    metadata: &PacketMetadata<'_>,
    records: &[EvidenceRecord],
    omissions: &OmissionSummary,
) -> String {
    // JSON string encoding preserves the exact question while preventing its
    // newlines or delimiter characters from becoming fake evidence records.
    let question_json = serde_json::to_string(metadata.question)
        .unwrap_or_else(|_| "\"<question encoding error>\"".to_string());
    let mut lines = vec![
        "EVIDENCE v1".to_string(),
        format!("QUESTION_JSON {}", question_json),
        format!(
            "COVERAGE tasks:{} collected,{} failed; checks:{} detected,{} clear,{} unknown,{} total",
            metadata.coverage.collected_tasks,
            metadata.coverage.failed_tasks,
            metadata.coverage.detected_checks,
            metadata.coverage.clear_checks,
            metadata.coverage.unknown_checks,
            metadata.coverage.total_checks,
        ),
    ];
    let mut scan = Vec::new();
    if let Some(scan_id) = metadata.scan_id {
        scan.push(format!("scan={}", clean_field(scan_id)));
    }
    if let Some(captured_at) = metadata.captured_at {
        scan.push(format!("captured={}", clean_field(captured_at)));
    }
    if let Some(age_minutes) = metadata.age_minutes {
        scan.push(format!("age_min={age_minutes}"));
    }
    if !scan.is_empty() {
        lines.insert(2, format!("META {}", scan.join(" ")));
    }
    for record in records {
        lines.push(render_record(record));
    }
    let mut omitted = vec![format!("records={}", omissions.records)];
    for (name, count) in [
        ("detected", omissions.detected),
        ("unknown", omissions.unknown),
        ("failed", omissions.failed),
        ("changes", omissions.changes),
        ("diagnostics", omissions.diagnostics),
    ] {
        if count > 0 {
            omitted.push(format!("{name}={count}"));
        }
    }
    lines.push(format!("OMITTED {}", omitted.join(" ")));
    lines.join("\n")
}

fn render_record(record: &EvidenceRecord) -> String {
    let sources = if record.source_tasks.is_empty() {
        "none".to_string()
    } else {
        record.source_tasks.join(",")
    };
    let classification = record
        .severity
        .as_deref()
        .map(|severity| {
            format!(
                "{}/{}/{}",
                kind_name(record.kind),
                status_name(record.status),
                clean_field(severity)
            )
        })
        .unwrap_or_else(|| format!("{}/{}", kind_name(record.kind), status_name(record.status)));
    format!(
        "{} {} id={} src={} title={} | {}",
        record.id,
        classification,
        clean_field(&record.source_id),
        clean_field(&sources),
        clean_field(&record.title),
        clean_field(&record.value)
    )
}

fn compact_diagnostic_output(
    output: &str,
    question_terms: &BTreeSet<String>,
    max_chars: usize,
) -> String {
    // Some diagnostics (event logs, installed programs) can be many MB. Do
    // not materialize a second full JSON tree just to produce a few hundred
    // prompt characters; those records still get a bounded raw excerpt.
    let parsed = (output.len() <= 512 * 1_024)
        .then(|| serde_json::from_str::<Value>(output.trim()).ok())
        .flatten();
    let Some(value) = parsed else {
        return compact_plain_text(output, max_chars);
    };
    let mut fields = Vec::new();
    collect_scalar_fields(&value, "$", 0, &mut fields);
    if fields.is_empty() {
        return compact_plain_text(output, max_chars);
    }
    fields.sort_by(|(left_path, left_value), (right_path, right_value)| {
        let left_text = format!("{} {}", left_path, left_value);
        let right_text = format!("{} {}", right_path, right_value);
        (
            Reverse(anomaly_score(&left_text)),
            Reverse(relevance_score(question_terms, &left_text)),
            Reverse(important_field_score(left_path)),
            left_path,
            left_value,
        )
            .cmp(&(
                Reverse(anomaly_score(&right_text)),
                Reverse(relevance_score(question_terms, &right_text)),
                Reverse(important_field_score(right_path)),
                right_path,
                right_value,
            ))
    });

    let total = fields.len();
    let mut included = Vec::new();
    for (path, value) in &fields {
        let fragment = compact_plain_text(&format!("{}={}", path, value), max_chars);
        let mut trial = included.clone();
        trial.push(fragment);
        let omitted = total.saturating_sub(trial.len());
        let rendered = render_field_fragments(&trial, omitted);
        if char_count(&rendered) <= max_chars {
            included = trial;
        }
    }
    if included.is_empty() {
        compact_plain_text("structured diagnostic data collected", max_chars)
    } else {
        render_field_fragments(&included, total.saturating_sub(included.len()))
    }
}

fn render_field_fragments(included: &[String], omitted: usize) -> String {
    let mut rendered = included.join("; ");
    if omitted > 0 {
        if !rendered.is_empty() {
            rendered.push_str("; ");
        }
        rendered.push_str(&format!("(+{} fields)", omitted));
    }
    rendered
}

fn collect_scalar_fields(value: &Value, path: &str, depth: usize, out: &mut Vec<(String, String)>) {
    if depth > 5 || out.len() >= 512 {
        return;
    }
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().take(24).enumerate() {
                collect_scalar_fields(item, &format!("{}[{}]", path, index), depth + 1, out);
                if out.len() >= 512 {
                    break;
                }
            }
        }
        Value::Object(map) => {
            // serde_json's default map may preserve insertion order depending
            // on features, so force lexical ordering here.
            let ordered: BTreeMap<_, _> = map.iter().collect();
            for (key, child) in ordered {
                let child_path = if path == "$" {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };
                collect_scalar_fields(child, &child_path, depth + 1, out);
                if out.len() >= 512 {
                    break;
                }
            }
        }
        Value::Null => out.push((path.to_string(), "null".to_string())),
        Value::Bool(value) => out.push((path.to_string(), value.to_string())),
        Value::Number(value) => out.push((path.to_string(), value.to_string())),
        Value::String(value) => out.push((path.to_string(), compact_plain_text(value, 160))),
    }
}

fn compact_plain_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut collapsed = String::with_capacity(max_chars.min(text.len()));
    let mut chars = 0usize;
    let mut pending_space = false;
    let mut truncated = false;
    for character in text.chars() {
        if character.is_whitespace() {
            pending_space = !collapsed.is_empty();
            continue;
        }
        if pending_space {
            collapsed.push(' ');
            chars += 1;
            pending_space = false;
        }
        collapsed.push(character);
        chars += 1;
        if chars > max_chars {
            truncated = true;
            break;
        }
    }
    if truncated {
        strict_truncate(&collapsed, max_chars, "…")
    } else {
        collapsed
    }
}

fn relevance_excerpt(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn strict_truncate(text: &str, max_chars: usize, marker: &str) -> String {
    if char_count(text) <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let marker_chars = char_count(marker);
    if marker_chars >= max_chars {
        return marker.chars().take(max_chars).collect();
    }
    let keep = max_chars - marker_chars;
    let mut shortened: String = text.chars().take(keep).collect();
    shortened.push_str(marker);
    shortened
}

fn clean_field(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "/")
}

fn query_terms(text: &str) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    for token in text
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|token| token.len() >= 3 && !is_stop_word(token))
    {
        terms.insert(token.clone());
        if let Some(synonyms) = synonyms_for(&token) {
            terms.extend(synonyms.iter().map(|synonym| synonym.to_string()));
        }
    }
    terms
}

fn relevance_score(question_terms: &BTreeSet<String>, text: &str) -> usize {
    if question_terms.is_empty() {
        return 0;
    }
    let text_terms = query_terms(text);
    question_terms.intersection(&text_terms).count()
}

fn synonyms_for(term: &str) -> Option<&'static [&'static str]> {
    match term {
        "disk" | "drive" | "storage" | "space" => Some(&["disk", "drive", "storage", "space"]),
        "memory" | "ram" => Some(&["memory", "ram"]),
        "cpu" | "processor" => Some(&["cpu", "processor"]),
        "network" | "wifi" | "internet" | "dns" => Some(&["network", "wifi", "internet", "dns"]),
        "crash" | "bsod" | "shutdown" => Some(&["crash", "bsod", "shutdown"]),
        "update" | "patch" | "hotfix" => Some(&["update", "patch", "hotfix"]),
        "driver" | "device" => Some(&["driver", "device"]),
        _ => None,
    }
}

fn is_stop_word(term: &str) -> bool {
    matches!(
        term,
        "the"
            | "and"
            | "for"
            | "that"
            | "this"
            | "with"
            | "from"
            | "what"
            | "why"
            | "how"
            | "are"
            | "was"
            | "were"
            | "has"
            | "have"
            | "about"
            | "please"
            | "windows"
            | "computer"
            | "system"
    )
}

fn anomaly_score(text: &str) -> usize {
    let lower = text.to_ascii_lowercase();
    [
        "critical",
        "failure",
        "failed",
        "error",
        "unhealthy",
        "degraded",
        "disabled",
        "warning",
        "unknown",
        "predicted",
        "corrupt",
    ]
    .iter()
    .filter(|needle| lower.contains(**needle))
    .count()
}

fn important_field_score(path: &str) -> usize {
    let lower = path.to_ascii_lowercase();
    [
        "status", "health", "error", "code", "free", "used", "percent", "version", "build",
        "model", "caption", "count", "state",
    ]
    .iter()
    .filter(|needle| lower.contains(**needle))
    .count()
}

fn kind_rank(kind: EvidenceKind) -> u8 {
    match kind {
        EvidenceKind::Issue => 0,
        EvidenceKind::Change => 1,
        EvidenceKind::Diagnostic => 2,
    }
}

fn kind_name(kind: EvidenceKind) -> &'static str {
    match kind {
        EvidenceKind::Issue => "issue",
        EvidenceKind::Diagnostic => "diagnostic",
        EvidenceKind::Change => "change",
    }
}

fn status_name(status: EvidenceStatus) -> &'static str {
    match status {
        EvidenceStatus::Detected => "detected",
        EvidenceStatus::Unknown => "unknown",
        EvidenceStatus::Failed => "failed",
        EvidenceStatus::Collected => "collected",
        EvidenceStatus::Changed => "changed",
        EvidenceStatus::Recovered => "recovered",
    }
}

fn severity_name(severity: IssueSeverity) -> &'static str {
    match severity {
        IssueSeverity::Critical => "critical",
        IssueSeverity::Warning => "warning",
        IssueSeverity::Info => "info",
        IssueSeverity::Ok => "ok",
    }
}

fn char_count(text: &str) -> usize {
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(success: bool, output: &str, error: Option<&str>) -> TaskResult {
        TaskResult {
            success,
            output: output.to_string(),
            error: error.map(str::to_string),
            duration_ms: 1,
        }
    }

    fn issue(id: &str, severity: IssueSeverity, status: IssueStatus, source: &str) -> Issue {
        Issue {
            id: id.to_string(),
            category: "Test".to_string(),
            severity,
            status,
            title: id.replace('_', " "),
            description: format!("description for {id}"),
            recommendation: "verify the evidence".to_string(),
            detected: status == IssueStatus::Detected,
            source_tasks: Some(vec![source.to_string()]),
            remediation: None,
        }
    }

    fn build(
        question: &str,
        results: &HashMap<String, TaskResult>,
        issues: &[Issue],
        max_chars: usize,
    ) -> Result<CompactEvidence, EvidenceBuildError> {
        build_compact_evidence(
            EvidenceRequest {
                question,
                scan_id: Some("scan-1"),
                captured_at: Some("2026-07-14T12:00:00Z"),
                age_minutes: Some(3),
                results,
                issues,
                comparison: None,
                preferred_source_ids: &[],
            },
            EvidencePolicy::compact(max_chars),
        )
    }

    #[test]
    fn preserves_exact_question_and_never_exceeds_budget() {
        let question = "Why is my disk full?\nDo not lose this second line: 🧭";
        let results = HashMap::from([(
            "logical_disk".to_string(),
            result(
                true,
                r#"[{"Drive":"C:","FreePercent":3,"Status":"Critical"}]"#,
                None,
            ),
        )]);
        let packet = build(question, &results, &[], 700).unwrap();
        assert_eq!(packet.packet.question, question);
        assert!(packet.rendered.chars().count() <= 700);
        assert!(packet.rendered.contains("Do not lose this second line"));
    }

    #[test]
    fn returns_error_instead_of_truncating_question_for_tiny_budget() {
        let question = "q".repeat(300);
        let error = build(&question, &HashMap::new(), &[], 100).unwrap_err();
        assert!(matches!(error, EvidenceBuildError::BudgetTooSmall { .. }));
    }

    #[test]
    fn rendering_is_deterministic_across_hashmap_insertion_order() {
        let first = HashMap::from([
            ("z_task".to_string(), result(true, r#"{"b":2}"#, None)),
            ("a_task".to_string(), result(false, "", Some("failed"))),
        ]);
        let mut second = HashMap::new();
        second.insert("a_task".to_string(), result(false, "", Some("failed")));
        second.insert("z_task".to_string(), result(true, r#"{"b":2}"#, None));
        let left = build("What failed?", &first, &[], 900).unwrap();
        let right = build("What failed?", &second, &[], 900).unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn reports_unknown_coverage_and_does_not_call_collection_health_ok() {
        let results = HashMap::from([(
            "logical_disk".to_string(),
            result(true, r#"{"FreePercent":55}"#, None),
        )]);
        let issues = vec![issue(
            "firewall_disabled",
            IssueSeverity::Info,
            IssueStatus::Unknown,
            "firewall_status",
        )];
        let packet = build("Summarize the scan", &results, &issues, 1000).unwrap();
        assert_eq!(packet.packet.coverage.unknown_checks, 1);
        assert!(packet.rendered.contains("1 unknown"));
        assert!(packet.rendered.contains("diagnostic/collected"));
        assert!(!packet.rendered.contains("/ok"));
    }

    #[test]
    fn explicit_source_precedes_critical_and_critical_precedes_warning() {
        let results = HashMap::new();
        let issues = vec![
            issue(
                "warning_issue",
                IssueSeverity::Warning,
                IssueStatus::Detected,
                "preferred_task",
            ),
            issue(
                "critical_issue",
                IssueSeverity::Critical,
                IssueStatus::Detected,
                "other_task",
            ),
            issue(
                "info_issue",
                IssueSeverity::Info,
                IssueStatus::Detected,
                "third_task",
            ),
        ];
        let preferred = vec!["preferred_task".to_string()];
        let packet = build_compact_evidence(
            EvidenceRequest {
                question: "Explain the scan",
                scan_id: None,
                captured_at: None,
                age_minutes: None,
                results: &results,
                issues: &issues,
                comparison: None,
                preferred_source_ids: &preferred,
            },
            EvidencePolicy {
                max_chars: 1600,
                max_record_chars: 200,
                max_records: 10,
                include_collected_tasks: false,
            },
        )
        .unwrap();
        let ids: Vec<_> = packet
            .packet
            .records
            .iter()
            .map(|record| record.source_id.as_str())
            .collect();
        assert_eq!(ids, vec!["warning_issue", "critical_issue", "info_issue"]);
    }

    #[test]
    fn whole_record_fitting_never_emits_partial_record_lines() {
        let results = HashMap::from([
            (
                "logical_disk".to_string(),
                result(true, &format!(r#"{{"data":"{}"}}"#, "x".repeat(800)), None),
            ),
            (
                "processor".to_string(),
                result(true, r#"{"Name":"CPU"}"#, None),
            ),
        ]);
        let packet = build("What hardware is present?", &results, &[], 510).unwrap();
        assert!(packet.rendered.chars().count() <= 510);
        for line in packet.rendered.lines().filter(|line| {
            line.starts_with('E') && line.chars().nth(1).is_some_and(|ch| ch.is_ascii_digit())
        }) {
            assert!(line.contains('/'));
            assert!(line.contains(" title="));
            assert!(line.contains(" | "));
        }
        assert!(
            packet
                .rendered
                .lines()
                .last()
                .unwrap()
                .starts_with("OMITTED ")
        );
    }

    #[test]
    fn phi_sized_budget_keeps_question_coverage_and_a_high_priority_record() {
        let results = HashMap::new();
        let issues = vec![issue(
            "low_disk_space",
            IssueSeverity::Critical,
            IssueStatus::Detected,
            "logical_disk",
        )];
        let packet = build_compact_evidence(
            EvidenceRequest {
                question: "Why is my disk full?",
                scan_id: Some("65b22cb8-8aac-4f8e-924d-9d65d1ff9e23"),
                captured_at: None,
                age_minutes: Some(2),
                results: &results,
                issues: &issues,
                comparison: None,
                preferred_source_ids: &[],
            },
            EvidencePolicy::compact(405),
        )
        .unwrap();
        assert!(packet.rendered.chars().count() <= 405);
        assert!(packet.rendered.contains("Why is my disk full?"));
        assert!(packet.rendered.contains("1 detected"));
        assert!(packet.rendered.contains("issue/detected/critical"));
        assert!(packet.rendered.contains("id=low_disk_space"));
    }

    #[test]
    fn strict_truncation_counts_unicode_characters() {
        let shortened = strict_truncate("🧭🧭🧭🧭", 3, "…");
        assert_eq!(shortened, "🧭🧭…");
        assert_eq!(shortened.chars().count(), 3);
    }

    #[test]
    fn output_compactor_prioritizes_anomalies_over_lexical_order() {
        let output = r#"{
            "aaa_model": "normal",
            "zzz_status": "Critical failure",
            "bbb_caption": "device"
        }"#;
        let compact = compact_diagnostic_output(output, &BTreeSet::new(), 54);
        assert!(compact.contains("zzz_status=Critical failure"));
    }
}
