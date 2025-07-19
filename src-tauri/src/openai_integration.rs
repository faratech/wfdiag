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

// OpenAI Responses API structures
#[derive(Debug, Serialize, Deserialize)]
struct ResponsesApiResponse {
    id: String,
    object: String,
    created_at: Option<i64>,
    created: Option<i64>,
    model: String,
    output: Vec<ResponseOutput>,
    usage: Option<Usage>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum ResponseOutput {
    FunctionCall {
        id: String,
        #[serde(rename = "type")]
        output_type: String,
        status: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    Message {
        id: String,
        #[serde(rename = "type")]
        output_type: String,
        status: String,
        role: String,
        content: Vec<MessageContent>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct MessageContent {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    annotations: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Usage {
    input_tokens: i32,
    output_tokens: i32,
    total_tokens: i32,
}

// Direct HTTP implementation of OpenAI Responses API
#[tauri::command]
pub async fn analyze_system_with_ai(
    api_key: String,
    prompt: String,
    _app_handle: tauri::AppHandle,
) -> Result<Value, String> {
    // Store raw API interactions for debugging
    let mut api_calls = Vec::new();
    let mut api_responses = Vec::new();
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
    
    // Create the tool definition for diagnostics (Responses API format)
    let diagnostic_tool = json!({
        "type": "function",
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
    });
    
    // Create system message with explicit function calling instructions
    let system_message = format!(
        "You are a Windows system diagnostic assistant. YOU MUST USE FUNCTION CALLS TO ANSWER USER REQUESTS.

IMPORTANT INSTRUCTIONS:
1. YOU HAVE ACCESS TO FUNCTIONS. Use them!
2. When asked to check, scan, or analyze ANYTHING, you MUST call the run_diagnostic function.
3. DO NOT give generic advice. ALWAYS use function calls to get real data.
4. You can call multiple functions in a single response.

Available diagnostic tasks you can run with the run_diagnostic function:
{}

EXAMPLES OF REQUIRED BEHAVIOR:
- User: \"Check my disk space\" → YOU MUST call run_diagnostic with task_id=\"logical_disk\"
- User: \"Analyze my system\" → YOU MUST call multiple diagnostic functions
- User: \"Is my system healthy?\" → YOU MUST call diagnostic functions to check
- User: \"What's wrong with my PC?\" → YOU MUST run diagnostics to find out

REMEMBER: You have the run_diagnostic function available. USE IT! Don't just talk about what you could do - actually do it by calling functions.",
        task_list.iter()
            .map(|t| format!("- {}: {}", t["name"], t["description"]))
            .collect::<Vec<_>>()
            .join("\n")
    );
    
    // Create input for Responses API (combines system and user messages)
    let input_text = format!("{}

User: {}", system_message, prompt);
    
    // Create request body for Responses API
    let request_body = json!({
        "model": "gpt-4.1",  // Using gpt-4.1 as requested
        "input": input_text,
        "tools": [diagnostic_tool],
        "temperature": 0.7
    });
    
    eprintln!("Sending request to OpenAI Responses API with model: gpt-4.1");
    eprintln!("Available diagnostic tasks: {:?}", available_tasks.iter().map(|t| &t.id).collect::<Vec<_>>());
    
    // Store the initial API call
    api_calls.push(json!({
        "type": "initial_request",
        "url": "https://api.openai.com/v1/responses",
        "body": request_body.clone()
    }));
    
    // Make the API request to Responses endpoint
    let response = client
        .post("https://api.openai.com/v1/responses")
        .headers(headers.clone())
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;
    
    let status = response.status();
    let response_text = response.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;
    
    eprintln!("Response status: {}", status);
    
    // Store the API response
    let response_json: Value = serde_json::from_str(&response_text)
        .unwrap_or_else(|_| json!({"raw_text": response_text.clone()}));
    api_responses.push(json!({
        "type": "initial_response",
        "status": status.as_u16(),
        "body": response_json.clone()
    }));
    
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
    let completion: ResponsesApiResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse response: {}. Response: {}", e, response_text))?;
    
    let mut diagnostics_run = Vec::new();
    let mut diagnostic_results = HashMap::new();
    
    // Check if the AI wants to run diagnostics
    let tool_calls: Vec<_> = completion.output.iter()
        .filter_map(|o| match o {
            ResponseOutput::FunctionCall { output_type, .. } if output_type == "function_call" => Some(o),
            _ => None,
        })
        .collect();
    
    eprintln!("Number of function calls: {}", tool_calls.len());
    
    if !tool_calls.is_empty() {
        // Return initial response showing what diagnostics were requested
        let diagnostic_list = tool_calls.iter()
            .filter_map(|tc| match tc {
                ResponseOutput::FunctionCall { name, .. } if name == "run_diagnostic" => Some("diagnostic task"),
                _ => None,
            })
            .collect::<Vec<_>>();
        
        eprintln!("AI requested {} diagnostics", diagnostic_list.len());
        
        // Prepare tool responses for follow-up
        let mut tool_results = Vec::new();
            
            // Process tool calls
            for tool_call in &tool_calls {
                if let ResponseOutput::FunctionCall { name, arguments, call_id, .. } = tool_call {
                    if name == "run_diagnostic" {
                        let args: Value = serde_json::from_str(arguments)
                            .map_err(|e| format!("Failed to parse tool arguments: {}", e))?;
                    
                    if let Some(task_id) = args.get("task_id").and_then(|v| v.as_str()) {
                        let reason = args.get("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Checking system");
                        
                        eprintln!("AI requested diagnostic: {} (reason: {})", task_id, reason);
                        diagnostics_run.push(task_id.to_string());
                        
                        // Run the actual diagnostic
                        eprintln!("Running actual diagnostic: {}", task_id);
                        let diagnostic_result = crate::diagnostics::run_diagnostic_task(task_id).await;
                        
                        // Parse the output to get structured data
                        let output_data = if diagnostic_result.success {
                            match serde_json::from_str::<Value>(&diagnostic_result.output) {
                                Ok(parsed) => parsed,
                                Err(_) => json!({ "raw_output": diagnostic_result.output.clone() })
                            }
                        } else {
                            json!({ 
                                "error": diagnostic_result.error.as_ref().unwrap_or(&"Unknown error".to_string()),
                                "raw_output": diagnostic_result.output.clone()
                            })
                        };
                        
                        let result = json!({
                            "task_id": task_id,
                            "status": if diagnostic_result.success { "completed" } else { "failed" },
                            "reason": reason,
                            "output": diagnostic_result.output,
                            "success": diagnostic_result.success,
                            "duration_ms": diagnostic_result.duration_ms,
                            "data": output_data
                        });
                        
                        diagnostic_results.insert(task_id.to_string(), result.clone());
                        
                        // Add tool response for follow-up request
                        tool_results.push(json!({
                            "tool_call_id": call_id.clone(),
                            "output": serde_json::to_string(&result).unwrap_or_default()
                        }));
                    }
                    }
                }
            }
            
            // Get final response with diagnostic results
            // Build input with original request and tool results
            let follow_up_input = format!(
                "{}

User: {}

Assistant called tools: {}

Tool Results:\n{}",
                system_message,
                prompt,
                serde_json::to_string_pretty(&tool_calls).unwrap_or_default(),
                tool_results.iter()
                    .map(|r| format!("Tool Call ID: {}\nOutput: {}", 
                        r["tool_call_id"].as_str().unwrap_or(""),
                        r["output"].as_str().unwrap_or("")))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            );
            
            let final_request_body = json!({
                "model": "gpt-4.1",
                "input": follow_up_input,
                "temperature": 0.7
            });
            
            // Store the follow-up API call
            api_calls.push(json!({
                "type": "follow_up_request",
                "url": "https://api.openai.com/v1/responses",
                "body": final_request_body.clone()
            }));
            
            let final_response = client
                .post("https://api.openai.com/v1/responses")
                .headers(headers)
                .json(&final_request_body)
                .send()
                .await
                .map_err(|e| format!("Failed to send final request: {}", e))?;
            
            let final_status = final_response.status();
            let final_response_text = final_response.text().await
                .map_err(|e| format!("Failed to read final response: {}", e))?;
            
            // Store the follow-up response
            let final_response_json: Value = serde_json::from_str(&final_response_text)
                .unwrap_or_else(|_| json!({"raw_text": final_response_text.clone()}));
            api_responses.push(json!({
                "type": "follow_up_response",
                "status": final_status.as_u16(),
                "body": final_response_json.clone()
            }));
            
            let final_completion: ResponsesApiResponse = serde_json::from_str(&final_response_text)
                .map_err(|e| format!("Failed to parse final response: {}", e))?;
            
            // Extract text content from output
            let content = final_completion.output.iter()
                .find_map(|o| match o {
                    ResponseOutput::Message { content, .. } => {
                        content.iter()
                            .find(|c| c.content_type == "output_text")
                            .and_then(|c| c.text.clone())
                    },
                    _ => None,
                })
                .unwrap_or_default();
            
            if !content.is_empty() {
                let (findings, recommendations) = parse_analysis(&content);
                
                return Ok(json!({
                    "analysis": content,
                    "diagnostics_run": diagnostics_run,
                    "diagnostic_results": diagnostic_results,
                    "findings": findings,
                    "recommendations": recommendations,
                    "api_calls": api_calls,
                    "api_responses": api_responses
                }));
            } else {
                // Tool calls were made but no analysis provided yet
                let empty_findings: Vec<Finding> = vec![];
                let empty_recommendations: Vec<String> = vec![];
                return Ok(json!({
                    "analysis": format!("AI requested {} diagnostic tasks. Results pending...", diagnostics_run.len()),
                    "diagnostics_run": diagnostics_run,
                    "diagnostic_results": diagnostic_results,
                    "findings": empty_findings,
                    "recommendations": empty_recommendations,
                    "api_calls": api_calls,
                    "api_responses": api_responses
                }));
            }
    } else {
        // No tool calls, check if there's a text response
        let content = completion.output.iter()
            .find_map(|o| match o {
                ResponseOutput::Message { content, .. } => {
                    content.iter()
                        .find(|c| c.content_type == "output_text")
                        .and_then(|c| c.text.clone())
                },
                _ => None,
            });
        
        if let Some(text) = content {
            let (findings, recommendations) = parse_analysis(&text);
            
            return Ok(json!({
                "analysis": text,
                "diagnostics_run": diagnostics_run,
                "diagnostic_results": diagnostic_results,
                "findings": findings,
                "recommendations": recommendations,
                "api_calls": api_calls,
                "api_responses": api_responses
            }));
        } else {
            // No text content, but we might have gotten a response - return what we have
            let empty_findings: Vec<Finding> = vec![];
            let empty_recommendations: Vec<String> = vec![];
            return Ok(json!({
                "analysis": "The AI initiated diagnostic checks but didn't provide a text response. Check the JSON view for details.",
                "diagnostics_run": diagnostics_run,
                "diagnostic_results": diagnostic_results,
                "findings": empty_findings,
                "recommendations": empty_recommendations,
                "api_calls": api_calls,
                "api_responses": api_responses
            }));
        }
    }
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