use crate::ai_service::ContextType;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

const WINDOWSFORUM_MCP_URL: &str = "https://mcp.windowsforum.com/";
const MAX_QUERY_CHARS: usize = 420;
const SOURCE_EXCERPT_CHARS: usize = 650;
const SAFE_QUERY_FIELDS: &[&str] = &[
    "Caption",
    "ProductName",
    "DisplayVersion",
    "ReleaseId",
    "CurrentBuild",
    "CurrentBuildNumber",
    "BuildNumber",
    "Version",
    "UBR",
    "HotFixID",
    "InstalledOn",
    "EditionID",
    "InstallationType",
    "OSArchitecture",
    "Status",
    "SourceName",
    "EventCode",
    "LogFile",
    "Type",
    "Level",
    "source",
    "code",
    "driver_version",
    "DriverVersion",
    "DriverProviderName",
    "DeviceClass",
    "State",
    "StartMode",
    "Model",
    "Manufacturer",
];

#[derive(Debug, Clone)]
struct GroundingSource {
    source: &'static str,
    title: String,
    url: Option<String>,
    excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundingTraceSource {
    pub source: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundingTrace {
    pub enabled: bool,
    pub query: String,
    pub source_count: usize,
    pub sources: Vec<GroundingTraceSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AnalysisGrounding {
    pub prompt_context: Option<String>,
    pub trace: GroundingTrace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroundingMode {
    General,
    WindowsRelease,
    KnowledgeBase,
}

/// Why a prompt needs current network evidence. Keeping this decision pure
/// lets chat and one-shot analysis avoid a network request unless the user is
/// actually asking for a time-sensitive Windows fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroundingDemand {
    None,
    CurrentWindowsFact,
    KnowledgeBase,
}

#[derive(Debug, Clone)]
struct GroundingQuery {
    text: String,
    mode: GroundingMode,
}

/// Live RAG context for one-shot AI analysis.
///
/// This deliberately does not use baked-in Windows release tables. Current
/// facts come through the WindowsForum MCP RAG endpoint on each request.
pub async fn analysis_grounding(
    context_type: ContextType,
    label: Option<&str>,
    data: &str,
    max_chars: usize,
) -> Option<AnalysisGrounding> {
    if !analysis_needs_live_grounding(context_type, label, data) {
        return None;
    }
    if !grounding_enabled() {
        return Some(AnalysisGrounding {
            prompt_context: None,
            trace: GroundingTrace {
                enabled: false,
                query: String::new(),
                source_count: 0,
                sources: Vec::new(),
                error: Some("Network grounding is disabled in Settings".to_string()),
            },
        });
    }
    let query = build_safe_query(label.unwrap_or("Windows diagnostics"), data);
    if query.text.trim().is_empty() {
        return Some(AnalysisGrounding {
            prompt_context: None,
            trace: GroundingTrace {
                enabled: true,
                query: String::new(),
                source_count: 0,
                sources: Vec::new(),
                error: Some("No safe grounding query could be built".to_string()),
            },
        });
    }
    Some(search_analysis_grounding(query, max_chars).await)
}

/// Cancellable live RAG search used by chat. Dropping the MCP request future
/// on cancellation also aborts its underlying HTTP request.
pub async fn search_grounding_cancellable(
    query: &str,
    max_chars: usize,
    cancel: &CancellationToken,
) -> Result<String, String> {
    if !grounding_enabled() {
        return Err("Network grounding is disabled in Settings".to_string());
    }
    let query = compact_text(query, MAX_QUERY_CHARS);
    if query.trim().is_empty() {
        return Err("search_windows_knowledge requires a query".to_string());
    }

    let sources = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err("Grounding request cancelled".to_string()),
        result = search_sources(&query, GroundingMode::General) => result?,
    };
    if sources.is_empty() {
        return Err("No live grounding sources returned results".to_string());
    }
    Ok(format_grounding(&query, &sources, max_chars))
}

/// Build a compact, safe chat grounding query. User text is included because
/// it is already the user's AI prompt; diagnostic output is reduced through
/// the same allowlisted field path used by one-shot analysis.
pub fn chat_grounding_query(user_query: &str, os_output: Option<&str>) -> String {
    let user_query = compact_text(user_query, 220);
    let Some(os_output) = os_output else {
        return compact_text(&format!("Windows {}", user_query), MAX_QUERY_CHARS);
    };
    let os_query = build_safe_query("Operating System", os_output).text;
    compact_text(&format!("{} {}", user_query, os_query), MAX_QUERY_CHARS)
}

/// Return the network-grounding demand for user-authored text. A KB article id
/// is an explicit request for external evidence. Other text must combine a
/// time-sensitive intent (for example "latest" or "still supported") with a
/// Windows release/update/driver subject. Generic words such as "current"
/// alone intentionally do not trigger web traffic for local live metrics.
pub(crate) fn grounding_demand(text: &str) -> GroundingDemand {
    if !kb_ids(text).is_empty() {
        return GroundingDemand::KnowledgeBase;
    }

    let normalized = normalized_demand_text(text);
    let terms: HashSet<&str> = normalized.split_whitespace().collect();
    let time_sensitive = [
        "latest",
        "newest",
        "outdated",
        "supported",
        "unsupported",
        "preview",
        "insider",
        "pending",
    ]
    .iter()
    .any(|term| terms.contains(term))
        || contains_phrase(
            &normalized,
            &[
                "up to date",
                "still supported",
                "end of support",
                "support status",
                "known issue",
                "known issues",
                "current build",
                "current version",
                "current release",
                "current driver",
                "release channel",
                "update available",
                "available update",
                "patch tuesday",
            ],
        );
    let windows_subject = [
        "windows", "build", "version", "update", "updates", "patch", "hotfix", "release",
        "support", "driver", "drivers", "insider", "preview", "channel",
    ]
    .iter()
    .any(|term| terms.contains(term));

    if time_sensitive && windows_subject {
        GroundingDemand::CurrentWindowsFact
    } else {
        GroundingDemand::None
    }
}

pub(crate) fn needs_live_grounding(text: &str) -> bool {
    grounding_demand(text) != GroundingDemand::None
}

/// Demand gate for one-shot diagnostic analysis. This deliberately evaluates
/// the label/data only after confirming the analysis kind can use live facts.
pub(crate) fn analysis_needs_live_grounding(
    context_type: ContextType,
    label: Option<&str>,
    data: &str,
) -> bool {
    if !grounding_supported_for(context_type) {
        return false;
    }
    let combined = format!("{} {}", label.unwrap_or_default(), data);
    needs_live_grounding(&combined)
}

async fn search_analysis_grounding(query: GroundingQuery, max_chars: usize) -> AnalysisGrounding {
    match search_sources(&query.text, query.mode).await {
        Ok(sources) if !sources.is_empty() => AnalysisGrounding {
            prompt_context: Some(format_grounding(&query.text, &sources, max_chars)),
            trace: trace_from_sources(query.text, &sources, None),
        },
        Ok(_) => AnalysisGrounding {
            prompt_context: None,
            trace: GroundingTrace {
                enabled: true,
                query: query.text,
                source_count: 0,
                sources: Vec::new(),
                error: Some("No live grounding sources returned results".to_string()),
            },
        },
        Err(error) => AnalysisGrounding {
            prompt_context: None,
            trace: GroundingTrace {
                enabled: true,
                query: query.text,
                source_count: 0,
                sources: Vec::new(),
                error: Some(error),
            },
        },
    }
}

async fn search_sources(query: &str, mode: GroundingMode) -> Result<Vec<GroundingSource>, String> {
    let endpoint = windowsforum_endpoint();
    let mut sources = Vec::new();

    if mode == GroundingMode::General {
        let forum = search_windowsforum(&endpoint, query);
        let kb = search_windowsforum_kb(&endpoint, query);
        let (forum, kb) = tokio::join!(forum, kb);
        if let Ok(mut docs) = forum {
            sources.append(&mut docs);
        }
        if let Ok(mut docs) = kb {
            sources.append(&mut docs);
        }
    } else {
        // Release-channel and update-status claims must be grounded in current
        // Microsoft Support/KB material. Generic forum semantic results often
        // contain old Insider articles for nearby build numbers and are too
        // noisy for this decision.
        let mut docs = search_windowsforum_kb(&endpoint, query).await?;
        sources.append(&mut docs);
    }

    let kb_article_limit = if mode == GroundingMode::KnowledgeBase {
        4
    } else {
        2
    };
    for kb_id in kb_ids(query).into_iter().take(kb_article_limit) {
        if let Ok(mut docs) = get_windowsforum_kb_article(&endpoint, &kb_id).await {
            sources.append(&mut docs);
        }
    }

    Ok(dedupe_sources(sources))
}

fn windowsforum_endpoint() -> String {
    std::env::var("WFDIAG_WINDOWSFORUM_MCP_URL")
        .unwrap_or_else(|_| WINDOWSFORUM_MCP_URL.to_string())
}

fn grounding_enabled() -> bool {
    crate::commands::settings::network_grounding_enabled()
}

fn grounding_supported_for(context_type: ContextType) -> bool {
    matches!(
        context_type,
        ContextType::DiagnosticInterpretation
            | ContextType::SectionSummary
            | ContextType::IssuePrioritization
            | ContextType::GeneralAnalysis
    )
}

fn normalized_demand_text(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_phrase(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| {
        text == *phrase
            || text.starts_with(&format!("{} ", phrase))
            || text.ends_with(&format!(" {}", phrase))
            || text.contains(&format!(" {} ", phrase))
    })
}

async fn search_windowsforum(endpoint: &str, query: &str) -> Result<Vec<GroundingSource>, String> {
    let result = crate::mcp_client::call_tool(
        endpoint,
        "search",
        json!({ "query": query, "source": "auto", "limit": 4 }),
    )
    .await?;
    Ok(extract_sources(
        "WindowsForum MCP search",
        &result,
        &["url", "contentUrl", "link"],
        &["text", "content", "excerpt", "message", "description"],
        4,
    ))
}

async fn search_windowsforum_kb(
    endpoint: &str,
    query: &str,
) -> Result<Vec<GroundingSource>, String> {
    let result =
        crate::mcp_client::call_tool(endpoint, "search_kb", json!({ "query": query, "limit": 8 }))
            .await?;
    Ok(extract_sources(
        "WindowsForum MCP KB proxy",
        &result,
        &["url", "contentUrl", "link"],
        &["content", "text", "excerpt", "description"],
        8,
    ))
}

async fn get_windowsforum_kb_article(
    endpoint: &str,
    kb_id: &str,
) -> Result<Vec<GroundingSource>, String> {
    let result =
        crate::mcp_client::call_tool(endpoint, "get_kb_article", json!({ "kb_id": kb_id })).await?;
    Ok(extract_sources(
        "WindowsForum MCP KB article",
        &result,
        &["url", "contentUrl", "link"],
        &["content", "text", "excerpt", "description"],
        1,
    ))
}

fn extract_sources(
    source: &'static str,
    result: &Value,
    url_keys: &[&str],
    excerpt_keys: &[&str],
    limit: usize,
) -> Vec<GroundingSource> {
    let structured = result.get("structuredContent").unwrap_or(result);
    let candidates = structured
        .get("results")
        .or_else(|| structured.get("result"))
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| structured.as_array().cloned())
        .or_else(|| {
            structured.as_object().and_then(|obj| {
                (obj.contains_key("title") || obj.contains_key("name"))
                    .then(|| vec![structured.clone()])
            })
        })
        .unwrap_or_default();

    candidates
        .iter()
        .take(limit)
        .filter_map(|item| {
            let title = first_string(item, &["title", "name", "thread_title"])?;
            let excerpt = first_string(item, excerpt_keys).unwrap_or_default();
            let url = first_string(item, url_keys);
            Some(GroundingSource {
                source,
                title: compact_text(&title, 180),
                url,
                excerpt: compact_text(&excerpt, SOURCE_EXCERPT_CHARS),
            })
        })
        .collect()
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn format_grounding(query: &str, sources: &[GroundingSource], max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let query_budget = (max_chars / 6).clamp(40, 100);
    let base = vec![
        "LIVE WINDOWS EVIDENCE (WindowsForum MCP)".to_string(),
        "RULE: Cite title/URL. BuildNumber alone does not prove patch, support, or preview status."
            .to_string(),
        format!("QUERY: {}", compact_text(query, query_budget)),
    ];
    let render = |records: &[String], omitted: usize| {
        let mut lines = base.clone();
        lines.extend(records.iter().cloned());
        lines.push(format!("OMITTED sources={omitted}"));
        lines.join("\n")
    };

    let minimal = render(&[], sources.len());
    if minimal.chars().count() > max_chars {
        return crate::ai_prompts::truncate_output(&minimal, max_chars);
    }

    let mut records = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let title = compact_text(&source.title, 70);
        let source_name = compact_text(source.source, 32);
        let prefix = format!("S{} [{}] {}", index + 1, source_name, title);
        let citation = source
            .url
            .as_deref()
            .map(|url| format!("{} | URL {}", prefix, compact_text(url, 120)));
        let full = citation.as_ref().map(|citation| {
            if source.excerpt.is_empty() {
                citation.clone()
            } else {
                format!("{} | {}", citation, compact_text(&source.excerpt, 120))
            }
        });
        let variants = [full.as_deref(), citation.as_deref(), Some(prefix.as_str())];
        if let Some(variant) = variants.into_iter().flatten().find(|variant| {
            let mut trial = records.clone();
            trial.push((*variant).to_string());
            render(&trial, sources.len().saturating_sub(trial.len()))
                .chars()
                .count()
                <= max_chars
        }) {
            records.push(variant.to_string());
        }
    }

    let grounded = render(&records, sources.len().saturating_sub(records.len()));
    debug_assert!(grounded.chars().count() <= max_chars);
    grounded
}

fn trace_from_sources(
    query: String,
    sources: &[GroundingSource],
    error: Option<String>,
) -> GroundingTrace {
    GroundingTrace {
        enabled: true,
        query,
        source_count: sources.len(),
        sources: sources
            .iter()
            .take(8)
            .map(|source| GroundingTraceSource {
                source: source.source.to_string(),
                title: source.title.clone(),
                url: source.url.clone(),
            })
            .collect(),
        error,
    }
}

fn dedupe_sources(sources: Vec<GroundingSource>) -> Vec<GroundingSource> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for source in sources {
        let key = source
            .url
            .as_deref()
            .unwrap_or(source.title.as_str())
            .to_ascii_lowercase();
        if seen.insert(key) {
            deduped.push(source);
        }
    }
    deduped
}

