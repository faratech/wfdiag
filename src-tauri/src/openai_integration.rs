use anyhow::Result;
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::responses::{
        CreateResponseArgs, InputParam, FunctionToolArgs,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

/// AI Provider options
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum AiProvider {
    #[default]
    OpenAI,
    PhiSilica, // Local Phi Silica via WindowsCopilotRuntimeServer
}

/// OpenAI model to use for all API calls (when Phi Silica is not available)
/// Change this constant to switch models globally
pub const OPENAI_MODEL: &str = "gpt-5-nano";

/// Phi Silica local server configuration
#[allow(dead_code)]
const PHI_SILICA_BASE_URL: &str = "http://localhost:5001/v1";
#[allow(dead_code)]
const PHI_SILICA_MODEL: &str = "phi-silica"; // Model name used by WindowsCopilotRuntimeServer

/// Check if Phi Silica (WindowsCopilotRuntimeServer) is available
async fn check_phi_silica_available() -> bool {
    // Try to connect to the local Phi Silica server via TCP
    use std::net::TcpStream;
    use std::time::Duration;

    let addr = "127.0.0.1:5001";

    match addr.parse() {
        Ok(addr) => TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok(),
        Err(_) => {
            // If the hardcoded address ever becomes invalid, treat the service as unavailable
            false
        }
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
    let phi_silica_available = check_phi_silica_available().await;

    Ok(AiProviderStatus {
        openai_available: true,
        phi_silica_available,
        phi_silica_info: if phi_silica_available {
            Some(
                "Phi Silica is available via WindowsCopilotRuntimeServer on localhost:5001"
                    .to_string(),
            )
        } else {
            Some("Phi Silica not available. Install WindowsCopilotRuntimeServer from Microsoft Store on a Copilot+ PC.".to_string())
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
pub async fn analyze_text(api_key: &str, system_prompt: &str, user_prompt: &str) -> Result<String, String> {
    let config = OpenAIConfig::new().with_api_key(api_key);
    let client = Client::with_config(config);

    let full_prompt = format!("{}\n\nUser: {}", system_prompt, user_prompt);

    let request = CreateResponseArgs::default()
        .model(OPENAI_MODEL)
        .input(InputParam::Text(full_prompt))
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    let response = client
        .responses()
        .create(request)
        .await
        .map_err(|e| {
            let error_detail = format!("{:?}", e);
            eprintln!("OpenAI API error details: {}", error_detail);

            if error_detail.contains("401") || error_detail.contains("Unauthorized") {
                "Invalid API key. Please check your OpenAI API key.".to_string()
            } else if error_detail.contains("404") || error_detail.contains("model_not_found") {
                format!("Model '{}' not found. Check if it's available on your account.", OPENAI_MODEL)
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
        return Err("OpenAI API key is empty. Please enter your API key in Settings.".to_string());
    }

    // Log key prefix for debugging (masked)
    let key_preview = if api_key.len() > 8 {
        format!("{}...{}", &api_key[..4], &api_key[api_key.len() - 4..])
    } else {
        "****".to_string()
    };
    println!("Using API key: {}", key_preview);

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
        .map_err(|e| format!("Failed to build tool: {}", e))?;

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

    // Create request with tools
    let request = CreateResponseArgs::default()
        .model(OPENAI_MODEL)
        .instructions(instructions)
        .input(InputParam::Text(prompt.clone()))
        .tools(vec![diagnostic_tool.into()])
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    // Debug: Print available tasks
    eprintln!(
        "Available diagnostic tasks: {:?}",
        available_tasks.iter().map(|t| &t.id).collect::<Vec<_>>()
    );

    let response = client.responses().create(request).await.map_err(|e| {
        eprintln!("OpenAI API error details: {:?}", e);
        let error_msg = format!("{:?}", e);
        if error_msg.contains("401") || error_msg.contains("Unauthorized") {
            "Invalid API key. Please check your OpenAI API key is correct and starts with 'sk-'.".to_string()
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
            format!(
                "OpenAI API error: {}. Make sure your API key is valid.",
                e
            )
        }
    })?;

    // Process the response - handle function calls if present
    let mut diagnostics_run = Vec::new();
    let mut diagnostic_results = HashMap::new();

    // Check for function calls in the output
    for item in response.output.iter() {
        if let async_openai::types::responses::OutputItem::FunctionCall(func_call) = item {
            // Parse the arguments JSON string
            let args: Value = serde_json::from_str(&func_call.arguments)
                .unwrap_or_else(|_| json!({}));

            let task_id = args.get("task_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            if !task_id.is_empty() {
                diagnostics_run.push(task_id.to_string());

                // Run the diagnostic
                let result = match crate::diagnostics::run_diagnostic_task_sync(task_id) {
                    Ok(task_result) => {
                        json!({
                            "task_id": task_id,
                            "status": "completed",
                            "output": task_result.output,
                            "error": task_result.error,
                            "success": task_result.error.is_none()
                        })
                    }
                    Err(e) => {
                        json!({
                            "task_id": task_id,
                            "status": "failed",
                            "error": format!("Failed to run diagnostic: {}", e),
                            "success": false
                        })
                    }
                };
                diagnostic_results.insert(task_id.to_string(), result);
            }
        }
    }

    // Get the text analysis
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

/// Analyze system using Phi Silica (WindowsCopilotRuntimeServer on localhost:5001)
/// This uses the same OpenAI-compatible API but with a local endpoint
#[allow(dead_code)]
#[tauri::command]
pub async fn analyze_system_with_phi_silica(
    prompt: String,
    _app_handle: tauri::AppHandle,
) -> Result<Value, String> {
    // First check if Phi Silica server is available
    if !check_phi_silica_available().await {
        return Err("Phi Silica is not available. Please ensure WindowsCopilotRuntimeServer is running on your Copilot+ PC. You can download it from https://github.com/sykuang/WindowsCopilotRuntimeServer".to_string());
    }

    // Create client configured for local Phi Silica server
    let config = OpenAIConfig::new()
        .with_api_base(PHI_SILICA_BASE_URL)
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
        .model(PHI_SILICA_MODEL)
        .input(InputParam::Text(full_prompt))
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    let response = client.responses().create(request).await.map_err(|e| {
        format!(
            "Phi Silica API error: {}. Ensure WindowsCopilotRuntimeServer is running.",
            e
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
        "provider": "phi_silica"
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
            let key = api_key.ok_or("OpenAI API key is required")?;
            analyze_system_with_ai(key, prompt, app_handle).await
        }
    }
}
