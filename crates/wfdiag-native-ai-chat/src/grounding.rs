//! Framework-neutral, cancellable `WindowsForum` MCP grounding.
//!
//! This is the single implementation of the untrusted-input → search-query
//! boundary. Diagnostic output, issue evidence, and chat text are all
//! attacker-influenced data, so nothing reaches the network unless it comes
//! from [`SAFE_QUERY_FIELDS`] and survives the value rejection rules in
//! [`safe_value_term`]. Both shells (native Reactor and Tauri) call these
//! functions; neither keeps a private copy.
//!
//! The module also owns the minimal streamable-HTTP MCP client used for the
//! read-only RAG lookups. The app only needs `initialize`,
//! `notifications/initialized`, and `tools/call`, so a full MCP runtime is
//! deliberately not pulled into the diagnostics process.

use futures::StreamExt;
use reqwest::{StatusCode, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const WINDOWSFORUM_MCP_URL: &str = "https://mcp.windowsforum.com/";
const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const CLIENT_NAME: &str = "wfdiag";
/// MCP `clientInfo.version` and user agent version.
///
/// This must report the *application* version rather than this engine crate's
/// own `0.1.0`. `WFDIAG_APP_VERSION` is the version stamp the shells already
/// use (`apps/wfdiag/build.rs` emits it, and `scripts/check-version-sync.py`
/// keeps it in step with `version.json`); exporting it in the build
/// environment therefore propagates the product version here as well.
/// Documented fallback: when it is absent this crate's own package version is
/// reported instead of inventing a number.
const CLIENT_VERSION: &str = match option_env!("WFDIAG_APP_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};
const MAX_QUERY_CHARS: usize = 420;
const SOURCE_EXCERPT_CHARS: usize = 650;
/// Largest payload that may be parsed as JSON before query building falls back
/// to the bounded plain-text scan.
const MAX_GROUNDING_JSON_BYTES: usize = 256 * 1024;
/// Largest plain-text prefix scanned for allowlisted `key: value` lines.
const MAX_GROUNDING_PLAIN_SCAN_BYTES: usize = 128 * 1024;
/// Upper bound on KB identifiers collected from a JSON payload.
const MAX_GROUNDING_KB_IDS: usize = 8;

/// The only diagnostic field names whose values may leave the machine.
///
/// Everything else — serial numbers, user names, paths, addresses — is
/// dropped before a query is built.
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

/// One citable record kept for the UI trace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroundingTraceSource {
    pub source: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// What the grounding attempt did, for display beside an analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroundingTrace {
    pub enabled: bool,
    pub query: String,
    pub source_count: usize,
    pub sources: Vec<GroundingTraceSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Live RAG context for one-shot AI analysis plus its trace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisGrounding {
    pub prompt_context: Option<String>,
    pub trace: GroundingTrace,
}

/// Which MCP tools a query may use.
///
/// Release-channel and update-status claims must be grounded in current
/// Microsoft Support/KB material, so those modes never run the noisy forum
/// search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroundingMode {
    General,
    WindowsRelease,
    KnowledgeBase,
}

/// Why a prompt needs current network evidence. Keeping this decision pure
/// lets chat and one-shot analysis avoid a network request unless the user is
/// actually asking for a time-sensitive Windows fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroundingDemand {
    None,
    CurrentWindowsFact,
    KnowledgeBase,
}

/// A sanitized query and the tool routing it earned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundingQuery {
    pub text: String,
    pub mode: GroundingMode,
}

/// Live RAG context for one-shot AI analysis.
///
/// This deliberately does not use baked-in Windows release tables. Current
/// facts come through the `WindowsForum` MCP RAG endpoint on each request.
///
/// `supported` is the caller's analysis-kind gate (the Tauri shell derives it
/// from its `ContextType`) and `network_enabled` is the settings kill switch.
/// Returns `None` when this prompt does not need live evidence at all.
pub async fn analysis_grounding(
    supported: bool,
    network_enabled: bool,
    label: Option<&str>,
    data: &str,
    max_chars: usize,
) -> Option<AnalysisGrounding> {
    analysis_grounding_cancellable(
        supported,
        network_enabled,
        label,
        data,
        max_chars,
        &CancellationToken::new(),
    )
    .await
}