fn build_safe_query(label: &str, data: &str) -> GroundingQuery {
    let mut parts = vec![format!("Windows {}", label)];
    if let Ok(value) = serde_json::from_str::<Value>(data.trim()) {
        if let Some(query) = build_windows_update_query(label, &value) {
            return query;
        }
        if let Some(query) = build_windows_release_query(label, &value) {
            return query;
        }
        collect_safe_terms(&value, &mut parts, 18);
    } else {
        collect_safe_plain_terms(data, &mut parts, 18);
    }
    GroundingQuery {
        text: compact_text(&parts.join(" "), MAX_QUERY_CHARS),
        mode: GroundingMode::General,
    }
}

fn collect_safe_plain_terms(data: &str, out: &mut Vec<String>, limit: usize) {
    for kb_id in kb_ids(data) {
        if out.len() >= limit {
            return;
        }
        out.push(kb_id);
    }
    for line in data.lines() {
        if out.len() >= limit {
            return;
        }
        let Some((key, value)) = line.split_once(':').or_else(|| line.split_once('=')) else {
            continue;
        };
        let key = key.trim();
        if !is_safe_query_field(key) {
            continue;
        }
        let value = value.trim();
        if value.is_empty()
            || value.contains('@')
            || value.contains("\\Users\\")
            || value.contains("/Users/")
            || value.len() > 160
        {
            continue;
        }
        out.push(format!("{} {}", key, value));
    }
}

