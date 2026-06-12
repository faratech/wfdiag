use anyhow::Result;
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::responses::{
        CreateResponseArgs, FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolArgs,
        InputItem, InputParam, Item, OutputItem, Tool,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::error::DiagError;

/// AI Provider options
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum AiProvider {
    #[default]
    OpenAI,
    PhiSilica, // On-device WinRT path (phi_silica.rs) or a local OpenAI-compatible server
}

/// OpenAI model to use for all API calls (when no local provider is available)
/// Change this constant to switch models globally
pub const OPENAI_MODEL: &str = "gpt-5-nano";

/// Default model alias requested from Foundry Local. Phi Silica itself is NOT
/// served by Foundry Local (it is only reachable through the Windows AI APIs,
/// which require package identity); phi-4-mini is Microsoft's documented
/// local fallback model.
pub(crate) const FOUNDRY_LOCAL_MODEL: &str = "phi-4-mini";

/// Extract `scheme://host:port` from the first http(s) URL found in `text`.
fn extract_http_base(text: &str) -> Option<String> {
    let start = text.find("http://").or_else(|| text.find("https://"))?;
    let url = &text[start..];
    let end = url
        .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
        .unwrap_or(url.len());
    let url = &url[..end];
    let scheme_end = url.find("://")? + 3;
    if url.len() <= scheme_end {
        return None;
    }
    // Cut any path component, keeping scheme://authority only
    let base_end = url[scheme_end..]
        .find('/')
        .map(|i| scheme_end + i)
        .unwrap_or(url.len());
    Some(url[..base_end].to_string())
}

/// TCP probe of an endpoint base URL (`scheme://host:port`).
fn probe_endpoint(base: &str) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let Some(scheme_end) = base.find("://") else {
        return false;
    };
    let host_port = &base[scheme_end + 3..];
    let host_port = if host_port.contains(':') {
        host_port.to_string()
    } else {
        format!("{}:80", host_port)
    };
    match host_port.to_socket_addrs() {
        Ok(mut addrs) => {
            addrs.any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok())
        }
        Err(_) => false,
    }
}

async fn probe_endpoint_async(base: &str) -> bool {
    let base = base.to_string();
    tokio::task::spawn_blocking(move || probe_endpoint(&base))
        .await
        .unwrap_or(false)
}

/// Read the user-configured local AI endpoint from settings, normalized to a
/// base URL without a trailing slash or `/v1` suffix.
fn configured_local_endpoint() -> Option<String> {
    let path = crate::commands::settings::get_settings_path().ok()?;
    let content = std::fs::read_to_string(path).ok()?;
    let settings: crate::commands::settings::AppSettings = serde_json::from_str(&content).ok()?;
    settings
        .local_ai_endpoint
        .map(|e| {
            let e = e.trim().trim_end_matches('/');
            e.strip_suffix("/v1").unwrap_or(e).to_string()
        })
        .filter(|e| !e.is_empty())
}

/// Ask the Foundry Local CLI where its service is listening. The service port
/// is dynamic by design — Microsoft documents that it must never be hardcoded,
/// so discovery goes through `foundry service status`.
async fn discover_foundry_endpoint() -> Option<String> {
    let mut cmd = tokio::process::Command::new("foundry");
    cmd.args(["service", "status"]);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;
    extract_http_base(&String::from_utf8_lossy(&output.stdout))
}

/// Resolve a reachable local OpenAI-compatible endpoint: an explicit setting
/// wins, otherwise Foundry Local is discovered via its CLI. Returns the base
/// URL (append `/v1` for the OpenAI-compatible API root).
pub(crate) async fn local_ai_endpoint() -> Option<String> {
    if let Some(endpoint) = configured_local_endpoint()
        && probe_endpoint_async(&endpoint).await
    {
        return Some(endpoint);
    }
    let endpoint = discover_foundry_endpoint().await?;
    if probe_endpoint_async(&endpoint).await {
        Some(endpoint)
    } else {
        None
    }
}

/// Response for AI provider availability check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderStatus {
    pub openai_available: bool, // Always true (user provides API key)
    pub phi_silica_available: bool,
    pub phi_silica_info: Option<String>,
}

