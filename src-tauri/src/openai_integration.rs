use anyhow::Result;
use async_openai::{
    types::{
        ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionToolArgs, ChatCompletionToolType,
        CreateChatCompletionRequestArgs, FunctionObjectArgs,
        ChatCompletionRequestUserMessageContent,
    },
    Client,
    config::OpenAIConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

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

    pub async fn analyze_system(&self, prompt: String, available_tasks: Vec<String>) -> Result<OpenAIResponse> {
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
            .content("You are a Windows system diagnostic expert. You analyze system diagnostics to identify issues and provide recommendations. 
When analyzing a system:
1. First, run relevant diagnostic tasks based on the user's prompt
2. Analyze the results to identify any issues or anomalies
3. Provide clear findings categorized by severity (Critical, Warning, Info)
4. Give actionable recommendations to fix any issues found

Available diagnostic tasks include system info, hardware details, driver status, disk health, network configuration, and more.
Always be thorough but concise in your analysis.")
            .build()?
            .into();

        let user_message = ChatCompletionRequestUserMessageArgs::default()
            .content(prompt)
            .build()?
            .into();

        let messages = vec![system_message, user_message];

        // Create the request
        let request = CreateChatCompletionRequestArgs::default()
            .model("gpt-4o-mini")
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
        if let Some(choice) = final_response.choices.first() {
            if let Some(tool_calls) = &choice.message.tool_calls {
                // Add assistant message with tool calls
                let assistant_message = ChatCompletionRequestAssistantMessageArgs::default()
                    .content(choice.message.content.clone().unwrap_or_default())
                    .tool_calls(tool_calls.clone())
                    .build()?
                    .into();
                messages.push(assistant_message);

                // Process each tool call
                for tool_call in tool_calls {
                    let function_name = &tool_call.function.name;
                    let arguments: Value = serde_json::from_str(&tool_call.function.arguments)?;

                    match function_name.as_str() {
                        "run_diagnostic" => {
                            if let Some(task_id) = arguments.get("task_id").and_then(|v| v.as_str()) {
                                diagnostics_run.push(task_id.to_string());
                                // Note: In the actual implementation, this would call the diagnostic
                                // For now, we'll return a placeholder indicating the tool needs to be called
                            }
                        }
                        "get_all_diagnostics" => {
                            // This would retrieve all diagnostic results
                        }
                        _ => {}
                    }
                }

                // For now, return a response indicating tool calls need to be handled
                return Ok(OpenAIResponse {
                    analysis: "Tool calls requested - please implement diagnostic execution".to_string(),
                    diagnostics_run,
                    findings: vec![],
                    recommendations: vec![],
                });
            }
        }

        // Parse the final response
        let analysis = final_response.choices.first()
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
    match analyzer.analyze_system(request.prompt, available_tasks).await {
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
        ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestToolMessageArgs,
        ChatCompletionToolArgs, ChatCompletionToolType,
        Role,
    };
    
    // Create the OpenAI client
    let config = OpenAIConfig::new().with_api_key(api_key);
    let client = Client::with_config(config);
    
    // Get available diagnostic tasks
    let available_tasks = crate::diagnostics::get_all_tasks();
    let task_list: Vec<Value> = available_tasks.iter().map(|task| {
        json!({
            "name": task.id,
            "description": task.description
        })
    }).collect();
    
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
        .content(format!("You are a Windows system diagnostic assistant with access to function calls.

You have access to the following function:
- run_diagnostic: Use this to run Windows diagnostic tasks and get real system information

When the user asks ANYTHING about:
- What tools/functions you have available
- Checking their system
- Running diagnostics
- Scanning for issues
- Analyzing system health

You MUST mention that you have the run_diagnostic function available and can run these diagnostic tasks:
{}

For system checks, ALWAYS use the run_diagnostic function instead of giving generic advice.

Example: If asked to check disk space, call run_diagnostic with task_id='disk_space' instead of explaining how to check disk space manually.", 
            task_list.iter().map(|t| format!("- {}: {}", t["name"], t["description"])).collect::<Vec<_>>().join("\n")))
        .build()
        .map_err(|e| format!("Failed to build message: {}", e))?
        .into();
    
    let user_message = ChatCompletionRequestUserMessageArgs::default()
        .content(ChatCompletionRequestUserMessageContent::Text(prompt.clone()))
        .build()
        .map_err(|e| format!("Failed to build message: {}", e))?
        .into();
    
    let mut messages = vec![system_message, user_message];
    
    // Create request with tools
    let request = CreateChatCompletionRequestArgs::default()
        .model("gpt-4o-mini")
        .messages(messages.clone())
        .tools(vec![diagnostic_tool])
        .tool_choice("auto")
        .build()
        .map_err(|e| format!("Failed to build request: {}", e))?;
    
    // Debug: Print available tasks
    eprintln!("Available diagnostic tasks: {:?}", available_tasks.iter().map(|t| &t.id).collect::<Vec<_>>());
    
    let response = match client.chat().create(request).await {
        Ok(resp) => {
            eprintln!("OpenAI response received successfully");
            resp
        },
        Err(e) => {
            eprintln!("OpenAI API error details: {:?}", e);
            let error_msg = format!("{:?}", e);
            if error_msg.contains("401") || error_msg.contains("Unauthorized") {
                return Err("Invalid API key. Please check your OpenAI API key is correct and starts with 'sk-'.".to_string());
            } else if error_msg.contains("404") {
                return Err("Model not found. The model 'gpt-4o-mini' may not be available on your account.".to_string());
            } else if error_msg.contains("429") {
                return Err("Rate limit exceeded. Please wait a moment and try again.".to_string());
            } else if error_msg.contains("insufficient_quota") {
                return Err("Insufficient quota. Please check your OpenAI account billing.".to_string());
            }
            return Err(format!("OpenAI API error: {}. Make sure your API key is valid.", e));
        }
    };
    
    let mut diagnostics_run = Vec::new();
    let mut diagnostic_results = HashMap::new();
    
    // Check if the AI wants to run diagnostics
    if let Some(choice) = response.choices.first() {
        eprintln!("Response has tool_calls: {}", choice.message.tool_calls.is_some());
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
                        let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("Checking system");
                        diagnostics_run.push(task_id.to_string());
                        
                        // Simulate diagnostic result (in real implementation, would run actual diagnostic)
                        let result = json!({
                            "task_id": task_id,
                            "status": "completed",
                            "reason": reason,
                            "output": format!("This is where the actual output from {} would appear. For now, this is a placeholder.", task_id),
                            "message": "Diagnostic completed successfully"
                        });
                        
                        diagnostic_results.insert(task_id.to_string(), result.clone());
                        
                        // Add tool response
                        let tool_message = ChatCompletionRequestToolMessageArgs::default()
                            .role(Role::Tool)
                            .tool_call_id(tool_call.id.clone())
                            .content(serde_json::to_string(&result).unwrap_or_default())
                            .build()
                            .map_err(|e| format!("Failed to build tool message: {}", e))?
                            .into();
                        messages.push(tool_message);
                    }
                }
            }
            
            // Get final response with diagnostic results
            let final_request = CreateChatCompletionRequestArgs::default()
                .model("gpt-4o-mini")
                .messages(messages)
                .build()
                .map_err(|e| format!("Failed to build final request: {}", e))?;
            
            let final_response = client.chat().create(final_request).await
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