fn build_windows_update_query(label: &str, value: &Value) -> Option<GroundingQuery> {
    let label_is_update = label.to_ascii_lowercase().contains("windows update");
    let ids = collect_json_kb_ids(value);
    if ids.is_empty() && !label_is_update {
        return None;
    }

    let mut parts = vec![
        "Windows installed updates hotfix history Microsoft Support".to_string(),
        ids.iter().take(8).cloned().collect::<Vec<_>>().join(" "),
    ];
    if let Some(description) = find_string_field(value, "Description") {
        parts.push(format!("Description {}", description));
    }

    Some(GroundingQuery {
        text: compact_text(&parts.join(" "), MAX_QUERY_CHARS),
        mode: GroundingMode::KnowledgeBase,
    })
}

fn build_windows_release_query(label: &str, value: &Value) -> Option<GroundingQuery> {
    let label_is_os = label.to_ascii_lowercase().contains("operating system");
    let caption = find_string_field(value, "Caption")
        .or_else(|| find_string_field(value, "ProductName"))
        .unwrap_or_default();
    let build = find_string_field(value, "CurrentBuild")
        .or_else(|| find_string_field(value, "CurrentBuildNumber"))
        .or_else(|| find_string_field(value, "BuildNumber"));
    let display_version = find_string_field(value, "DisplayVersion");
    let full_build =
        find_string_field(value, "FullBuild").or_else(|| find_string_field(value, "Version"));
    let is_windows_os = caption.to_ascii_lowercase().contains("windows") && build.is_some();
    if !label_is_os && !is_windows_os {
        return None;
    }

    let mut parts = vec!["Windows".to_string()];
    if caption.to_ascii_lowercase().contains("windows 11") {
        parts.push("11".to_string());
    }
    parts.push("update history release information Microsoft Support".to_string());
    if let Some(display_version) = display_version {
        parts.push(format!("version {}", display_version));
    }
    if let Some(build) = build {
        parts.push(format!("OS Builds {}", build));
    }
    if let Some(full_build) = full_build {
        parts.push(format!("full build {}", full_build));
    }

    Some(GroundingQuery {
        text: compact_text(&parts.join(" "), MAX_QUERY_CHARS),
        mode: GroundingMode::WindowsRelease,
    })
}