/// Tauri command to check AI provider availability
#[tauri::command]
pub async fn get_ai_provider_status() -> Result<AiProviderStatus, String> {
    let endpoint = local_ai_endpoint().await;

    Ok(AiProviderStatus {
        openai_available: true,
        phi_silica_available: endpoint.is_some(),
        phi_silica_info: match &endpoint {
            Some(endpoint) => Some(format!(
                "Local AI available via Foundry Local at {} (model: {})",
                endpoint, FOUNDRY_LOCAL_MODEL
            )),
            None => Some(
                "No local AI endpoint found. Install Foundry Local (winget install \
                 Microsoft.FoundryLocal) and run 'foundry service start', or set a custom \
                 endpoint in Settings. On-device Phi Silica uses the Windows AI APIs instead \
                 and requires package identity (MSIX or sparse package)."
                    .to_string(),
            ),
        },
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIRequest {
    pub api_key: String,
    pub prompt: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIResponse {
    pub analysis: String,
    pub diagnostics_run: Vec<String>,
    pub findings: Vec<Finding>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub category: String,
    pub severity: String,
    pub description: String,
    pub details: Option<String>,
}

/// Simple text analysis using Responses API
pub async fn analyze_text(
    api_key: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let config = OpenAIConfig::new().with_api_key(api_key);
    let client = Client::with_config(config);

    let full_prompt = format!("{}\n\nUser: {}", system_prompt, user_prompt);

    let request = CreateResponseArgs::default()
        .model(OPENAI_MODEL)
        .input(InputParam::Text(full_prompt))
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    let response = client.responses().create(request).await.map_err(|e| {
        let error_detail = format!("{:?}", e);
        eprintln!("OpenAI API error details: {}", error_detail);

        if error_detail.contains("401") || error_detail.contains("Unauthorized") {
            "Invalid API key. Please check your OpenAI API key.".to_string()
        } else if error_detail.contains("404") || error_detail.contains("model_not_found") {
            format!(
                "Model '{}' not found. Check if it's available on your account.",
                OPENAI_MODEL
            )
        } else if error_detail.contains("429") {
            "Rate limit exceeded. Please wait a moment.".to_string()
        } else if error_detail.contains("insufficient_quota") {
            "Insufficient quota. Check your OpenAI billing.".to_string()
        } else {
            format!("API error: {}", e)
        }
    })?;

    // Extract text from response
    Ok(response.output_text().unwrap_or_default())
}

// Tauri command handler for simple analysis
#[tauri::command]
pub async fn analyze_with_openai(
    request: OpenAIRequest,
    _app_handle: tauri::AppHandle,
) -> Result<OpenAIResponse, String> {
    let system_prompt = "You are a Windows system diagnostic expert. Analyze the user's request and provide helpful analysis.";

    let analysis = analyze_text(&request.api_key, system_prompt, &request.prompt).await?;
    let (findings, recommendations) = parse_analysis(&analysis);

    Ok(OpenAIResponse {
        analysis,
        diagnostics_run: Vec::new(),
        findings,
        recommendations,
    })
}

/// Enhanced version with tool calling support using Responses API
#[tauri::command]
pub async fn analyze_system_with_ai(
    api_key: String,
    prompt: String,
    _app_handle: tauri::AppHandle,
) -> Result<Value, String> {
    // Validate API key is not empty
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        return Err(DiagError::api_key(
            "validate",
            "OpenAI API key is empty. Please enter your API key in Settings.",
        )
        .into());
    }

    // Never echo any characters of the API key. In debug builds, log only its presence.
    #[cfg(debug_assertions)]
    eprintln!("OpenAI API key present (len={})", api_key.len());

    // Create the OpenAI client
    let config = OpenAIConfig::new().with_api_key(api_key);
    let client = Client::with_config(config);

    // Get available diagnostic tasks
    let available_tasks = crate::diagnostics::get_all_tasks();
    let task_list: Vec<Value> = available_tasks
        .iter()
        .map(|task| {
            json!({
                "name": task.id,
                "description": task.description
            })
        })
        .collect();

    // Define the diagnostic tool using Responses API function tool
    let diagnostic_tool = FunctionToolArgs::default()
        .name("run_diagnostic")
        .description("Run a Windows system diagnostic task to gather information")
        .parameters(json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The ID of the diagnostic task to run",
                    "enum": available_tasks.iter().map(|t| &t.id).collect::<Vec<_>>()
                },
                "reason": {
                    "type": "string",
                    "description": "Why you're running this diagnostic"
                }
            },
            "required": ["task_id", "reason"]
        }))
        .build()
        .map_err(|e| DiagError::AiAnalysisFailed {
            reason: format!("Failed to build tool: {}", e),
        })?;

    // System instructions with clear behavior rules
    let instructions = format!("You are a Windows system diagnostic assistant that TAKES IMMEDIATE ACTION.

CRITICAL BEHAVIOR RULES:
1. When user asks ANYTHING, IMMEDIATELY run AT LEAST 5-10 relevant diagnostics WITHOUT asking
2. NEVER list what diagnostics you can run or ask what they want - JUST RUN THEM
3. NEVER say you have access to or can run - you MUST run them RIGHT NOW
4. For ANY question about the system, run these IMMEDIATELY:
   - comp_system, os_info, processor, physical_memory, logical_disk
   - Then run 5+ more based on the specific question

ACTION PATTERNS:
- User asks about the system → RUN: comp_system, os_info, processor, physical_memory, logical_disk, disk_drive, network_adapter, ipconfig, installed_programs, services
- Check for issues → RUN: event_logs, dism_health, chkdsk, drivers_list, windows_update, scheduled_tasks, startup_command
- Performance issues → RUN: systeminfo, processes, services, startup_command, disk_usage, physical_memory
- Network problems → RUN: network_adapter, ipconfig, hosts_file, firewall_rules, dns_cache
- ANY other question → RUN at least 5 relevant diagnostics immediately

You have the run_diagnostic function with these tasks:
{}

FORMATTING:
- Present results as clean bullet points
- Show only relevant extracted information
- Never show raw JSON
- After running diagnostics, provide specific actionable recommendations

REMEMBER: The user wants ACTION, not explanations of what you could do. RUN DIAGNOSTICS IMMEDIATELY.",
        task_list.iter().map(|t| format!("- {}: {}", t["name"], t["description"])).collect::<Vec<_>>().join("\n"));

    // Reusable tool set (cloned into each turn of the tool-calling loop below).
    let tools: Vec<Tool> = vec![diagnostic_tool.into()];

    // Create request with tools
    let request = CreateResponseArgs::default()
        .model(OPENAI_MODEL)
        .instructions(instructions.clone())
        .input(InputParam::Text(prompt.clone()))
        .tools(tools.clone())
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    // Debug: Print available tasks
    eprintln!(
        "Available diagnostic tasks: {:?}",
        available_tasks.iter().map(|t| &t.id).collect::<Vec<_>>()
    );

    let mut response = client.responses().create(request).await.map_err(|e| {
        eprintln!("OpenAI API error details: {:?}", e);
        let error_msg = format!("{:?}", e);
        if error_msg.contains("401") || error_msg.contains("Unauthorized") {
            "Invalid API key. Please check your OpenAI API key is correct and starts with 'sk-'."
                .to_string()
        } else if error_msg.contains("404") {
            format!(
                "Model not found. The model '{}' may not be available on your account.",
                OPENAI_MODEL
            )
        } else if error_msg.contains("429") {
            "Rate limit exceeded. Please wait a moment and try again.".to_string()
        } else if error_msg.contains("insufficient_quota") {
            "Insufficient quota. Please check your OpenAI account billing.".to_string()
        } else {
            format!("OpenAI API error: {}. Make sure your API key is valid.", e)
        }
    })?;

    // Process the response - handle function calls if present
    let mut diagnostics_run = Vec::new();
    let mut diagnostic_results = HashMap::new();

    // Tool-calling loop: the model responds with function calls, we execute the requested
    // diagnostics, feed their outputs back via a follow-up request, and let the model
    // produce its FINAL analysis. Without this second turn, `analysis` would be whatever
    // (usually empty) text the model emitted BEFORE it ever saw any diagnostic data.
    const MAX_TOOL_ITERATIONS: usize = 5;
    for _ in 0..MAX_TOOL_ITERATIONS {
        // Collect and execute the function calls in this turn, building the tool outputs
        // to send back, each keyed by the model's call_id.
        let mut tool_outputs: Vec<InputItem> = Vec::new();
        for item in response.output.iter() {
            if let OutputItem::FunctionCall(func_call) = item {
                let args: Value =
                    serde_json::from_str(&func_call.arguments).unwrap_or_else(|_| json!({}));
                let task_id = args
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if task_id.is_empty() {
                    continue;
                }

                diagnostics_run.push(task_id.to_string());
                let result = match crate::diagnostics::run_diagnostic_task_sync(task_id) {
                    Ok(task_result) => json!({
                        "task_id": task_id,
                        "status": "completed",
                        "output": task_result.output,
                        "error": task_result.error,
                        "success": task_result.error.is_none()
                    }),
                    Err(e) => json!({
                        "task_id": task_id,
                        "status": "failed",
                        "error": format!("Failed to run diagnostic: {}", e),
                        "success": false
                    }),
                };

                tool_outputs.push(InputItem::Item(Item::FunctionCallOutput(
                    FunctionCallOutputItemParam {
                        call_id: func_call.call_id.clone(),
                        output: FunctionCallOutput::Text(result.to_string()),
                        id: None,
                        status: None,
                    },
                )));
                diagnostic_results.insert(task_id.to_string(), result);
            }
        }

        // No tool calls this turn → the model has produced its final text answer.
        if tool_outputs.is_empty() {
            break;
        }

        // Send the diagnostic results back (chained via previous_response_id) and ask the
        // model to continue. Re-pass instructions/tools since they don't carry over.
        let follow_up = CreateResponseArgs::default()
            .model(OPENAI_MODEL)
            .instructions(instructions.clone())
            .previous_response_id(response.id.clone())
            .input(InputParam::Items(tool_outputs))
            .tools(tools.clone())
            .build()
            .map_err(|e| format!("Failed to build follow-up request: {}", e))?;

        response = client
            .responses()
            .create(follow_up)
            .await
            .map_err(|e| format!("OpenAI API error on follow-up request: {:?}", e))?;
    }

    // Now that the model has seen the diagnostic results, take its final text analysis.
    let analysis = response.output_text().unwrap_or_default();
    let (findings, recommendations) = parse_analysis(&analysis);

    Ok(json!({
        "analysis": analysis,
        "diagnostics_run": diagnostics_run,
        "diagnostic_results": diagnostic_results,
        "findings": findings,
        "recommendations": recommendations
    }))
}

