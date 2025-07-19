use anyhow::Result;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
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

// OpenAI API response structures
#[derive(Debug, Serialize, Deserialize)]
struct ChatCompletionResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Choice {
    index: i32,
    message: Message,
    finish_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolCall {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct FunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Usage {
    prompt_tokens: i32,
    completion_tokens: i32,
    total_tokens: i32,
}

// Direct HTTP implementation of OpenAI Responses API
#[tauri::command]
pub async fn analyze_system_with_ai(
    api_key: String,
    prompt: String,
    _app_handle: tauri::AppHandle,
) -> Result<Value, String> {
    // Create HTTP client
    let client = reqwest::Client::new();
    
    // Set up headers
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION, 
        HeaderValue::from_str(&format!("Bearer {}", api_key))
            .map_err(|e| format!("Invalid API key format: {}", e))?
    );
    
    // Get available diagnostic tasks
    let available_tasks = crate::diagnostics::get_all_tasks();
    let task_list: Vec<Value> = available_tasks.iter().map(|task| {
        json!({
            "name": task.id,
            "description": task.description
        })
    }).collect();
    
    // Create the tool definition for diagnostics
    let diagnostic_tool = json!({
        "type": "function",
        "function": {
            "name": "run_diagnostic",
            "description": "Run a Windows system diagnostic task to gather information about the system",
            "parameters": {
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
            }
        }
    });
    
    // Create system message with clear instructions
    let system_message = format!(
        "You are a Windows system diagnostic assistant with access to diagnostic tools.

You have access to the run_diagnostic function which can execute these diagnostic tasks:
{}

IMPORTANT: When the user asks you to:
- Check their system
- Run diagnostics
- Scan for issues
- Analyze system health
- Find out what's wrong

You MUST use the run_diagnostic function to gather real system information.

Example responses:
- User: 'Check my disk space' -> Use run_diagnostic with task_id='disk_space'
- User: 'What tools do you have?' -> Explain you have the run_diagnostic function and list available tasks
- User: 'Scan my system' -> Use multiple diagnostic functions to check various aspects

Always use actual diagnostic data, not generic advice.",
        task_list.iter()
            .map(|t| format!("- {}: {}", t["name"], t["description"]))
            .collect::<Vec<_>>()
            .join("\n")
    );
    
    // Create initial messages
    let mut messages = vec![
        json!({
            "role": "system",
            "content": system_message
        }),
        json!({
            "role": "user",
            "content": prompt
        })
    ];
    
    // Create request body
    let request_body = json!({
        "model": "gpt-4.1",  // Using gpt-4.1 as requested
        "messages": messages,
        "tools": [diagnostic_tool],
        "tool_choice": "auto",
        "temperature": 0.7
    });
    
    eprintln!("Sending request to OpenAI Responses API with model: gpt-4.1");
    eprintln!("Available diagnostic tasks: {:?}", available_tasks.iter().map(|t| &t.id).collect::<Vec<_>>());
    
    // Make the API request
    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .headers(headers.clone())
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;
    
    let status = response.status();
    let response_text = response.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;
    
    eprintln!("Response status: {}", status);
    
    if !status.is_success() {
        eprintln!("Error response: {}", response_text);
        
        // Parse error response for better error messages
        if let Ok(error_json) = serde_json::from_str::<Value>(&response_text) {
            if let Some(error) = error_json.get("error") {
                let error_message = error.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error");
                let error_type = error.get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                
                return match error_type {
                    "invalid_api_key" => Err("Invalid API key. Please check your OpenAI API key.".to_string()),
                    "insufficient_quota" => Err("Insufficient quota. Please check your OpenAI account billing.".to_string()),
                    "model_not_found" => Err("Model 'gpt-4.1' not found. Please check if you have access to this model.".to_string()),
                    _ => Err(format!("OpenAI API error: {}", error_message)),
                };
            }
        }
        
        return Err(format!("API request failed with status {}: {}", status, response_text));
    }
    
    // Parse the response
    let completion: ChatCompletionResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse response: {}. Response: {}", e, response_text))?;
    
    let mut diagnostics_run = Vec::new();
    let mut diagnostic_results = HashMap::new();
    
    // Check if the AI wants to run diagnostics
    if let Some(choice) = completion.choices.first() {
        eprintln!("Response has tool_calls: {}", choice.message.tool_calls.is_some());
        
        if let Some(tool_calls) = &choice.message.tool_calls {
            eprintln!("Number of tool calls: {}", tool_calls.len());
            
            // Add assistant message to conversation
            messages.push(json!({
                "role": "assistant",
                "content": choice.message.content.clone().unwrap_or_default(),
                "tool_calls": tool_calls
            }));
            
            // Process tool calls
            for tool_call in tool_calls {
                if tool_call.function.name == "run_diagnostic" {
                    let args: Value = serde_json::from_str(&tool_call.function.arguments)
                        .map_err(|e| format!("Failed to parse tool arguments: {}", e))?;
                    
                    if let Some(task_id) = args.get("task_id").and_then(|v| v.as_str()) {
                        let reason = args.get("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Checking system");
                        
                        eprintln!("AI requested diagnostic: {} (reason: {})", task_id, reason);
                        diagnostics_run.push(task_id.to_string());
                        
                        // TODO: In a real implementation, run the actual diagnostic here
                        // For now, create a placeholder result
                        let result = json!({
                            "task_id": task_id,
                            "status": "completed",
                            "reason": reason,
                            "output": format!("Diagnostic '{}' results would appear here. This is a placeholder.", task_id),
                            "data": {
                                "example": "This would contain actual diagnostic data"
                            }
                        });
                        
                        diagnostic_results.insert(task_id.to_string(), result.clone());
                        
                        // Add tool response message
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_call.id,
                            "content": serde_json::to_string(&result).unwrap_or_default()
                        }));
                    }
                }
            }
            