/// [`analysis_grounding`] for hosts that can cancel an in-flight request.
///
/// A cancelled search resolves to a trace carrying `Grounding request
/// cancelled`; callers that need a distinct cancellation outcome check their
/// own token afterwards.
pub async fn analysis_grounding_cancellable(
    supported: bool,
    network_enabled: bool,
    label: Option<&str>,
    data: &str,
    max_chars: usize,
    cancel: &CancellationToken,
) -> Option<AnalysisGrounding> {
    if !analysis_needs_live_grounding(supported, label, data) {
        return None;
    }
    if !network_enabled {
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
    Some(ground_query(&query, max_chars, cancel).await)
}

/// Run one already-sanitized query and render both prompt context and trace.
pub async fn ground_query(
    query: &GroundingQuery,
    max_chars: usize,
    cancel: &CancellationToken,
) -> AnalysisGrounding {
    let endpoint = windowsforum_endpoint();
    let lookup = search_sources(&endpoint, &query.text, query.mode);
    let searched = tokio::select! {
        biased;
        () = cancel.cancelled() => Err("Grounding request cancelled".to_string()),
        result = lookup => result,
    };
    match searched {
        Ok(sources) if !sources.is_empty() => AnalysisGrounding {
            prompt_context: Some(format_grounding(&query.text, &sources, max_chars)),
            trace: trace_from_sources(query.text.clone(), &sources, None),
        },
        Ok(_) => AnalysisGrounding {
            prompt_context: None,
            trace: GroundingTrace {
                enabled: true,
                query: query.text.clone(),
                source_count: 0,
                sources: Vec::new(),
                error: Some("No live grounding sources returned results".to_string()),
            },
        },
        Err(error) => AnalysisGrounding {
            prompt_context: None,
            trace: GroundingTrace {
                enabled: true,
                query: query.text.clone(),
                source_count: 0,
                sources: Vec::new(),
                error: Some(error),
            },
        },
    }
}

/// Search the read-only `WindowsForum` knowledge tools and return a bounded,
/// citable evidence packet. Dropping the selected request future on
/// cancellation aborts the underlying HTTP request.
///
/// Callers own the network kill switch: this function performs the request it
/// is asked for.
pub async fn search_windows_knowledge(
    query: &str,
    max_chars: usize,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let query = compact_text(query, MAX_QUERY_CHARS);
    if query.trim().is_empty() {
        return Err("search_windows_knowledge requires a query".to_string());
    }
    let endpoint = windowsforum_endpoint();
    let lookup = search_sources(&endpoint, &query, GroundingMode::General);
    let sources = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err("Grounding request cancelled".to_string()),
        result = lookup => result?,
    };
    if sources.is_empty() {
        return Err("No live grounding sources returned results".to_string());
    }
    Ok(format_grounding(&query, &sources, max_chars))
}

/// Build a compact, safe chat grounding query. User text is included because
/// it is already the user's AI prompt; diagnostic output is reduced through
/// the same allowlisted field path used by one-shot analysis.
#[must_use]
pub fn chat_grounding_query(user_query: &str, os_output: Option<&str>) -> String {
    let user_query = compact_text(user_query, 220);
    let Some(os_output) = os_output else {
        return compact_text(&format!("Windows {user_query}"), MAX_QUERY_CHARS);
    };
    let os_query = build_safe_query("Operating System", os_output).text;
    compact_text(&format!("{user_query} {os_query}"), MAX_QUERY_CHARS)
}

