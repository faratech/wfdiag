use anyhow::Result;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyRequest {
    pub message: String,
    pub instructions: Option<String>,
    pub timeout: Option<u64>,
    pub stream: Option<bool>,
    pub model: Option<String>,
    pub previous_response_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyResponse {
    pub id: Option<String>,
    pub output: Option<Vec<OutputContent>>,
    pub final_output: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputContent {
    pub content: Vec<ContentItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentItem {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticRequest {
    pub task_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticResult {
    pub task_id: String,
    pub status: String,
    pub output: String,
    pub timestamp: String,
}

pub struct WindowsForumProxy {
    client: reqwest::Client,
    base_url: String,
    session_cookies: Option<String>,
    oauth_token: Option<String>,
}

impl WindowsForumProxy {
    pub fn new(session_cookies: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        
        Self {
            client,
            base_url: "https://windowsforum.com".to_string(),
            session_cookies,
            oauth_token: None,
        }
    }
    
    pub fn new_with_oauth(oauth_token: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        
        Self {
            client,
            base_url: "https://windowsforum.com".to_string(),
            session_cookies: None,
            oauth_token: Some(oauth_token),
        }
    }

    pub async fn analyze_system(
        &self,
        prompt: String,
        diagnostic_results: Option<HashMap<String, DiagnosticResult>>,
    ) -> Result<ProxyResponse> {
        let mut instructions = format!(
            "You are a Windows system diagnostic expert analyzing system information. \
            User is running Windows Forum Diagnostics Tool."
        );

        if let Some(results) = diagnostic_results {
            instructions.push_str("\n\nDiagnostic Results:\n");
            for (task_id, result) in results.iter() {
                instructions.push_str(&format!(
                    "\n{}:\n{}\n",
                    task_id, result.output
                ));
            }
        }

        let request_body = ProxyRequest {
            message: prompt,
            instructions: Some(instructions),
            timeout: Some(60000),
            stream: Some(false),
            model: Some("gpt-4.1".to_string()),
            previous_response_id: None,
        };

        let mut request = self.client
            .post(format!("{}/chat.php", self.base_url))
            .json(&request_body)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Origin", &self.base_url)
            .header("Referer", format!("{}/", self.base_url));

        // Use OAuth token if available, otherwise use cookies
        if let Some(token) = &self.oauth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        } else if let Some(cookies) = &self.session_cookies {
            request = request.header("Cookie", cookies);
        }

        let response = request.send().await?;
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "WindowsForum proxy error: {}",
                response.status()
            ));
        }

        let proxy_response: ProxyResponse = response.json().await?;
        Ok(proxy_response)
    }

    pub async fn check_authentication(&self) -> Result<bool> {
        let request_body = json!({
            "action": "getUserData"
        });

        let mut request = self.client
            .post(format!("{}/chat.php", self.base_url))
            .json(&request_body)
            .header("X-Requested-With", "XMLHttpRequest");

        // Use OAuth token if available, otherwise use cookies
        if let Some(token) = &self.oauth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        } else if let Some(cookies) = &self.session_cookies {
            request = request.header("Cookie", cookies);
        }

        let response = request.send().await?;
        
        if !response.status().is_success() {
            return Ok(false);
        }

        let user_data: Value = response.json().await?;
        
        Ok(user_data.get("user_id").is_some() && 
           !user_data["user_id"].as_str().unwrap_or("").starts_with("guest_"))
    }
}

