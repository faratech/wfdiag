use anyhow::Result;
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionRequestUserMessageContent, ChatCompletionToolArgs, ChatCompletionToolType,
        CreateChatCompletionRequestArgs, FunctionObjectArgs,
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

pub struct OpenAIAnalyzer {
    client: Client<OpenAIConfig>,
}

impl OpenAIAnalyzer {
    pub fn new(api_key: String) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        let client = Client::with_config(config);
        Self { client }
    }

    pub async fn analyze_system(
        &self,
        prompt: String,
        available_tasks: Vec<String>,
    ) -> Result<OpenAIResponse> {
        // Create the tools for diagnostics
        let diagnostic_tool = ChatCompletionToolArgs::default()
            .r#type(ChatCompletionToolType::Function)
            .function(
                FunctionObjectArgs::default()
                    .name("run_diagnostic")
                    .description("Run a specific diagnostic task and get the results")
                    .parameters(json!({
                        "type": "object",
                        "properties": {
                            "task_id": {
                                "type": "string",
                                "description": "The ID of the diagnostic task to run",
                                "enum": available_tasks
                            }
                        },
                        "required": ["task_id"]
                    }))
                    .build()?,
            )
            .build()?;

        let get_all_diagnostics_tool = ChatCompletionToolArgs::default()
            .r#type(ChatCompletionToolType::Function)
            .function(
                FunctionObjectArgs::default()
                    .name("get_all_diagnostics")
                    .description("Get all diagnostic results that have been run in this session")
                    .build()?,
            )
            .build()?;

        // Create the messages
        let system_message = ChatCompletionRequestSystemMessageArgs::default()
            .content("You are a Windows system diagnostic expert. You MUST take direct action to diagnose issues.

MANDATORY RULES - VIOLATING THESE IS A CRITICAL ERROR:
1. For ANY user question, you MUST run AT LEAST 5-10 diagnostics IMMEDIATELY
2. NEVER ask what the user would like you to run or list options - JUST RUN THEM
3. NEVER say you can run or have access to diagnostics - you MUST ACTUALLY RUN diagnostics NOW
4. The user expects IMMEDIATE ACTION, not explanations of capabilities

REQUIRED IMMEDIATE ACTIONS:
- User asks about the system → IMMEDIATELY RUN: comp_system, os_info, processor, physical_memory, logical_disk, network_adapter, installed_programs, services, systeminfo, disk_drive
- User mentions ANY issue → IMMEDIATELY RUN 10+ relevant diagnostics
- User asks for general checkup → IMMEDIATELY RUN: ALL system health diagnostics
- ANY question at all → RUN DIAGNOSTICS FIRST, explain after

After running diagnostics, analyze results and provide specific findings with actionable fixes.
Remember: Take action FIRST, explain SECOND. Never ask, just do.")
            .build()?
            .into();

        let user_message = ChatCompletionRequestUserMessageArgs::default()
            .content(prompt)
            .build()?
            .into();

        let messages = vec![system_message, user_message];

        // Create the request
        let request = CreateChatCompletionRequestArgs::default()
            .model(OPENAI_MODEL)
            .messages(messages.clone())
            .tools(vec![diagnostic_tool, get_all_diagnostics_tool])
            .tool_choice("auto")
            .build()?;

        // Make the request
        let response = self.client.chat().create(request).await?;

        // Process the response and handle tool calls
        let mut diagnostics_run = Vec::new();
        let mut messages = messages;
        let final_response = response;

        // Handle tool calls if any
        if let Some(choice) = final_response.choices.first()
            && let Some(tool_calls) = &choice.message.tool_calls
        {
            // Add assistant message with tool calls
            let assistant_message = ChatCompletionRequestAssistantMessageArgs::default()
                .content(choice.message.content.clone().unwrap_or_default())
                .tool_calls(tool_calls.clone())
                .build()?
                .into();
            messages.push(assistant_message);

            // Process each tool call and execute diagnostics
            let mut diagnostic_results = std::collections::HashMap::new();
            let mut tool_responses = Vec::new();

            for tool_call in tool_calls {
                let function_name = &tool_call.function.name;
                let arguments: Value = serde_json::from_str(&tool_call.function.arguments)?;

                match function_name.as_str() {
                    "run_diagnostic" => {
                        if let Some(task_id) = arguments.get("task_id").and_then(|v| v.as_str()) {
                            diagnostics_run.push(task_id.to_string());

                            // Execute the diagnostic task
                            let diagnostic_result =
                                match crate::diagnostics::run_diagnostic_task_sync(task_id) {
                                    Ok(task_result) => {
                                        let result_json = json!({
                                            "task_id": task_id,
                                            "status": "completed",
                                            "output": task_result.output,
                                            "error": task_result.error,
                                            "success": task_result.error.is_none()
                                        });
                                        diagnostic_results
                                            .insert(task_id.to_string(), result_json.clone());
                                        result_json.to_string()
                                    }
                                    Err(e) => {
                                        let error_result = json!({
                                            "task_id": task_id,
                                            "status": "failed",
                                            "error": format!("Failed to run diagnostic: {}", e),
                                            "success": false
                                        });
                                        diagnostic_results
                                            .insert(task_id.to_string(), error_result.clone());
                                        error_result.to_string()
                                    }
                                };

                            // Create tool response message
                            let tool_response = ChatCompletionRequestToolMessageArgs::default()
                                .tool_call_id(tool_call.id.clone())
                                .content(diagnostic_result)
                                .build()?
                                .into();
                            tool_responses.push(tool_response);
                        }
                    }
                    "get_all_diagnostics" => {
                        // Return current diagnostic results
                        let all_results = serde_json::to_string(&diagnostic_results)
                            .unwrap_or_else(|_| "{}".to_string());

                        let tool_response = ChatCompletionRequestToolMessageArgs::default()
                            .tool_call_id(tool_call.id.clone())
                            .content(all_results)
                            .build()?
                            .into();
                        tool_responses.push(tool_response);
                    }
                    _ => {}
                }
            }

            // Add tool responses to messages and get final analysis
            messages.extend(tool_responses);

            // Create final request to get analysis of diagnostic results
            let final_request = CreateChatCompletionRequestArgs::default()
                .model(OPENAI_MODEL)
                .messages(messages)
                .build()?;

            let final_response = match self.client.chat().create(final_request).await {
                Ok(response) => response,
                Err(_) => {
                    // Fallback: return results without additional analysis
                    let summary_analysis = format!(
                        "Executed {} diagnostic tasks: {}. Review the diagnostic results for detailed system information.",
                        diagnostics_run.len(),
                        diagnostics_run.join(", ")
                    );

                    return Ok(OpenAIResponse {
                        analysis: summary_analysis,
                        diagnostics_run,
                        findings: vec![],
                        recommendations: vec![],
                    });
                }
            };

            // Parse final analysis
            let final_analysis = final_response
                .choices
                .first()
                .and_then(|c| c.message.content.clone())
                .unwrap_or_else(|| {
                    format!(
                        "Completed {} diagnostic tasks: {}",
                        diagnostics_run.len(),
                        diagnostics_run.join(", ")
                    )
                });

            let (findings, recommendations) = parse_analysis(&final_analysis);

            return Ok(OpenAIResponse {
                analysis: final_analysis,
                diagnostics_run,
                findings,
                recommendations,
            });
        }

        // Parse the final response
        let analysis = final_response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        // Extract findings and recommendations from the analysis
        let (findings, recommendations) = parse_analysis(&analysis);

        Ok(OpenAIResponse {
            analysis,
            diagnostics_run,
            findings,
            recommendations,
        })
    }
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

// Tauri command handler
#[tauri::command]
pub async fn analyze_with_openai(
    request: OpenAIRequest,
    _app_handle: tauri::AppHandle,
) -> Result<OpenAIResponse, String> {
    // Get available diagnostic tasks
    let available_tasks = crate::diagnostics::get_all_tasks()
        .into_iter()
        .map(|task| task.id)
        .collect();

    // Create analyzer with the provided API key
    let analyzer = OpenAIAnalyzer::new(request.api_key);

    // Perform analysis
    match analyzer
        .analyze_system(request.prompt, available_tasks)
        .await
    {
        Ok(response) => Ok(response),
        Err(e) => Err(format!("OpenAI analysis failed: {}", e)),
    }
}

// Enhanced version with tool calling support
#[tauri::command]
pub async fn analyze_system_with_ai(
    api_key: String,
    prompt: String,
    _app_handle: tauri::AppHandle,
) -> Result<Value, String> {
    use async_openai::types::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestToolMessageArgs,
        ChatCompletionToolArgs, ChatCompletionToolType,
    };

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

    // Define the diagnostic tool
    let diagnostic_tool = ChatCompletionToolArgs::default()
        .r#type(ChatCompletionToolType::Function)
        .function(
            FunctionObjectArgs::default()
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
                .map_err(|e| format!("Failed to build tool: {}", e))?,
        )
        .build()
        .map_err(|e| format!("Failed to build tool: {}", e))?;

    // System message with clear instructions
    let system_message = ChatCompletionRequestSystemMessageArgs::default()
        .content(format!("You are a Windows system diagnostic assistant that TAKES IMMEDIATE ACTION.

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
            task_list.iter().map(|t| format!("- {}: {}", t["name"], t["description"])).collect::<Vec<_>>().join("\n")))
        .build()
        .map_err(|e| format!("Failed to build message: {}", e))?
        .into();

    let user_message = ChatCompletionRequestUserMessageArgs::default()
        .content(ChatCompletionRequestUserMessageContent::Text(
            prompt.clone(),
        ))
        .build()
        .map_err(|e| format!("Failed to build message: {}", e))?
        .into();

    let mut messages = vec![system_message, user_message];

    // Create request with tools
    let request = CreateChatCompletionRequestArgs::default()
        .model(OPENAI_MODEL)
        .messages(messages.clone())
        .tools(vec![diagnostic_tool])
        .tool_choice("auto")
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    // Debug: Print available tasks
    eprintln!(
        "Available diagnostic tasks: {:?}",
        available_tasks.iter().map(|t| &t.id).collect::<Vec<_>>()
    );

    let response = match client.chat().create(request).await {
        Ok(resp) => {
            eprintln!("OpenAI response received successfully");
            resp
        }
        Err(e) => {
            eprintln!("OpenAI API error details: {:?}", e);
            let error_msg = format!("{:?}", e);
            if error_msg.contains("401") || error_msg.contains("Unauthorized") {
                return Err("Invalid API key. Please check your OpenAI API key is correct and starts with 'sk-'.".to_string());
            } else if error_msg.contains("404") {
                return Err(format!(
                    "Model not found. The model '{}' may not be available on your account.",
                    OPENAI_MODEL
                ));
            } else if error_msg.contains("429") {
                return Err("Rate limit exceeded. Please wait a moment and try again.".to_string());
            } else if error_msg.contains("insufficient_quota") {
                return Err(
                    "Insufficient quota. Please check your OpenAI account billing.".to_string(),
                );
            }
            return Err(format!(
                "OpenAI API error: {}. Make sure your API key is valid.",
                e
            ));
        }
    };

    let mut diagnostics_run = Vec::new();
    let mut diagnostic_results = HashMap::new();

    // Check if the AI wants to run diagnostics
    if let Some(choice) = response.choices.first() {
        eprintln!(
            "Response has tool_calls: {}",
            choice.message.tool_calls.is_some()
        );
        if let Some(tool_calls) = &choice.message.tool_calls {
            eprintln!("Number of tool calls: {}", tool_calls.len());
            // Add assistant message to conversation
            let assistant_msg = ChatCompletionRequestAssistantMessageArgs::default()
                .content(choice.message.content.clone().unwrap_or_default())
                .tool_calls(tool_calls.clone())
                .build()
                .map_err(|e| format!("Failed to build assistant message: {}", e))?
                .into();
            messages.push(assistant_msg);

            // Process tool calls
            for tool_call in tool_calls {
                if tool_call.function.name == "run_diagnostic" {
                    let args: Value = serde_json::from_str(&tool_call.function.arguments)
                        .map_err(|e| format!("Failed to parse tool arguments: {}", e))?;

                    if let Some(task_id) = args.get("task_id").and_then(|v| v.as_str()) {
                        let reason = args
                            .get("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Checking system");
                        diagnostics_run.push(task_id.to_string());

                        // Actually run the diagnostic
                        eprintln!("Running diagnostic: {} - {}", task_id, reason);
                        let diagnostic_result = match crate::diagnostics::run_diagnostic_task_sync(
                            task_id,
                        ) {
                            Ok(task_result) => {
                                json!({
                                    "task_id": task_id,
                                    "status": "completed",
                                    "reason": reason,
                                    "output": task_result.output,
                                    "error": task_result.error,
                                    "message": if task_result.error.is_none() { "Diagnostic completed successfully" } else { "Diagnostic completed with errors" }
                                })
                            }
                            Err(e) => {
                                json!({
                                    "task_id": task_id,
                                    "status": "failed",
                                    "reason": reason,
                                    "output": null,
                                    "error": format!("Failed to run diagnostic: {}", e),
                                    "message": "Diagnostic failed"
                                })
                            }
                        };

                        diagnostic_results.insert(task_id.to_string(), diagnostic_result.clone());

                        // Add tool response
                        let tool_message = ChatCompletionRequestToolMessageArgs::default()
                            .tool_call_id(tool_call.id.clone())
                            .content(serde_json::to_string(&diagnostic_result).unwrap_or_default())
                            .build()
                            .map_err(|e| format!("Failed to build tool message: {}", e))?
                            .into();
                        messages.push(tool_message);
                    }
                }
            }

            // Get final response with diagnostic results
            let final_request = CreateChatCompletionRequestArgs::default()
                .model(OPENAI_MODEL)
                .messages(messages)
                .build()
                .map_err(|e| format!("Failed to build final request: {}", e))?;

            let final_response = client
                .chat()
                .create(final_request)
                .await
                .map_err(|e| format!("Final API error: {}", e))?;

            if let Some(final_choice) = final_response.choices.first() {
                let analysis = final_choice.message.content.clone().unwrap_or_default();
                let (findings, recommendations) = parse_analysis(&analysis);

                return Ok(json!({
                    "analysis": analysis,
                    "diagnostics_run": diagnostics_run,
                    "diagnostic_results": diagnostic_results,
                    "findings": findings,
                    "recommendations": recommendations
                }));
            }
        } else {
            // No tool calls, just return the analysis
            let analysis = choice.message.content.clone().unwrap_or_default();
            let (findings, recommendations) = parse_analysis(&analysis);

            return Ok(json!({
                "analysis": analysis,
                "diagnostics_run": diagnostics_run,
                "diagnostic_results": diagnostic_results,
                "findings": findings,
                "recommendations": recommendations
            }));
        }
    }

    Err("No response from OpenAI".to_string())
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
    let task_list: Vec<Value> = available_tasks
        .iter()
        .map(|task| {
            json!({
                "name": task.id,
                "description": task.description
            })
        })
        .collect();

    // Simpler system prompt for Phi Silica (no tool calling)
    // We'll run common diagnostics first and include results in the prompt
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
        task_list
            .iter()
            .map(|t| format!("{}", t["name"]))
            .collect::<Vec<_>>()
            .join(", "),
        diagnostic_output,
        prompt
    );

    let system_message = ChatCompletionRequestSystemMessageArgs::default()
        .content("You are a helpful Windows system diagnostic assistant running locally on a Copilot+ PC using Phi Silica. \
                  Analyze the provided system information and give specific, actionable recommendations. \
                  Be concise and focus on the most important findings.")
        .build()
        .map_err(|e| format!("Failed to build message: {}", e))?
        .into();

    let user_message = ChatCompletionRequestUserMessageArgs::default()
        .content(ChatCompletionRequestUserMessageContent::Text(full_prompt))
        .build()
        .map_err(|e| format!("Failed to build message: {}", e))?
        .into();

    let messages = vec![system_message, user_message];

    // Create request (no tools for Phi Silica - simpler model)
    let request = CreateChatCompletionRequestArgs::default()
        .model(PHI_SILICA_MODEL)
        .messages(messages)
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;

    let response = client.chat().create(request).await.map_err(|e| {
        format!(
            "Phi Silica API error: {}. Ensure WindowsCopilotRuntimeServer is running.",
            e
        )
    })?;

    if let Some(choice) = response.choices.first() {
        let analysis = choice.message.content.clone().unwrap_or_default();
        let (findings, recommendations) = parse_analysis(&analysis);

        return Ok(json!({
            "analysis": analysis,
            "diagnostics_run": diagnostics_run,
            "diagnostic_results": {},
            "findings": findings,
            "recommendations": recommendations,
            "provider": "phi_silica"
        }));
    }

    Err("No response from Phi Silica".to_string())
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