/// Return the network-grounding demand for the given text pieces.
///
/// A KB article id is an explicit request for external evidence. Other text
/// must combine a time-sensitive intent (for example "latest" or "still
/// supported") with a Windows release/update/driver subject. Generic words
/// such as "current" alone intentionally do not trigger web traffic for local
/// live metrics.
///
/// The pieces are scanned in place — a label and a multi-megabyte diagnostic
/// payload are never concatenated — while phrases spanning the boundary
/// between pieces are still recognized.
#[must_use]
pub fn grounding_demand(parts: &[&str]) -> GroundingDemand {
    const TIME_SENSITIVE_TERMS: &[&str] = &[
        "latest",
        "newest",
        "outdated",
        "supported",
        "unsupported",
        "preview",
        "insider",
        "pending",
    ];
    const WINDOWS_SUBJECT_TERMS: &[&str] = &[
        "windows", "build", "version", "update", "updates", "patch", "hotfix", "release",
        "support", "driver", "drivers", "insider", "preview", "channel",
    ];
    const PHRASE_WORDS: &[&str] = &[
        "up",
        "to",
        "date",
        "still",
        "supported",
        "end",
        "of",
        "support",
        "status",
        "known",
        "issue",
        "issues",
        "current",
        "build",
        "version",
        "release",
        "driver",
        "channel",
        "update",
        "available",
        "patch",
        "tuesday",
    ];

    let mut time_sensitive = false;
    let mut windows_subject = false;
    let mut previous_was_kb = false;
    let mut previous_word = "";
    let mut before_previous_word = "";

    for text in parts {
        for token in text
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
        {
            let inline_kb_digits = token
                .strip_prefix("KB")
                .or_else(|| token.strip_prefix("kb"))
                .filter(|digits| !digits.is_empty());
            let separated_kb_digits = previous_was_kb.then_some(token);
            if inline_kb_digits
                .or(separated_kb_digits)
                .is_some_and(valid_kb_digits)
            {
                return GroundingDemand::KnowledgeBase;
            }
            previous_was_kb = token.eq_ignore_ascii_case("kb");

            time_sensitive |= TIME_SENSITIVE_TERMS
                .iter()
                .any(|term| token.eq_ignore_ascii_case(term));
            windows_subject |= WINDOWS_SUBJECT_TERMS
                .iter()
                .any(|term| token.eq_ignore_ascii_case(term));

            let word = PHRASE_WORDS
                .iter()
                .copied()
                .find(|word| token.eq_ignore_ascii_case(word))
                .unwrap_or("");
            time_sensitive |= matches!(
                (before_previous_word, previous_word, word),
                ("up", "to", "date") | ("end", "of", "support")
            ) || matches!(
                (previous_word, word),
                ("still", "supported")
                    | ("support", "status")
                    | ("known", "issue" | "issues")
                    | ("current", "build" | "version" | "release" | "driver")
                    | ("release", "channel")
                    | ("update", "available")
                    | ("available", "update")
                    | ("patch", "tuesday")
            );
            before_previous_word = previous_word;
            previous_word = word;
        }
    }

    if time_sensitive && windows_subject {
        GroundingDemand::CurrentWindowsFact
    } else {
        GroundingDemand::None
    }
}

/// Whether the given text pieces need live evidence at all.
#[must_use]
pub fn needs_live_grounding(parts: &[&str]) -> bool {
    grounding_demand(parts) != GroundingDemand::None
}

/// Demand gate for one-shot diagnostic analysis. This deliberately evaluates
/// the label/data only after confirming the analysis kind can use live facts.
#[must_use]
pub fn analysis_needs_live_grounding(supported: bool, label: Option<&str>, data: &str) -> bool {
    supported && needs_live_grounding(&[label.unwrap_or_default(), data])
}