fn parse_analysis(analysis: &str) -> (Vec<Finding>, Vec<String>) {
    // Simple parsing logic - in production, this would be more sophisticated
    let mut findings = Vec::new();
    let mut recommendations = Vec::new();

    let lines: Vec<&str> = analysis.lines().collect();
    let mut in_findings = false;
    let mut in_recommendations = false;

    for line in lines {
        let lower = line.to_lowercase();

        if lower.contains("finding") || lower.contains("issue") {
            in_findings = true;
            in_recommendations = false;
        } else if lower.contains("recommendation") || lower.contains("suggest") {
            in_findings = false;
            in_recommendations = true;
        }

        if in_findings && !line.trim().is_empty() {
            // Try to parse severity from the line
            let severity = if lower.contains("critical") {
                "Critical"
            } else if lower.contains("warning") || lower.contains("warn") {
                "Warning"
            } else {
                "Info"
            };

            // Extract category based on keywords
            let category = if lower.contains("disk") || lower.contains("storage") {
                "Storage"
            } else if lower.contains("network") {
                "Network"
            } else if lower.contains("driver") {
                "Drivers"
            } else if lower.contains("memory") || lower.contains("ram") {
                "Memory"
            } else if lower.contains("cpu") || lower.contains("processor") {
                "CPU"
            } else {
                "System"
            };

            if line.trim().len() > 5 && !line.trim().starts_with("##") {
                findings.push(Finding {
                    category: category.to_string(),
                    severity: severity.to_string(),
                    description: line.trim().to_string(),
                    details: None,
                });
            }
        }

        if in_recommendations && !line.trim().is_empty() && line.trim().len() > 5 {
            recommendations.push(line.trim().to_string());
        }
    }

    (findings, recommendations)
}