            // Get final response with diagnostic results
            let final_request_body = json!({
                "model": "gpt-4.1",
                "messages": messages,
                "temperature": 0.7
            });
            
            let final_response = client
                .post("https://api.openai.com/v1/chat/completions")
                .headers(headers)
                .json(&final_request_body)
                .send()
                .await
                .map_err(|e| format!("Failed to send final request: {}", e))?;
            
            let final_response_text = final_response.text().await
                .map_err(|e| format!("Failed to read final response: {}", e))?;
            
            let final_completion: ChatCompletionResponse = serde_json::from_str(&final_response_text)
                .map_err(|e| format!("Failed to parse final response: {}", e))?;
            
            if let Some(final_choice) = final_completion.choices.first() {
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

fn parse_analysis(analysis: &str) -> (Vec<Finding>, Vec<String>) {
    let mut findings = Vec::new();
    let mut recommendations = Vec::new();
    
    let lines: Vec<&str> = analysis.lines().collect();
    let mut in_findings = false;
    let mut in_recommendations = false;
    
    for line in lines {
        let lower = line.to_lowercase();
        
        if lower.contains("finding") || lower.contains("issue") || lower.contains("problem") {
            in_findings = true;
            in_recommendations = false;
        } else if lower.contains("recommendation") || lower.contains("suggest") || lower.contains("solution") {
            in_findings = false;
            in_recommendations = true;
        }
        
        if in_findings && !line.trim().is_empty() && line.trim().len() > 5 {
            // Try to parse severity from the line
            let severity = if lower.contains("critical") || lower.contains("severe") {
                "Critical"
            } else if lower.contains("warning") || lower.contains("warn") || lower.contains("caution") {
                "Warning"
            } else {
                "Info"
            };
            
            // Extract category based on keywords
            let category = if lower.contains("disk") || lower.contains("storage") || lower.contains("drive") {
                "Storage"
            } else if lower.contains("network") || lower.contains("internet") || lower.contains("connection") {
                "Network"
            } else if lower.contains("driver") {
                "Drivers"
            } else if lower.contains("memory") || lower.contains("ram") {
                "Memory"
            } else if lower.contains("cpu") || lower.contains("processor") {
                "CPU"
            } else if lower.contains("security") || lower.contains("antivirus") || lower.contains("firewall") {
                "Security"
            } else {
                "System"
            };
            
            if !line.trim().starts_with("#") && !line.trim().starts_with("*") {
                findings.push(Finding {
                    category: category.to_string(),
                    severity: severity.to_string(),
                    description: line.trim().to_string(),
                    details: None,
                });
            }
        }
        
        if in_recommendations && !line.trim().is_empty() && line.trim().len() > 5 {
            if !line.trim().starts_with("#") && !line.trim().starts_with("*") {
                recommendations.push(line.trim().to_string());
            }
        }
    }
    
    (findings, recommendations)
}

// Legacy command for compatibility
#[tauri::command]
pub async fn analyze_with_openai(
    request: OpenAIRequest,
    app_handle: tauri::AppHandle,
) -> Result<OpenAIResponse, String> {
    // Forward to the new implementation
    let result = analyze_system_with_ai(request.api_key, request.prompt, app_handle).await?;
    
    // Convert the result to the expected format
    let response = OpenAIResponse {
        analysis: result.get("analysis")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        diagnostics_run: result.get("diagnostics_run")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect())
            .unwrap_or_default(),
        findings: result.get("findings")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        recommendations: result.get("recommendations")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect())
            .unwrap_or_default(),
    };
    
    Ok(response)
}