/// Reduce untrusted diagnostic content to an allowlisted, bounded query.
#[must_use]
pub fn build_safe_query(label: &str, data: &str) -> GroundingQuery {
    let mut parts = vec![format!("Windows {label}")];
    let trimmed = data.trim();
    // Parsing a multi-megabyte payload as JSON just to reject it is wasted
    // work on a UI-adjacent path; oversize input takes the bounded plain scan.
    if trimmed.len() <= MAX_GROUNDING_JSON_BYTES
        && let Ok(value) = serde_json::from_str::<Value>(trimmed)
    {
        if let Some(query) = build_windows_update_query(label, &value) {
            return query;
        }
        if let Some(query) = build_windows_release_query(label, &value) {
            return query;
        }
        collect_safe_terms(&value, &mut parts, 18);
        return GroundingQuery {
            text: compact_text(&parts.join(" "), MAX_QUERY_CHARS),
            mode: GroundingMode::General,
        };
    }
    collect_safe_plain_terms(
        utf8_prefix(data, MAX_GROUNDING_PLAIN_SCAN_BYTES),
        &mut parts,
        18,
    );
    GroundingQuery {
        text: compact_text(&parts.join(" "), MAX_QUERY_CHARS),
        mode: GroundingMode::General,
    }
}

/// Recover a citable trace from an already-rendered evidence packet.
///
/// [`trace_from_sources`] is the primary path and keeps untruncated titles and
/// URLs. This string-only fallback exists for hosts that hold nothing but the
/// rendered block (the chat pre-grounding packet), and never exposes the
/// diagnostic output or the provider prompt.
#[must_use]
pub fn trace_from_rendered_grounding(query: &str, rendered: &str) -> GroundingTrace {
    let mut sources = Vec::new();
    let mut omitted = 0;
    for line in rendered.lines() {
        if let Some(value) = line.strip_prefix("OMITTED sources=") {
            omitted = value.trim().parse::<usize>().unwrap_or_default();
            continue;
        }
        let Some((_, record)) = line.split_once(' ') else {
            continue;
        };
        if !line.starts_with('S') || !line.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
            continue;
        }
        let Some(source_end) = record.find(']') else {
            continue;
        };
        let source = record.get(1..source_end).unwrap_or_default().to_string();
        let details = record.get(source_end + 1..).unwrap_or_default().trim();
        let title = details.split(" | ").next().unwrap_or_default().to_string();
        let url = details
            .split(" | ")
            .find_map(|part| part.strip_prefix("URL "))
            .map(str::to_string);
        if !title.is_empty() {
            sources.push(GroundingTraceSource { source, title, url });
        }
    }
    GroundingTrace {
        enabled: true,
        query: query.to_string(),
        source_count: sources.len().saturating_add(omitted),
        sources,
        error: None,
    }
}

fn windowsforum_endpoint() -> String {
    std::env::var("WFDIAG_WINDOWSFORUM_MCP_URL")
        .unwrap_or_else(|_| WINDOWSFORUM_MCP_URL.to_string())
}

async fn search_sources(
    endpoint: &str,
    query: &str,
    mode: GroundingMode,
) -> Result<Vec<GroundingSource>, String> {
    let mut sources = Vec::new();
    if mode == GroundingMode::General {
        let forum = call_tool(
            endpoint,
            "search",
            json!({ "query": query, "source": "auto", "limit": 4 }),
        );
        let kb = call_tool(endpoint, "search_kb", json!({ "query": query, "limit": 8 }));
        let (forum, kb) = tokio::join!(forum, kb);
        if let Ok(result) = forum {
            sources.extend(extract_sources(
                "WindowsForum MCP search",
                &result,
                &["url", "contentUrl", "link"],
                &["text", "content", "excerpt", "message", "description"],
                4,
            ));
        }
        if let Ok(result) = kb {
            sources.extend(kb_proxy_sources(&result));
        }
    } else {
        // Release-channel and update-status claims must be grounded in current
        // Microsoft Support/KB material. Generic forum semantic results often
        // contain old Insider articles for nearby build numbers and are too
        // noisy for this decision, and a failed KB lookup must not be masked
        // by forum chatter.
        let result =
            call_tool(endpoint, "search_kb", json!({ "query": query, "limit": 8 })).await?;
        sources.extend(kb_proxy_sources(&result));
    }

    // Explicit Microsoft KB identifiers are high-signal and the MCP exposes a
    // dedicated article lookup. Search results alone can omit or rank down the
    // exact requested article.
    let kb_article_limit = if mode == GroundingMode::KnowledgeBase {
        4
    } else {
        2
    };
    for kb_id in kb_ids(query, kb_article_limit) {
        if let Ok(result) = call_tool(endpoint, "get_kb_article", json!({ "kb_id": kb_id })).await {
            sources.extend(extract_sources(
                "WindowsForum MCP KB article",
                &result,
                &["url", "contentUrl", "link"],
                &["content", "text", "excerpt", "description"],
                1,
            ));
        }
    }
    Ok(dedupe_sources(sources))
}