/// Analyze system using a local OpenAI-compatible endpoint (Foundry Local).
/// The endpoint is discovered dynamically — settings override first, then the
/// foundry CLI. Command name kept for IPC compatibility; the model served is
/// phi-4-mini, not Phi Silica (which is only reachable via the Windows AI
/// APIs and needs package identity).
#[allow(dead_code)]
#[tauri::command]
pub async fn analyze_system_with_phi_silica(
    prompt: String,
    _app_handle: tauri::AppHandle,
) -> Result<Value, String> {
    let Some(endpoint) = local_ai_endpoint().await else {
        return Err(DiagError::ai_unavailable(
            "foundry_local",
            "No local AI endpoint available. Install Foundry Local and run 'foundry service \
             start', or configure an endpoint in Settings.",
        )
        .into());
    };

    // Create client configured for the local OpenAI-compatible server
    let config = OpenAIConfig::new()
        .with_api_base(format!("{}/v1", endpoint))
        .with_api_key("not-needed"); // Local server doesn't require API key
    let client = Client::with_config(config);

    // Get available diagnostic tasks for context
    let available_tasks = crate::diagnostics::get_all_tasks();

    // Run common diagnostics first and include results in the prompt
    let mut diagnostic_output = String::new();
    let common_diagnostics = vec![
        "comp_system",
        "os_info",
        "processor",
        "physical_memory",
        "logical_disk",
        "network_adapter",
    ];

    let mut diagnostics_run = Vec::new();
    for task_id in &common_diagnostics {
        if let Ok(result) = crate::diagnostics::run_diagnostic_task_sync(task_id) {
            diagnostics_run.push(task_id.to_string());
            diagnostic_output.push_str(&format!("\n=== {} ===\n", task_id));
            diagnostic_output.push_str(&result.output);
            if let Some(error) = result.error {
                diagnostic_output.push_str(&format!("Error: {}", error));
            }
        }
    }

    // Build the full prompt with diagnostic data
    let full_prompt = format!(
        "You are a Windows system diagnostic expert. Analyze the following system information and respond to the user's question.\n\n\
        Available diagnostic commands that could be run: {}\n\n\
        Current system diagnostic results:\n{}\n\n\
        User's question: {}",
        available_tasks
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>()
            .join(", "),
        diagnostic_output,
        prompt
    );

    let request = CreateResponseArgs::default()
        .model(FOUNDRY_LOCAL_MODEL)
        .input(InputParam::Text(full_prompt))
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    let response = client.responses().create(request).await.map_err(|e| {
        format!(
            "Local AI endpoint error: {}. Ensure the Foundry Local service is running \
             ('foundry service start') and the model '{}' is loaded.",
            e, FOUNDRY_LOCAL_MODEL
        )
    })?;

    let analysis = response.output_text().unwrap_or_default();
    let (findings, recommendations) = parse_analysis(&analysis);

    Ok(json!({
        "analysis": analysis,
        "diagnostics_run": diagnostics_run,
        "diagnostic_results": {},
        "findings": findings,
        "recommendations": recommendations,
        "provider": "foundry_local"
    }))
}

