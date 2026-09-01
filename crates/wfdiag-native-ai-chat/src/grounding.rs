//! Framework-neutral, cancellable `WindowsForum` MCP grounding used by chat.

use futures::StreamExt;
use reqwest::{StatusCode, header};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const WINDOWSFORUM_MCP_URL: &str = "https://mcp.windowsforum.com/";
const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const MAX_QUERY_CHARS: usize = 420;
const SOURCE_EXCERPT_CHARS: usize = 650;

#[derive(Debug, Clone)]
struct GroundingSource {
    source: &'static str,
    title: String,
    url: Option<String>,
    excerpt: String,
}

/// Search the read-only `WindowsForum` knowledge tools and return a bounded,
/// citable evidence packet. Dropping the selected request future on
/// cancellation aborts the underlying HTTP request.
pub async fn search_windows_knowledge(
    query: &str,
    max_chars: usize,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let query = compact_text(query, MAX_QUERY_CHARS);
    if query.trim().is_empty() {
        return Err("search_windows_knowledge requires a query".to_string());
    }
    let endpoint = std::env::var("WFDIAG_WINDOWSFORUM_MCP_URL")
        .unwrap_or_else(|_| WINDOWSFORUM_MCP_URL.to_string());
    let lookup = search_sources(&endpoint, &query);
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

async fn search_sources(endpoint: &str, query: &str) -> Result<Vec<GroundingSource>, String> {
    let forum = call_tool(
        endpoint,
        "search",
        json!({ "query": query, "source": "auto", "limit": 4 }),
    );
    let kb = call_tool(endpoint, "search_kb", json!({ "query": query, "limit": 8 }));
    let (forum, kb) = tokio::join!(forum, kb);
    let mut sources = Vec::new();
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
        sources.extend(extract_sources(
            "WindowsForum MCP KB proxy",
            &result,
            &["url", "contentUrl", "link"],
            &["content", "text", "excerpt", "description"],
            8,
        ));
    }
    // Explicit Microsoft KB identifiers are high-signal and the MCP exposes
    // a dedicated article lookup. Search results alone can omit or rank down
    // the exact requested article, so retain the shipping Tauri path here.
    for kb_id in kb_ids(query).into_iter().take(2) {
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
    if sources.is_empty() {
        return Err("WindowsForum MCP searches did not return usable results".to_string());
    }
    Ok(dedupe_sources(sources))
}

fn kb_ids(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    let tokens = query
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
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
        if (6..=8).contains(&digits.len())
            && digits.chars().all(|character| character.is_ascii_digit())
        {
            let id = format!("KB{digits}");
            if seen.insert(id.clone()) {
                ids.push(id);
            }
        }
    }
    ids
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
            .user_agent(format!("wfdiag/{}", env!("CARGO_PKG_VERSION")))
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
                "clientInfo": {"name": "wfdiag", "version": env!("CARGO_PKG_VERSION")},
            },
        }))
        .await?;
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
    render(&records, sources.len().saturating_sub(records.len()))
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
    fn grounding_budget_retains_a_citable_record() {
        let sources = vec![GroundingSource {
            source: "WindowsForum MCP KB proxy",
            title: "Windows 11 release information".to_string(),
            url: Some("https://support.microsoft.com/help/5094126".to_string()),
            excerpt: "Current Microsoft support and OS build details.".repeat(8),
        }];
        let rendered = format_grounding("Windows 11 latest build", &sources, 405);
        assert!(rendered.chars().count() <= 405);
        assert!(rendered.contains("https://support.microsoft.com/help/5094126"));
    }

    #[test]
    fn extracts_exact_kb_ids_for_article_lookup() {
        assert_eq!(
            kb_ids("Check KB5094126, kb 5089549, and KB5094126 again"),
            vec!["KB5094126", "KB5089549"]
        );
        assert!(kb_ids("serial 12345678").is_empty());
    }
}