fn kb_proxy_sources(result: &Value) -> Vec<GroundingSource> {
    extract_sources(
        "WindowsForum MCP KB proxy",
        result,
        &["url", "contentUrl", "link"],
        &["content", "text", "excerpt", "description"],
        8,
    )
}

fn valid_kb_digits(digits: &str) -> bool {
    (6..=8).contains(&digits.len()) && digits.bytes().all(|character| character.is_ascii_digit())
}

fn kb_ids(query: &str, limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    let mut previous_was_kb = false;
    for token in query
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
    {
        let digits = token
            .strip_prefix("KB")
            .or_else(|| token.strip_prefix("kb"))
            .filter(|digits| !digits.is_empty())
            .or_else(|| previous_was_kb.then_some(token));
        if let Some(digits) = digits.filter(|digits| valid_kb_digits(digits)) {
            let id = format!("KB{digits}");
            if seen.insert(id.clone()) {
                ids.push(id);
                if ids.len() == limit {
                    break;
                }
            }
        }
        previous_was_kb = token.eq_ignore_ascii_case("kb");
    }
    ids
}

fn collect_json_kb_ids(value: &Value) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    collect_json_kb_ids_into(value, &mut seen, &mut ids, MAX_GROUNDING_KB_IDS);
    ids
}

fn collect_json_kb_ids_into(
    value: &Value,
    seen: &mut HashSet<String>,
    ids: &mut Vec<String>,
    limit: usize,
) {
    if ids.len() >= limit {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_json_kb_ids_into(item, seen, ids, limit);
                if ids.len() >= limit {
                    break;
                }
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                if key.eq_ignore_ascii_case("HotFixID")
                    && let Some(text) = primitive_to_query_text(value)
                {
                    for id in kb_ids(&text, limit.saturating_sub(ids.len())) {
                        if seen.insert(id.clone()) {
                            ids.push(id);
                        }
                    }
                }
                if matches!(value, Value::Array(_) | Value::Object(_)) {
                    collect_json_kb_ids_into(value, seen, ids, limit);
                }
                if ids.len() >= limit {
                    break;
                }
            }
        }
        _ => {}
    }
}

fn collect_safe_plain_terms(data: &str, out: &mut Vec<String>, limit: usize) {
    let remaining = limit.saturating_sub(out.len());
    for kb_id in kb_ids(data, remaining) {
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
        out.push(format!("{key} {value}"));
    }
}

fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
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
        parts.push(format!("Description {description}"));
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
        parts.push(format!("version {display_version}"));
    }
    if let Some(build) = build {
        parts.push(format!("OS Builds {build}"));
    }
    if let Some(full_build) = full_build {
        parts.push(format!("full build {full_build}"));
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

/// Reject a value even from an allowlisted field when it looks like personal
/// data: an address, a user profile path, or an unbounded blob.
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
    Some(format!("{key} {raw}"))
}

fn primitive_to_query_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
    .filter(|text| !text.is_empty())
}

struct McpHttpClient {
    endpoint: String,
    client: reqwest::Client,
    session_id: Option<String>,
    request_id: u64,
}