fn find_string_field(value: &Value, wanted: &str) -> Option<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| find_string_field(item, wanted)),
        Value::Object(map) => {
            for (key, value) in map {
                if key.eq_ignore_ascii_case(wanted)
                    && let Some(text) = primitive_to_query_text(value)
                {
                    return Some(text);
                }
                if matches!(value, Value::Array(_) | Value::Object(_))
                    && let Some(text) = find_string_field(value, wanted)
                {
                    return Some(text);
                }
            }
            None
        }
        _ => None,
    }
}

fn collect_safe_terms(value: &Value, out: &mut Vec<String>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items.iter().take(8) {
                collect_safe_terms(item, out, limit);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                if out.len() >= limit {
                    break;
                }
                if is_safe_query_field(key)
                    && let Some(term) = safe_value_term(key, value)
                {
                    out.push(term);
                }
                if matches!(value, Value::Array(_) | Value::Object(_)) {
                    collect_safe_terms(value, out, limit);
                }
            }
        }
        _ => {}
    }
}

fn is_safe_query_field(key: &str) -> bool {
    SAFE_QUERY_FIELDS
        .iter()
        .any(|allowed| key.eq_ignore_ascii_case(allowed))
}

fn safe_value_term(key: &str, value: &Value) -> Option<String> {
    let raw = primitive_to_query_text(value)?;
    if raw.is_empty()
        || raw.contains('@')
        || raw.contains("\\Users\\")
        || raw.contains("/Users/")
        || raw.len() > 160
    {
        return None;
    }
    Some(format!("{} {}", key, raw))
}