/// Unified AI analysis command that supports both OpenAI and Phi Silica
#[allow(dead_code)]
#[tauri::command]
pub async fn analyze_system_with_ai_provider(
    provider: String,
    api_key: Option<String>,
    prompt: String,
    app_handle: tauri::AppHandle,
) -> Result<Value, String> {
    match provider.as_str() {
        "phi_silica" => analyze_system_with_phi_silica(prompt, app_handle).await,
        _ => {
            // Default to OpenAI
            let key = api_key
                .ok_or_else(|| DiagError::api_key("validate", "OpenAI API key is required"))?;
            analyze_system_with_ai(key, prompt, app_handle).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extract_http_base;

    #[test]
    fn extracts_base_from_foundry_status_output() {
        let out = "🟢 Model management service is running on http://127.0.0.1:55769/openai/status";
        assert_eq!(
            extract_http_base(out),
            Some("http://127.0.0.1:55769".to_string())
        );
    }

    #[test]
    fn extracts_base_without_path() {
        assert_eq!(
            extract_http_base("endpoint: http://localhost:5273 ready"),
            Some("http://localhost:5273".to_string())
        );
    }

    #[test]
    fn returns_none_when_no_url_present() {
        assert_eq!(extract_http_base("service is not running"), None);
        assert_eq!(extract_http_base("http://"), None);
    }
}