impl McpHttpClient {
    fn new(endpoint: &str) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(format!("{CLIENT_NAME}/{CLIENT_VERSION}"))
            .build()
            .map_err(|error| format!("MCP client init failed: {error}"))?;
        Ok(Self {
            endpoint: endpoint.to_string(),
            client,
            session_id: None,
            request_id: 1,
        })
    }

    async fn initialize(&mut self) -> Result<(), String> {
        let id = self.next_id();
        self.post(json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "id": id,
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": CLIENT_NAME, "version": CLIENT_VERSION},
            },
        }))
        .await?;
        // Some servers return 202 Accepted for this notification. It is a
        // protocol courtesy, not useful data, so only fail hard on transport.
        let _ = self
            .post(json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
                "params": {},
            }))
            .await;
        Ok(())
    }

    async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let id = self.next_id();
        self.post(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "id": id,
            "params": {"name": name, "arguments": arguments},
        }))
        .await
    }

    const fn next_id(&mut self) -> u64 {
        let id = self.request_id;
        self.request_id += 1;
        id
    }

    async fn post(&mut self, body: Value) -> Result<Value, String> {
        let mut request = self
            .client
            .post(&self.endpoint)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "text/event-stream, application/json")
            .json(&body);
        if let Some(session_id) = &self.session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("MCP request failed: {error}"))?;
        if let Some(session_id) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            self.session_id = Some(session_id.to_string());
        }
        parse_response(response).await
    }
}

async fn call_tool(endpoint: &str, name: &str, arguments: Value) -> Result<Value, String> {
    let mut client = McpHttpClient::new(endpoint)?;
    client.initialize().await?;
    client.call_tool(name, arguments).await
}

async fn parse_response(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    if status == StatusCode::ACCEPTED {
        return Ok(Value::Null);
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "MCP HTTP {status}: {}",
            body.chars().take(500).collect::<String>()
        ));
    }
    if content_type.contains("text/event-stream") {
        let mut stream = response.bytes_stream();
        let mut body = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| format!("MCP SSE read failed: {error}"))?;
            body.push_str(&String::from_utf8_lossy(&chunk));
        }
        parse_sse_response(&body)
    } else {
        let value = response
            .json::<Value>()
            .await
            .map_err(|error| format!("MCP JSON parse failed: {error}"))?;
        json_rpc_result(value)
    }
}

fn parse_sse_response(body: &str) -> Result<Value, String> {
    let mut last_result = None;
    let mut data_lines = Vec::new();
    for line in body.lines().chain(std::iter::once("")) {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if !data_lines.is_empty() {
                let payload = data_lines.join("\n");
                if let Ok(value) = serde_json::from_str::<Value>(&payload)
                    && value.get("method").is_none()
                {
                    last_result = Some(json_rpc_result(value)?);
                }
                data_lines.clear();
            }
        } else if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.trim_start().to_string());
        }
    }
    last_result.ok_or_else(|| "MCP SSE response did not include a result".to_string())
}

