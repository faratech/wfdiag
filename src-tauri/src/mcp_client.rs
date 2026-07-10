use futures::StreamExt;
use reqwest::{StatusCode, header};
use serde_json::{Value, json};
use std::time::Duration;

const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const CLIENT_NAME: &str = "wfdiag";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Minimal streamable-HTTP MCP client for read-only RAG lookups.
///
/// The app only needs `initialize`, `notifications/initialized`, and
/// `tools/call`; keeping this small avoids pulling a full MCP runtime into the
/// diagnostics process.
pub struct McpHttpClient {
    endpoint: String,
    client: reqwest::Client,
    session_id: Option<String>,
    request_id: u64,
}

impl McpHttpClient {
    pub fn new(endpoint: impl Into<String>) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(format!("{}/{}", CLIENT_NAME, CLIENT_VERSION))
            .build()
            .map_err(|e| format!("MCP client init failed: {}", e))?;
        Ok(Self {
            endpoint: endpoint.into(),
            client,
            session_id: None,
            request_id: 1,
        })
    }

    pub async fn initialize(&mut self) -> Result<(), String> {
        let id = self.next_id();
        self.post(json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "id": id,
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": CLIENT_NAME,
                    "version": CLIENT_VERSION,
                },
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

    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let id = self.next_id();
        self.post(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "id": id,
            "params": {
                "name": name,
                "arguments": arguments,
            },
        }))
        .await
    }

    fn next_id(&mut self) -> u64 {
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
            .map_err(|e| format!("MCP request failed: {}", e))?;
        if let Some(session_id) = response
            .headers()
            .get("mcp-session-id")
            .or_else(|| response.headers().get("Mcp-Session-Id"))
            .and_then(|v| v.to_str().ok())
        {
            self.session_id = Some(session_id.to_string());
        }

        parse_response(response).await
    }
}

pub async fn call_tool(endpoint: &str, name: &str, arguments: Value) -> Result<Value, String> {
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
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "MCP HTTP {}: {}",
            status,
            body.chars().take(500).collect::<String>()
        ));
    }

    if content_type.contains("text/event-stream") {
        let mut stream = response.bytes_stream();
        let mut body = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("MCP SSE read failed: {}", e))?;
            body.push_str(&String::from_utf8_lossy(&chunk));
        }
        parse_sse_response(&body)
    } else {
        let value: Value = response
            .json()
            .await
            .map_err(|e| format!("MCP JSON parse failed: {}", e))?;
        json_rpc_result(value)
    }
}

fn parse_sse_response(body: &str) -> Result<Value, String> {
    let mut last_result = None;
    let mut data_lines = Vec::new();

    for line in body.lines().chain(std::iter::once("")) {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.is_empty() {
            if !data_lines.is_empty() {
                let payload = data_lines.join("\n");
                if let Ok(value) = serde_json::from_str::<Value>(&payload)
                    && value.get("method").is_none()
                {
                    last_result = Some(json_rpc_result(value)?);
                }
                data_lines.clear();
            }
            continue;
        }
        if let Some(data) = trimmed.strip_prefix("data:") {
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
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string()));
    }
    let result = value.get("result").cloned().unwrap_or(value);
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(mcp_result_text(&result).unwrap_or_else(|| result.to_string()));
    }
    Ok(result)
}

fn mcp_result_text(result: &Value) -> Option<String> {
    result
        .get("content")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|item| item.get("text").and_then(Value::as_str))
        .map(str::to_string)
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
        let parsed = parse_sse_response(body).unwrap();
        assert_eq!(parsed, json!({"ok": true}));
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
}