#[tauri::command]
pub async fn analyze_with_forum_proxy(
    prompt: String,
    diagnostic_results: Option<HashMap<String, DiagnosticResult>>,
    cookies: Option<String>,
) -> Result<Value, String> {
    let proxy = WindowsForumProxy::new(cookies);
    
    match proxy.check_authentication().await {
        Ok(true) => {},
        Ok(false) => return Err("Not authenticated with WindowsForum. Please log in.".to_string()),
        Err(e) => return Err(format!("Failed to check authentication: {}", e)),
    }
    
    match proxy.analyze_system(prompt, diagnostic_results).await {
        Ok(response) => {
            let analysis = if let Some(final_output) = response.final_output {
                final_output
            } else if let Some(output) = response.output {
                output.iter()
                    .flat_map(|o| &o.content)
                    .filter_map(|c| c.text.as_ref())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                "No analysis available".to_string()
            };

            Ok(json!({
                "analysis": analysis,
                "response_id": response.id,
                "source": "WindowsForum AI Proxy"
            }))
        }
        Err(e) => Err(format!("Analysis failed: {}", e))
    }
}

#[tauri::command]
pub async fn analyze_with_oauth(
    prompt: String,
    diagnostic_results: Option<HashMap<String, DiagnosticResult>>,
    oauth_state: tauri::State<'_, std::sync::Mutex<crate::oauth::OAuthState>>,
) -> Result<Value, String> {
    // Get OAuth token
    let token = {
        let state = oauth_state.lock().unwrap();
        if !state.is_authenticated() {
            return Err("Not authenticated. Please sign in with OAuth.".to_string());
        }
        state.token.as_ref()
            .map(|t| t.access_token.clone())
            .ok_or_else(|| "No OAuth token available".to_string())?
    };
    
    let proxy = WindowsForumProxy::new_with_oauth(token);
    
    match proxy.analyze_system(prompt, diagnostic_results).await {
        Ok(response) => {
            let analysis = if let Some(final_output) = response.final_output {
                final_output
            } else if let Some(output) = response.output {
                output.iter()
                    .flat_map(|o| &o.content)
                    .filter_map(|c| c.text.as_ref())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                "No analysis available".to_string()
            };

            Ok(json!({
                "analysis": analysis,
                "response_id": response.id,
                "source": "WindowsForum AI Proxy (OAuth)"
            }))
        }
        Err(e) => Err(format!("Analysis failed: {}", e))
    }
}

#[tauri::command]
pub async fn check_forum_authentication(cookies: Option<String>) -> Result<bool, String> {
    let proxy = WindowsForumProxy::new(cookies);
    proxy.check_authentication()
        .await
        .map_err(|e| format!("Authentication check failed: {}", e))
}

#[tauri::command]
pub async fn run_diagnostic_with_analysis(
    task_id: String,
    cookies: Option<String>,
) -> Result<Value, String> {
    use crate::diagnostics;
    
    let task = diagnostics::get_all_tasks()
        .into_iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| format!("Task {} not found", task_id))?;
    
    let diagnostic_result = diagnostics::run_diagnostic_task(&task_id).await;
    
    let output_str = serde_json::to_string_pretty(&diagnostic_result.output)
        .unwrap_or_else(|_| "Failed to serialize output".to_string());
    
    let mut results = HashMap::new();
    results.insert(
        task_id.clone(),
        DiagnosticResult {
            task_id: task_id.clone(),
            status: if diagnostic_result.success { "completed" } else { "failed" }.to_string(),
            output: output_str.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        },
    );
    
    let prompt = format!(
        "Analyze the results from the {} diagnostic and provide insights:",
        task.description
    );
    
    let proxy = WindowsForumProxy::new(cookies);
    
    match proxy.analyze_system(prompt, Some(results)).await {
        Ok(response) => {
            let analysis = if let Some(final_output) = response.final_output {
                final_output
            } else if let Some(output) = response.output {
                output.iter()
                    .flat_map(|o| &o.content)
                    .filter_map(|c| c.text.as_ref())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                "No analysis available".to_string()
            };

            Ok(json!({
                "task_id": task_id,
                "diagnostic_output": output_str,
                "ai_analysis": analysis,
                "response_id": response.id,
            }))
        }
        Err(e) => {
            Ok(json!({
                "task_id": task_id,
                "diagnostic_output": output_str,
                "ai_analysis": format!("Analysis unavailable: {}", e),
                "response_id": null,
            }))
        }
    }
}