fn primitive_to_query_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}

fn compact_text(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        collapsed
    } else {
        collapsed.chars().take(max_chars).collect()
    }
}

fn kb_ids(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    let tokens: Vec<_> = query
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    for (index, token) in tokens.iter().enumerate() {
        let digits = token
            .strip_prefix("KB")
            .or_else(|| token.strip_prefix("kb"))
            .filter(|digits| !digits.is_empty())
            .or_else(|| {
                token
                    .eq_ignore_ascii_case("kb")
                    .then(|| tokens.get(index + 1).copied())
                    .flatten()
            });
        let Some(digits) = digits else { continue };
        if digits.len() >= 6 && digits.len() <= 8 && digits.chars().all(|ch| ch.is_ascii_digit()) {
            let id = format!("KB{}", digits);
            if seen.insert(id.clone()) {
                ids.push(id);
            }
        }
    }
    ids
}

fn collect_json_kb_ids(value: &Value) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    collect_json_kb_ids_into(value, &mut seen, &mut ids);
    ids
}

fn collect_json_kb_ids_into(value: &Value, seen: &mut HashSet<String>, ids: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_json_kb_ids_into(item, seen, ids);
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                if key.eq_ignore_ascii_case("HotFixID")
                    && let Some(text) = primitive_to_query_text(value)
                {
                    for id in kb_ids(&text) {
                        if seen.insert(id.clone()) {
                            ids.push(id);
                        }
                    }
                }
                if matches!(value, Value::Array(_) | Value::Object(_)) {
                    collect_json_kb_ids_into(value, seen, ids);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_query_keeps_build_fields_but_drops_pii() {
        let data = r#"[{
            "Caption": "Microsoft Windows 11 Pro",
            "BuildNumber": "26200",
            "DisplayVersion": "26H1",
            "RegisteredUser": "person@example.com",
            "SerialNumber": "00330-50000-00000-AAOEM",
            "SystemDirectory": "C:\\WINDOWS\\system32"
        }]"#;
        let query = build_safe_query("Operating System", data);
        assert_eq!(query.mode, GroundingMode::WindowsRelease);
        assert!(query.text.contains("OS Builds 26200"));
        assert!(query.text.contains("version 26H1"));
        assert!(query.text.contains("Microsoft Support"));
        assert!(!query.text.contains("person@example.com"));
        assert!(!query.text.contains("00330"));
        assert!(!query.text.contains("system32"));
    }

    #[test]
    fn safe_plain_query_uses_allowlisted_fields_only() {
        let data = "Caption: Microsoft Windows 11 Pro\nBuildNumber: 26200\nRegisteredUser: person@example.com\nMACAddress: 00-11-22-33-44-55\nIPAddress: 203.0.113.42\nSystemDirectory: C:\\WINDOWS\\system32";
        let query = build_safe_query("Operating System", data);
        assert!(query.text.contains("Caption Microsoft Windows 11 Pro"));
        assert!(query.text.contains("BuildNumber 26200"));
        assert!(!query.text.contains("person@example.com"));
        assert!(!query.text.contains("00-11-22"));
        assert!(!query.text.contains("203.0.113"));
        assert!(!query.text.contains("system32"));
    }

    #[test]
    fn extracts_structured_sources() {
        let result = json!({
            "structuredContent": {
                "results": [{
                    "title": "Windows 11 release information",
                    "url": "https://support.microsoft.com/help/5094126",
                    "content": "Version 25H2 current release data"
                }]
            }
        });
        let sources = extract_sources(
            "WindowsForum MCP KB proxy",
            &result,
            &["url"],
            &["content"],
            3,
        );
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source, "WindowsForum MCP KB proxy");
        assert!(sources[0].excerpt.contains("25H2"));
    }

    #[test]
    fn compact_grounding_budget_keeps_a_citable_source_record() {
        let sources = vec![GroundingSource {
            source: "WindowsForum MCP KB proxy",
            title: "Windows 11 release information".to_string(),
            url: Some("https://support.microsoft.com/help/5094126".to_string()),
            excerpt: "Current Microsoft support and OS build details.".repeat(8),
        }];
        let rendered = format_grounding("Windows 11 latest build support status", &sources, 405);
        assert!(rendered.chars().count() <= 405);
        assert!(rendered.contains("Windows 11 release information"));
        assert!(rendered.contains("https://support.microsoft.com/help/5094126"));
        assert!(rendered.contains("OMITTED sources=0"));
    }

    #[test]
    fn extracts_kb_ids() {
        let ids = kb_ids("Windows KB5094126, kb5089549, and KB 5071234");
        assert_eq!(ids.len(), 3);
        assert_eq!(ids, vec!["KB5094126", "KB5089549", "KB5071234"]);
        assert!(kb_ids("serial 12345678").is_empty());
    }

    #[test]
    fn windows_update_query_prefers_hotfix_ids_over_caption_urls() {
        let data = r#"{
            "installed_updates": [
                {
                    "Caption": "http://support.microsoft.com/?kbid=5092427",
                    "Description": "Update",
                    "HotFixID": "KB5092427",
                    "InstalledOn": "5/27/2026"
                },
                {
                    "Caption": "https://support.microsoft.com/help/5094126",
                    "Description": "Security Update",
                    "HotFixID": "KB5094126",
                    "InstalledOn": "6/9/2026"
                }
            ]
        }"#;
        let query = build_safe_query("Windows Update History", data);
        assert_eq!(query.mode, GroundingMode::KnowledgeBase);
        assert!(query.text.contains("KB5092427"));
        assert!(query.text.contains("KB5094126"));
        assert!(!query.text.contains("support.microsoft.com"));
        assert!(!query.text.contains("Caption"));
    }

    #[test]
    fn chat_grounding_query_sanitizes_os_output() {
        let data = r#"[{
            "Caption": "Microsoft Windows 11 Pro",
            "BuildNumber": "26200",
            "DisplayVersion": "26H1",
            "RegisteredUser": "person@example.com",
            "SerialNumber": "00330-50000-00000-AAOEM"
        }]"#;
        let query = chat_grounding_query("am I on the latest build?", Some(data));
        assert!(query.contains("latest build"));
        assert!(query.contains("OS Builds 26200"));
        assert!(query.contains("version 26H1"));
        assert!(!query.contains("person@example.com"));
        assert!(!query.contains("00330"));
    }

    #[test]
    fn demand_gate_accepts_current_windows_facts_and_kb_ids() {
        assert_eq!(
            grounding_demand("Am I on the latest Windows build?"),
            GroundingDemand::CurrentWindowsFact
        );
        assert_eq!(
            grounding_demand("Is KB5094126 related to this failure?"),
            GroundingDemand::KnowledgeBase
        );
        assert_eq!(
            grounding_demand("Open KB 5094126"),
            GroundingDemand::KnowledgeBase
        );
        assert!(needs_live_grounding(
            "Is this display driver still supported?"
        ));
    }

    #[test]
    fn demand_gate_rejects_local_metrics_and_incidental_substrings() {
        assert_eq!(
            grounding_demand("What is my current memory usage?"),
            GroundingDemand::None
        );
        assert_eq!(
            grounding_demand("Explain the RAM modules in this scan"),
            GroundingDemand::None
        );
        assert_eq!(
            grounding_demand("The previewer process uses memory"),
            GroundingDemand::None
        );
        assert_eq!(
            grounding_demand("My updater.exe process is slow"),
            GroundingDemand::None
        );
        assert_eq!(
            grounding_demand("Device serial 12345678 was collected"),
            GroundingDemand::None
        );
    }

    #[test]
    fn one_shot_grounding_is_demand_driven() {
        assert!(!analysis_needs_live_grounding(
            ContextType::DiagnosticInterpretation,
            Some("Physical Memory"),
            r#"[{"Capacity": 34359738368}]"#,
        ));
        assert!(analysis_needs_live_grounding(
            ContextType::DiagnosticInterpretation,
            Some("Operating System"),
            "Is Windows build 26200 still supported?",
        ));
        assert!(analysis_needs_live_grounding(
            ContextType::SectionSummary,
            Some("Windows Update History"),
            r#"{"HotFixID":"KB5094126"}"#,
        ));
    }
}