fn json_rpc_result(value: Value) -> Result<Value, String> {
    if let Some(error) = value.get("error") {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(|| error.to_string(), str::to_string));
    }
    let result = value.get("result").cloned().unwrap_or(value);
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let message = result
            .get("content")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find_map(|item| item.get("text").and_then(Value::as_str))
            })
            .map_or_else(|| result.to_string(), str::to_string);
        return Err(message);
    }
    Ok(result)
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
            structured.as_object().and_then(|object| {
                (object.contains_key("title") || object.contains_key("name"))
                    .then(|| vec![structured.clone()])
            })
        })
        .unwrap_or_default();
    candidates
        .iter()
        .take(limit)
        .filter_map(|item| {
            let title = first_string(item, &["title", "name", "thread_title"])?;
            Some(GroundingSource {
                source,
                title: compact_text(&title, 180),
                url: first_string(item, url_keys),
                excerpt: compact_text(
                    &first_string(item, excerpt_keys).unwrap_or_default(),
                    SOURCE_EXCERPT_CHARS,
                ),
            })
        })
        .collect()
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
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
        return crate::truncate_output(&minimal, max_chars);
    }
    let mut records = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let prefix = format!(
            "S{} [{}] {}",
            index + 1,
            compact_text(source.source, 32),
            compact_text(&source.title, 70)
        );
        let citation = source
            .url
            .as_deref()
            .map(|url| format!("{prefix} | URL {}", compact_text(url, 120)));
        let full = citation.as_ref().map(|citation| {
            if source.excerpt.is_empty() {
                citation.clone()
            } else {
                format!("{citation} | {}", compact_text(&source.excerpt, 120))
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

fn compact_text(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        collapsed
    } else {
        collapsed.chars().take(max_chars).collect()
    }
}

fn dedupe_sources(sources: Vec<GroundingSource>) -> Vec<GroundingSource> {
    let mut seen = HashSet::new();
    sources
        .into_iter()
        .filter(|source| {
            seen.insert(
                source
                    .url
                    .as_deref()
                    .unwrap_or(source.title.as_str())
                    .to_ascii_lowercase(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sse_json_rpc_result_and_ignores_notifications() {
        let body = "event: message\n\
                    data: {\"method\":\"notifications/message\",\"params\":{}}\n\
                    \n\
                    event: message\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"ok\":true}}\n\
                    \n";
        assert_eq!(parse_sse_response(body).unwrap(), json!({"ok": true}));
    }

    #[test]
    fn reports_json_rpc_error() {
        let value = json!({"jsonrpc": "2.0", "id": 1, "error": {"message": "bad tool"}});
        assert_eq!(json_rpc_result(value).unwrap_err(), "bad tool");
    }

    #[test]
    fn reports_mcp_tool_error_result() {
        let value = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "isError": true,
                "content": [{"type": "text", "text": "Unknown tool"}]
            }
        });
        assert_eq!(json_rpc_result(value).unwrap_err(), "Unknown tool");
    }

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
    fn safe_query_retains_release_evidence_and_drops_pii() {
        let query = build_safe_query(
            "Operating System",
            r#"[{"Caption":"Microsoft Windows 11 Pro","BuildNumber":"26200","DisplayVersion":"25H2","RegisteredUser":"person@example.com","SerialNumber":"private"}]"#,
        );
        assert!(query.text.contains("OS Builds 26200"));
        assert!(query.text.contains("version 25H2"));
        assert!(query.text.contains("Microsoft Support"));
        assert!(!query.text.contains("person@example.com"));
        assert!(!query.text.contains("private"));
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
        let ids = kb_ids("Windows KB5094126, kb5089549, and KB 5071234", 8);
        assert_eq!(ids.len(), 3);
        assert_eq!(ids, vec!["KB5094126", "KB5089549", "KB5071234"]);
        assert_eq!(
            kb_ids("Check KB5094126, kb 5089549, and KB5094126 again", 8),
            vec!["KB5094126", "KB5089549"]
        );
        assert!(kb_ids("serial 12345678", 8).is_empty());
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
    fn oversized_grounding_query_uses_bounded_streaming_kb_extraction() {
        let mut data = "HotFixID: KB5094126\n".to_string();
        data.push_str(&"x".repeat(MAX_GROUNDING_JSON_BYTES + 1));
        let query = build_safe_query("Windows Update History", &data);
        assert!(query.text.contains("KB5094126"));
        assert!(query.text.chars().count() <= MAX_QUERY_CHARS);
        assert_eq!(
            kb_ids("KB5094126 KB 5094218 KB5094301", 2),
            ["KB5094126".to_string(), "KB5094218".to_string()]
        );
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
            grounding_demand(&["Am I on the latest Windows build?"]),
            GroundingDemand::CurrentWindowsFact
        );
        assert_eq!(
            grounding_demand(&["Is KB5094126 related to this failure?"]),
            GroundingDemand::KnowledgeBase
        );
        assert_eq!(
            grounding_demand(&["Open KB 5094126"]),
            GroundingDemand::KnowledgeBase
        );
        assert!(needs_live_grounding(&[
            "Is this display driver still supported?"
        ]));
    }

    #[test]
    fn demand_gate_rejects_local_metrics_and_incidental_substrings() {
        assert_eq!(
            grounding_demand(&["What is my current memory usage?"]),
            GroundingDemand::None
        );
        assert_eq!(
            grounding_demand(&["Explain the RAM modules in this scan"]),
            GroundingDemand::None
        );
        assert_eq!(
            grounding_demand(&["The previewer process uses memory"]),
            GroundingDemand::None
        );
        assert_eq!(
            grounding_demand(&["My updater.exe process is slow"]),
            GroundingDemand::None
        );
        assert_eq!(
            grounding_demand(&["Device serial 12345678 was collected"]),
            GroundingDemand::None
        );
    }

    #[test]
    fn grounding_predicate_preserves_cross_piece_phrases_and_kb_ids() {
        assert!(needs_live_grounding(&["Current", "version Windows"]));
        assert!(needs_live_grounding(&["KB", "5094126"]));
        assert!(!needs_live_grounding(&[
            "Operating System",
            "Windows diagnostic collection completed",
        ]));
    }

    #[test]
    fn one_shot_grounding_is_demand_driven() {
        assert!(!analysis_needs_live_grounding(
            true,
            Some("Physical Memory"),
            r#"[{"Capacity": 34359738368}]"#,
        ));
        assert!(analysis_needs_live_grounding(
            true,
            Some("Operating System"),
            "Is Windows build 26200 still supported?",
        ));
        assert!(analysis_needs_live_grounding(
            true,
            Some("Windows Update History"),
            r#"{"HotFixID":"KB5094126"}"#,
        ));
        // An analysis kind that must never reach the network (the shells map
        // this from their own context type) is rejected before any scan.
        assert!(!analysis_needs_live_grounding(
            false,
            Some("Windows Update History"),
            r#"{"HotFixID":"KB5094126"}"#,
        ));
    }

    #[test]
    fn rendered_grounding_recovers_citable_trace() {
        let trace = trace_from_rendered_grounding(
            "Windows KB5094126",
            "LIVE WINDOWS EVIDENCE (WindowsForum MCP)\n\
             S1 [WindowsForum MCP KB proxy] Windows update history | URL https://support.microsoft.com/help/5094126 | Current build\n\
             OMITTED sources=2",
        );
        assert_eq!(trace.source_count, 3);
        assert_eq!(trace.sources.len(), 1);
        assert_eq!(trace.sources[0].source, "WindowsForum MCP KB proxy");
        assert_eq!(
            trace.sources[0].url.as_deref(),
            Some("https://support.microsoft.com/help/5094126")
        );
    }

    #[tokio::test]
    async fn disabled_network_and_undemanded_analysis_never_search() {
        assert!(
            analysis_grounding(true, true, Some("Physical Memory"), "{}", 1_200)
                .await
                .is_none()
        );
        let disabled = analysis_grounding(
            true,
            false,
            Some("Operating System"),
            "Is Windows build 26200 still supported?",
            1_200,
        )
        .await
        .expect("a demanded analysis reports why grounding was skipped");
        assert!(!disabled.trace.enabled);
        assert_eq!(
            disabled.trace.error.as_deref(),
            Some("Network grounding is disabled in Settings")
        );
        assert!(disabled.prompt_context.is_none());
    }

    #[tokio::test]
    async fn cancelled_grounding_reports_cancellation_without_a_request() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let grounding = ground_query(
            &GroundingQuery {
                text: "Windows 11 update history".to_string(),
                mode: GroundingMode::WindowsRelease,
            },
            1_200,
            &cancel,
        )
        .await;
        assert_eq!(
            grounding.trace.error.as_deref(),
            Some("Grounding request cancelled")
        );
        assert!(grounding.prompt_context.is_none());
    }

    #[tokio::test]
    async fn empty_chat_query_is_rejected_before_any_request() {
        let error = search_windows_knowledge("   ", 1_200, &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(error, "search_windows_knowledge requires a query");
    }
}
