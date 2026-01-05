//! AI Integration UI - Phi Silica and OpenAI support for diagnostic analysis

use crate::{AiProvider, WfDiagApp, AiAnalysisResult};
use eframe::egui::{self, Color32, RichText, Margin};
use tokio::sync::mpsc;

/// Check if AI is available (either Phi Silica or OpenAI configured)
pub fn is_ai_available(app: &WfDiagApp) -> bool {
    if !app.settings.ai_enabled {
        return false;
    }

    match app.settings.ai_provider {
        AiProvider::Auto => {
            // Phi Silica available OR OpenAI key configured
            app.ai_phi_silica_status.as_ref().map(|s| s.available).unwrap_or(false)
                || app.settings.openai_api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false)
        }
        AiProvider::PhiSilica => {
            app.ai_phi_silica_status.as_ref().map(|s| s.available).unwrap_or(false)
        }
        AiProvider::OpenAI => {
            app.settings.openai_api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false)
        }
    }
}

/// Get the active AI provider name for display
pub fn get_active_provider_name(app: &WfDiagApp) -> &'static str {
    if !app.settings.ai_enabled {
        return "Disabled";
    }

    match app.settings.ai_provider {
        AiProvider::Auto => {
            if app.ai_phi_silica_status.as_ref().map(|s| s.available).unwrap_or(false) {
                "Phi Silica"
            } else if app.settings.openai_api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false) {
                "OpenAI"
            } else {
                "None"
            }
        }
        AiProvider::PhiSilica => "Phi Silica",
        AiProvider::OpenAI => "OpenAI",
    }
}

/// Start checking Phi Silica availability in background
pub fn start_phi_silica_check(app: &mut WfDiagApp) {
    if app.ai_status_checking {
        return;
    }

    app.ai_status_checking = true;
    let (tx, rx) = mpsc::channel(1);
    app.ai_status_rx = Some(rx);

    let runtime = app.runtime.clone();
    std::thread::spawn(move || {
        runtime.block_on(async {
            match wfdiag_tauri::phi_silica::check_phi_silica_available().await {
                Ok(status) => {
                    let _ = tx.send(status).await;
                }
                Err(e) => {
                    let status = wfdiag_tauri::phi_silica::PhiSilicaStatus {
                        available: false,
                        message: e,
                        error_code: None,
                        windows_build: None,
                        ready_state: None,
                    };
                    let _ = tx.send(status).await;
                }
            }
        });
    });
}

/// Process AI status updates
pub fn process_ai_status_updates(app: &mut WfDiagApp) {
    if let Some(ref mut rx) = app.ai_status_rx {
        match rx.try_recv() {
            Ok(status) => {
                app.ai_phi_silica_status = Some(status);
                app.ai_status_checking = false;
                app.ai_status_rx = None;
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {
                app.ai_status_checking = false;
                app.ai_status_rx = None;
            }
        }
    }
}

/// Process AI analysis results
pub fn process_ai_analysis_updates(app: &mut WfDiagApp) {
    if let Some(ref mut rx) = app.ai_analysis_rx {
        // Process all available results
        loop {
            match rx.try_recv() {
                Ok(result) => {
                    app.ai_loading.remove(&result.task_id);
                    match result.interpretation {
                        Ok(interpretation) => {
                            app.ai_interpretations.insert(result.task_id.clone(), interpretation);
                            app.ai_errors.remove(&result.task_id);
                        }
                        Err(error) => {
                            app.ai_errors.insert(result.task_id.clone(), error);
                        }
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    app.ai_analysis_rx = None;
                    break;
                }
            }
        }
    }
}

/// Request AI interpretation for a diagnostic task
pub fn request_interpretation(app: &mut WfDiagApp, task_id: &str, task_name: &str, output: &str) {
    // Check if already cached or loading
    if app.ai_interpretations.contains_key(task_id) || app.ai_loading.get(task_id).copied().unwrap_or(false) {
        return;
    }

    // Mark as loading
    app.ai_loading.insert(task_id.to_string(), true);
    app.ai_errors.remove(task_id);

    // Create channel if needed
    let tx = if app.ai_analysis_rx.is_none() {
        let (tx, rx) = mpsc::channel(32);
        app.ai_analysis_rx = Some(rx);
        tx
    } else {
        // We need to keep the same sender - this is a limitation
        // For now, create new channel each time (not ideal but works)
        let (tx, rx) = mpsc::channel(32);
        app.ai_analysis_rx = Some(rx);
        tx
    };

    let task_id = task_id.to_string();
    let task_name = task_name.to_string();
    let output = output.to_string();
    let api_key = app.settings.openai_api_key.clone();
    let provider = app.settings.ai_provider;
    let phi_available = app.ai_phi_silica_status.as_ref().map(|s| s.available).unwrap_or(false);
    let runtime = app.runtime.clone();

    std::thread::spawn(move || {
        runtime.block_on(async {
            let result = analyze_diagnostic(&task_id, &task_name, &output, api_key, provider, phi_available).await;
            let _ = tx.send(AiAnalysisResult {
                task_id,
                interpretation: result,
            }).await;
        });
    });
}

/// Perform AI analysis of a diagnostic
async fn analyze_diagnostic(
    task_id: &str,
    task_name: &str,
    output: &str,
    api_key: Option<String>,
    provider: AiProvider,
    phi_available: bool,
) -> Result<String, String> {
    // Determine which provider to use
    let use_phi = match provider {
        AiProvider::Auto => phi_available,
        AiProvider::PhiSilica => phi_available,
        AiProvider::OpenAI => false,
    };

    if use_phi {
        // Use Phi Silica
        analyze_with_phi_silica(task_name, output).await
    } else if let Some(key) = api_key {
        if key.is_empty() {
            return Err("OpenAI API key not configured".to_string());
        }
        // Use OpenAI
        analyze_with_openai(&key, task_name, output).await
    } else {
        Err("No AI provider available. Configure OpenAI API key or use a Copilot+ PC.".to_string())
    }
}

/// Maximum characters for diagnostic output sent to AI
/// OpenAI can handle much more, but we limit to keep responses focused
const MAX_DIAGNOSTIC_OUTPUT_CHARS: usize = 12000;

/// Maximum characters for Phi Silica (very limited context ~4K tokens)
const PHI_SILICA_MAX_CHARS: usize = 1200;

/// Analyze using Phi Silica (on-device AI)
async fn analyze_with_phi_silica(task_name: &str, output: &str) -> Result<String, String> {
    // Convert JSON to readable text for much better token efficiency
    let readable_output = json_to_readable_text(output, PHI_SILICA_MAX_CHARS);

    // Check if output indicates success/no issues
    let output_lower = output.to_lowercase();
    let seems_ok = output_lower.contains("no issues") ||
                   output_lower.contains("passed") ||
                   output_lower.contains("healthy") ||
                   output_lower.contains("[]") ||
                   output.trim().is_empty();

    // Build prompt with clear instructions
    let prompt = if seems_ok || readable_output.is_empty() {
        format!(
            "Windows diagnostic '{}' completed successfully with no issues detected. \
             Write one sentence confirming the system is healthy in this area.",
            task_name
        )
    } else {
        format!(
            "Windows diagnostic results for '{}':\n{}\n\n\
             Explain what this means in 2-3 sentences.",
            task_name,
            readable_output
        )
    };

    wfdiag_tauri::phi_silica::analyze_with_phi_silica(prompt)
        .await
        .map(|r| r.analysis)
}

/// Convert JSON output to human-readable text for Phi Silica
fn json_to_readable_text(output: &str, max_chars: usize) -> String {
    let trimmed = output.trim();

    // Try to parse as JSON
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let text = render_json_value(&json, 0);
            return truncate_text(&text, max_chars);
        }
    }

    // Not JSON - just truncate
    truncate_text(output, max_chars)
}

/// Render JSON value as readable text
fn render_json_value(value: &serde_json::Value, depth: usize) -> String {
    if depth > 2 {
        return String::new();
    }

    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(b) => if *b { "Yes" } else { "No" }.to_string(),
        serde_json::Value::Number(n) => format_number(n),
        serde_json::Value::String(s) => {
            if s.len() > 100 { format!("{}...", &s[..50]) } else { s.clone() }
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() { return String::new(); }
            let items: Vec<String> = arr.iter().take(3)
                .filter_map(|item| {
                    let rendered = render_json_value(item, depth + 1);
                    if rendered.is_empty() { None } else { Some(rendered) }
                })
                .collect();
            if arr.len() > 3 {
                format!("{}, ... ({} total)", items.join("; "), arr.len())
            } else {
                items.join("; ")
            }
        }
        serde_json::Value::Object(obj) => render_object(obj, depth),
    }
}

/// Render JSON object with priority fields
fn render_object(obj: &serde_json::Map<String, serde_json::Value>, depth: usize) -> String {
    let priority = ["Name", "Caption", "Status", "Size", "Capacity", "Speed",
                   "Manufacturer", "Model", "FreeSpace", "TotalPhysicalMemory"];

    let mut parts = Vec::new();

    // Priority fields first
    for field in priority {
        if let Some(val) = obj.get(field) {
            let rendered = render_json_value(val, depth + 1);
            if !rendered.is_empty() {
                // Format bytes nicely
                let display = if field.contains("Size") || field.contains("Memory") || field.contains("Space") || field.contains("Capacity") {
                    format_bytes_str(&rendered)
                } else {
                    rendered
                };
                parts.push(format!("{}: {}", field, display));
            }
        }
    }

    // Add a few other fields if we have room
    let max_fields = 6;
    for (key, val) in obj.iter() {
        if parts.len() >= max_fields { break; }
        if priority.contains(&key.as_str()) { continue; }
        if key.starts_with("__") || key.contains("Path") || key.contains("Class") { continue; }

        let rendered = render_json_value(val, depth + 1);
        if !rendered.is_empty() && rendered.len() < 80 {
            parts.push(format!("{}: {}", key, rendered));
        }
    }

    parts.join(", ")
}

/// Format number, converting large values to GB/MB
fn format_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_u64() {
        if i > 1_000_000_000 { format!("{:.1} GB", i as f64 / 1_073_741_824.0) }
        else if i > 1_000_000 { format!("{:.1} MB", i as f64 / 1_048_576.0) }
        else { i.to_string() }
    } else {
        n.to_string()
    }
}

/// Format string as bytes if it looks numeric
fn format_bytes_str(s: &str) -> String {
    if let Ok(bytes) = s.parse::<u64>() {
        if bytes > 1_000_000_000 { return format!("{:.1} GB", bytes as f64 / 1_073_741_824.0); }
        if bytes > 1_000_000 { return format!("{:.1} MB", bytes as f64 / 1_048_576.0); }
    }
    s.to_string()
}

/// Truncate text to max chars
fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        text.to_string()
    } else {
        format!("{}...", &text[..max_chars.saturating_sub(3)])
    }
}

/// Analyze using OpenAI Responses API
async fn analyze_with_openai(api_key: &str, task_name: &str, output: &str) -> Result<String, String> {
    use async_openai::{Client, config::OpenAIConfig};
    use async_openai::types::responses::{CreateResponseArgs, InputParam};

    let config = OpenAIConfig::new().with_api_key(api_key);
    let client = Client::with_config(config);

    // Safely truncate output - OpenAI can handle large context
    let truncated_output = if output.chars().count() > MAX_DIAGNOSTIC_OUTPUT_CHARS {
        let truncated: String = output.chars().take(MAX_DIAGNOSTIC_OUTPUT_CHARS).collect();
        format!("{}\n\n[... output truncated, {} more characters ...]", truncated, output.chars().count() - MAX_DIAGNOSTIC_OUTPUT_CHARS)
    } else {
        output.to_string()
    };

    // Sanitize content - remove null bytes and control chars that might cause API issues
    let sanitized_output: String = truncated_output
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\r' || *c == '\t')
        .collect();

    let prompt = format!(
        "You are a Windows system diagnostic expert. Analyze diagnostic output and explain what it means in plain language. Be concise (2-3 sentences). Focus on whether this is normal or needs attention.\n\nDiagnostic: {}\n\nResult:\n{}",
        task_name,
        sanitized_output
    );

    let request = CreateResponseArgs::default()
        .model(wfdiag_tauri::openai_integration::OPENAI_MODEL)
        .input(InputParam::Text(prompt))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.responses().create(request).await
        .map_err(|e| {
            // Use debug format to get full error details
            let error_detail = format!("{:?}", e);
            eprintln!("OpenAI API error details: {}", error_detail);

            // Parse specific error types for better user messages
            if error_detail.contains("401") || error_detail.contains("Unauthorized") {
                "Invalid API key. Please check your OpenAI API key.".to_string()
            } else if error_detail.contains("404") || error_detail.contains("model_not_found") {
                format!("Model '{}' not found. Check if it's available on your account.",
                    wfdiag_tauri::openai_integration::OPENAI_MODEL)
            } else if error_detail.contains("429") {
                "Rate limit exceeded. Please wait a moment.".to_string()
            } else if error_detail.contains("insufficient_quota") {
                "Insufficient quota. Check your OpenAI billing.".to_string()
            } else {
                // Show the full error for debugging
                format!("API error: {}", e)
            }
        })?;

    Ok(response.output_text().unwrap_or_default())
}

/// Render AI interpretation panel for a diagnostic
pub fn render_ai_panel(
    ui: &mut egui::Ui,
    app: &mut WfDiagApp,
    task_id: &str,
    task_name: &str,
    output: &str,
    is_success: bool,
) {
    // Clone data to avoid borrow conflicts
    let ai_enabled = app.settings.ai_enabled;
    let ai_available = is_ai_available(app);
    let is_loading = app.ai_loading.get(task_id).copied().unwrap_or(false);
    let cached = app.ai_interpretations.get(task_id).cloned();
    let error = app.ai_errors.get(task_id).cloned();
    let provider = get_active_provider_name(app).to_string();

    // Track actions to take after rendering
    let mut should_retry = false;
    let mut should_explain = false;

    // AI panel frame
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(99, 102, 241, 20))
        .corner_radius(6.0)
        .inner_margin(Margin::same(10))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(99, 102, 241, 50)))
        .show(ui, |ui| {
            // Header
            ui.horizontal(|ui| {
                ui.label(RichText::new("✨").size(14.0));
                ui.label(RichText::new("AI Interpretation").size(11.0).strong().color(Color32::from_rgb(99, 102, 241)));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ai_available {
                        ui.label(RichText::new(&provider).size(9.0).weak());
                    }
                });
            });

            ui.add_space(6.0);

            // Content
            if !ai_enabled {
                ui.label(RichText::new("AI analysis is disabled. Enable in Settings.").size(11.0).weak());
            } else if !ai_available {
                ui.label(RichText::new("No AI provider available. Configure OpenAI in Settings or use a Copilot+ PC.").size(11.0).weak());
            } else if is_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(format!("Analyzing with {}...", provider)).size(11.0).weak());
                });
            } else if let Some(ref interpretation) = cached {
                ui.label(RichText::new(interpretation).size(11.0));
            } else if let Some(ref err) = error {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").size(11.0).color(Color32::from_rgb(220, 100, 100)));
                    ui.label(RichText::new(err).size(11.0).color(Color32::from_rgb(220, 100, 100)));
                });
                if ui.small_button("Retry").clicked() {
                    should_retry = true;
                }
            } else {
                // Show prompt to analyze
                let default_msg = if is_success {
                    "This check passed. Click to get an AI explanation."
                } else {
                    "This check needs attention. Click for AI guidance."
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(default_msg).size(11.0).weak());
                    if ui.small_button("Explain").clicked() {
                        should_explain = true;
                    }
                });
            }
        });

    // Handle deferred actions
    if should_retry {
        app.ai_errors.remove(task_id);
        request_interpretation(app, task_id, task_name, output);
    }
    if should_explain {
        request_interpretation(app, task_id, task_name, output);
    }
}

/// Render AI status indicator for toolbar/settings
pub fn render_ai_status_badge(ui: &mut egui::Ui, app: &WfDiagApp) {
    let ai_available = is_ai_available(app);
    let provider = get_active_provider_name(app);

    let (color, text) = if !app.settings.ai_enabled {
        (Color32::GRAY, "AI Off")
    } else if ai_available {
        (Color32::from_rgb(99, 102, 241), provider)
    } else {
        (Color32::from_rgb(200, 150, 50), "AI N/A")
    };

    egui::Frame::new()
        .fill(color.linear_multiply(0.2))
        .corner_radius(4.0)
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(format!("✨ {}", text)).size(10.0).color(color));
        });
}

// ============================================================================
// System Monitoring AI Analysis
// ============================================================================

/// Request AI analysis of system monitoring stats
pub fn request_monitoring_analysis(app: &mut WfDiagApp) {
    const MONITORING_AI_KEY: &str = "__monitoring_analysis__";

    // Check if already loading
    if app.ai_loading.get(MONITORING_AI_KEY).copied().unwrap_or(false) {
        return;
    }

    #[cfg(windows)]
    {
        // Build stats summary from current monitoring state
        let stats = match &app.monitoring_state.stats {
            Some(s) => s.clone(),
            None => return,
        };

        let stats_text = format!(
            "Current System Stats:\n\
             - CPU: {:.1}% utilization at {} MHz\n\
             - Memory: {:.1}% used ({:.1} GB / {:.1} GB)\n\
             - Swap: {:.1}% used ({:.1} GB / {:.1} GB)\n\
             - Network: ↑{:.1} KB/s ↓{:.1} KB/s\n\
             - Disks: {}\n\
             {}",
            stats.cpu_utilization,
            stats.cpu_frequency,
            stats.memory_utilization,
            stats.memory_used_gb,
            stats.memory_total_gb,
            stats.swap_utilization,
            stats.swap_used_gb,
            stats.swap_total_gb,
            stats.network_upload_kb,
            stats.network_download_kb,
            stats.disks.iter().map(|d| format!("{}: {:.1}% full ({:.1}/{:.1} GB)",
                d.mount_point, d.utilization, d.used_gb, d.total_gb)).collect::<Vec<_>>().join(", "),
            if stats.npu_available {
                format!("- NPU: {} ({})",
                    stats.npu_name.as_deref().unwrap_or("Available"),
                    stats.npu_utilization.map(|u| format!("{:.1}% util", u)).unwrap_or_else(|| "metrics N/A".to_string()))
            } else {
                String::new()
            }
        );

        // Mark as loading
        app.ai_loading.insert(MONITORING_AI_KEY.to_string(), true);
        app.ai_errors.remove(MONITORING_AI_KEY);

        // Create channel
        let (tx, rx) = mpsc::channel(1);
        app.ai_analysis_rx = Some(rx);

        let api_key = app.settings.openai_api_key.clone();
        let provider = app.settings.ai_provider;
        let phi_available = app.ai_phi_silica_status.as_ref().map(|s| s.available).unwrap_or(false);
        let runtime = app.runtime.clone();

        std::thread::spawn(move || {
            runtime.block_on(async {
                let result = analyze_monitoring_stats(&stats_text, api_key, provider, phi_available).await;
                let _ = tx.send(crate::AiAnalysisResult {
                    task_id: MONITORING_AI_KEY.to_string(),
                    interpretation: result,
                }).await;
            });
        });
    }
}

/// Analyze system monitoring stats with AI
#[cfg(windows)]
async fn analyze_monitoring_stats(
    stats_text: &str,
    api_key: Option<String>,
    provider: crate::AiProvider,
    phi_available: bool,
) -> Result<String, String> {
    let use_phi = match provider {
        crate::AiProvider::Auto => phi_available,
        crate::AiProvider::PhiSilica => phi_available,
        crate::AiProvider::OpenAI => false,
    };

    let prompt = format!(
        "Analyze these Windows system monitoring stats. Provide a brief assessment (3-4 sentences) of:\n\
         1. Overall system health\n\
         2. Any concerns or bottlenecks\n\
         3. Recommendations if any issues are detected\n\n\
         {}",
        stats_text
    );

    if use_phi {
        wfdiag_tauri::phi_silica::analyze_with_phi_silica(prompt)
            .await
            .map(|r| r.analysis)
    } else if let Some(key) = api_key {
        if key.is_empty() {
            return Err("OpenAI API key not configured".to_string());
        }
        analyze_generic_with_openai(&key, &prompt).await
    } else {
        Err("No AI provider available".to_string())
    }
}

/// Generic OpenAI analysis function
async fn analyze_generic_with_openai(api_key: &str, prompt: &str) -> Result<String, String> {
    use async_openai::{Client, config::OpenAIConfig};
    use async_openai::types::responses::{CreateResponseArgs, InputParam};

    let config = OpenAIConfig::new().with_api_key(api_key);
    let client = Client::with_config(config);

    let system_prompt = "You are a Windows system diagnostic expert. Provide concise, actionable insights.";
    let full_prompt = format!("{}\n\n{}", system_prompt, prompt);

    let request = CreateResponseArgs::default()
        .model(wfdiag_tauri::openai_integration::OPENAI_MODEL)
        .input(InputParam::Text(full_prompt))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.responses().create(request).await
        .map_err(|e| format!("API error: {}", e))?;

    Ok(response.output_text().unwrap_or_default())
}

/// Render monitoring AI panel
pub fn render_monitoring_ai_panel(ui: &mut egui::Ui, app: &mut WfDiagApp) {
    const MONITORING_AI_KEY: &str = "__monitoring_analysis__";

    let ai_available = is_ai_available(app);
    let is_loading = app.ai_loading.get(MONITORING_AI_KEY).copied().unwrap_or(false);
    let cached = app.ai_interpretations.get(MONITORING_AI_KEY).cloned();
    let error = app.ai_errors.get(MONITORING_AI_KEY).cloned();
    let provider = get_active_provider_name(app).to_string();
    let has_stats = app.monitoring_state.stats.is_some();

    let mut should_analyze = false;
    let mut should_retry = false;

    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(99, 102, 241, 20))
        .corner_radius(6.0)
        .inner_margin(Margin::same(10))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(99, 102, 241, 50)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("✨").size(14.0));
                ui.label(RichText::new("AI System Analysis").size(11.0).strong().color(Color32::from_rgb(99, 102, 241)));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ai_available {
                        ui.label(RichText::new(&provider).size(9.0).weak());
                    }
                });
            });

            ui.add_space(6.0);

            if !app.settings.ai_enabled {
                ui.label(RichText::new("AI analysis is disabled. Enable in Settings.").size(11.0).weak());
            } else if !ai_available {
                ui.label(RichText::new("No AI provider available.").size(11.0).weak());
            } else if !has_stats {
                ui.label(RichText::new("Start monitoring to enable AI analysis.").size(11.0).weak());
            } else if is_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(format!("Analyzing with {}...", provider)).size(11.0).weak());
                });
            } else if let Some(ref interpretation) = cached {
                ui.label(RichText::new(interpretation).size(11.0));
                ui.add_space(4.0);
                if ui.small_button("Refresh Analysis").clicked() {
                    should_analyze = true;
                }
            } else if let Some(ref err) = error {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").size(11.0).color(Color32::from_rgb(220, 100, 100)));
                    ui.label(RichText::new(err).size(11.0).color(Color32::from_rgb(220, 100, 100)));
                });
                if ui.small_button("Retry").clicked() {
                    should_retry = true;
                }
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Get AI insights on your system performance.").size(11.0).weak());
                    if ui.small_button("Analyze").clicked() {
                        should_analyze = true;
                    }
                });
            }
        });

    if should_analyze || should_retry {
        if should_retry {
            app.ai_errors.remove(MONITORING_AI_KEY);
        }
        app.ai_interpretations.remove(MONITORING_AI_KEY);
        request_monitoring_analysis(app);
    }
}

// ============================================================================
// Process List AI Analysis
// ============================================================================

/// Request AI analysis of running processes
pub fn request_process_analysis(app: &mut WfDiagApp) {
    const PROCESS_AI_KEY: &str = "__process_analysis__";

    if app.ai_loading.get(PROCESS_AI_KEY).copied().unwrap_or(false) {
        return;
    }

    if app.all_processes.is_empty() {
        return;
    }

    // Get top 10 processes by CPU
    let mut by_cpu = app.all_processes.clone();
    by_cpu.sort_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap_or(std::cmp::Ordering::Equal));
    let top_cpu: Vec<_> = by_cpu.into_iter().take(10).collect();

    // Get top 10 processes by Memory
    let mut by_mem = app.all_processes.clone();
    by_mem.sort_by(|a, b| b.memory_mb.partial_cmp(&a.memory_mb).unwrap_or(std::cmp::Ordering::Equal));
    let top_mem: Vec<_> = by_mem.into_iter().take(10).collect();

    let process_text = format!(
        "Running Processes Analysis:\n\n\
         Top 10 by CPU:\n{}\n\n\
         Top 10 by Memory:\n{}\n\n\
         Total processes: {}",
        top_cpu.iter().map(|p| format!("  - {} (PID {}): {:.1}% CPU, {:.0} MB RAM",
            p.name, p.pid, p.cpu_percent, p.memory_mb)).collect::<Vec<_>>().join("\n"),
        top_mem.iter().map(|p| format!("  - {} (PID {}): {:.0} MB RAM, {:.1}% CPU",
            p.name, p.pid, p.memory_mb, p.cpu_percent)).collect::<Vec<_>>().join("\n"),
        app.all_processes.len()
    );

    app.ai_loading.insert(PROCESS_AI_KEY.to_string(), true);
    app.ai_errors.remove(PROCESS_AI_KEY);

    let (tx, rx) = mpsc::channel(1);
    app.ai_analysis_rx = Some(rx);

    let api_key = app.settings.openai_api_key.clone();
    let provider = app.settings.ai_provider;
    let phi_available = app.ai_phi_silica_status.as_ref().map(|s| s.available).unwrap_or(false);
    let runtime = app.runtime.clone();

    std::thread::spawn(move || {
        runtime.block_on(async {
            let result = analyze_processes(&process_text, api_key, provider, phi_available).await;
            let _ = tx.send(crate::AiAnalysisResult {
                task_id: PROCESS_AI_KEY.to_string(),
                interpretation: result,
            }).await;
        });
    });
}

async fn analyze_processes(
    process_text: &str,
    api_key: Option<String>,
    provider: crate::AiProvider,
    phi_available: bool,
) -> Result<String, String> {
    let use_phi = match provider {
        crate::AiProvider::Auto => phi_available,
        crate::AiProvider::PhiSilica => phi_available,
        crate::AiProvider::OpenAI => false,
    };

    let prompt = format!(
        "Analyze these Windows processes. Provide a brief assessment (3-4 sentences):\n\
         1. Identify any unusually high resource usage\n\
         2. Flag any potentially suspicious or unnecessary processes\n\
         3. Suggest optimizations if applicable\n\n\
         {}",
        process_text
    );

    if use_phi {
        wfdiag_tauri::phi_silica::analyze_with_phi_silica(prompt)
            .await
            .map(|r| r.analysis)
    } else if let Some(key) = api_key {
        if key.is_empty() {
            return Err("OpenAI API key not configured".to_string());
        }
        analyze_generic_with_openai(&key, &prompt).await
    } else {
        Err("No AI provider available".to_string())
    }
}

/// Render process list AI panel
pub fn render_process_ai_panel(ui: &mut egui::Ui, app: &mut WfDiagApp) {
    const PROCESS_AI_KEY: &str = "__process_analysis__";

    let ai_available = is_ai_available(app);
    let is_loading = app.ai_loading.get(PROCESS_AI_KEY).copied().unwrap_or(false);
    let cached = app.ai_interpretations.get(PROCESS_AI_KEY).cloned();
    let error = app.ai_errors.get(PROCESS_AI_KEY).cloned();
    let provider = get_active_provider_name(app).to_string();
    let has_processes = !app.all_processes.is_empty();

    let mut should_analyze = false;
    let mut should_retry = false;

    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(99, 102, 241, 20))
        .corner_radius(6.0)
        .inner_margin(Margin::same(10))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(99, 102, 241, 50)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("✨").size(14.0));
                ui.label(RichText::new("AI Process Analysis").size(11.0).strong().color(Color32::from_rgb(99, 102, 241)));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ai_available {
                        ui.label(RichText::new(&provider).size(9.0).weak());
                    }
                });
            });

            ui.add_space(6.0);

            if !app.settings.ai_enabled {
                ui.label(RichText::new("AI analysis is disabled. Enable in Settings.").size(11.0).weak());
            } else if !ai_available {
                ui.label(RichText::new("No AI provider available.").size(11.0).weak());
            } else if !has_processes {
                ui.label(RichText::new("Start monitoring to enable AI analysis.").size(11.0).weak());
            } else if is_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(format!("Analyzing processes with {}...", provider)).size(11.0).weak());
                });
            } else if let Some(ref interpretation) = cached {
                ui.label(RichText::new(interpretation).size(11.0));
                ui.add_space(4.0);
                if ui.small_button("Refresh Analysis").clicked() {
                    should_analyze = true;
                }
            } else if let Some(ref err) = error {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").size(11.0).color(Color32::from_rgb(220, 100, 100)));
                    ui.label(RichText::new(err).size(11.0).color(Color32::from_rgb(220, 100, 100)));
                });
                if ui.small_button("Retry").clicked() {
                    should_retry = true;
                }
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Get AI insights on running processes.").size(11.0).weak());
                    if ui.small_button("Analyze Processes").clicked() {
                        should_analyze = true;
                    }
                });
            }
        });

    if should_analyze || should_retry {
        if should_retry {
            app.ai_errors.remove(PROCESS_AI_KEY);
        }
        app.ai_interpretations.remove(PROCESS_AI_KEY);
        request_process_analysis(app);
    }
}

// ============================================================================
// Scan History AI Analysis
// ============================================================================

/// Request AI analysis of scan comparison
pub fn request_comparison_analysis(app: &mut WfDiagApp) {
    const COMPARISON_AI_KEY: &str = "__comparison_analysis__";

    if app.ai_loading.get(COMPARISON_AI_KEY).copied().unwrap_or(false) {
        return;
    }

    let comparison = match &app.comparison_result {
        Some(c) => c.clone(),
        None => return,
    };

    let comparison_text = format!(
        "Scan Comparison Analysis:\n\n\
         Previous scan: {} ({} passed, {} failed)\n\
         Current scan: {} ({} passed, {} failed)\n\n\
         Total changes: {}\n\
         New failures: {}\n\
         New successes: {}\n\n\
         New Failures:\n{}\n\n\
         New Successes:\n{}",
        comparison.previous_scan.timestamp.format("%Y-%m-%d %H:%M"),
        comparison.previous_scan.success_count,
        comparison.previous_scan.failure_count,
        comparison.current_scan.timestamp.format("%Y-%m-%d %H:%M"),
        comparison.current_scan.success_count,
        comparison.current_scan.failure_count,
        comparison.total_changes,
        comparison.new_failures.len(),
        comparison.new_successes.len(),
        comparison.new_failures.iter().map(|c| format!("  - {}: {}", c.task_name, c.category)).collect::<Vec<_>>().join("\n"),
        comparison.new_successes.iter().map(|c| format!("  - {}: {}", c.task_name, c.category)).collect::<Vec<_>>().join("\n"),
    );

    app.ai_loading.insert(COMPARISON_AI_KEY.to_string(), true);
    app.ai_errors.remove(COMPARISON_AI_KEY);

    let (tx, rx) = mpsc::channel(1);
    app.ai_analysis_rx = Some(rx);

    let api_key = app.settings.openai_api_key.clone();
    let provider = app.settings.ai_provider;
    let phi_available = app.ai_phi_silica_status.as_ref().map(|s| s.available).unwrap_or(false);
    let runtime = app.runtime.clone();

    std::thread::spawn(move || {
        runtime.block_on(async {
            let result = analyze_comparison(&comparison_text, api_key, provider, phi_available).await;
            let _ = tx.send(crate::AiAnalysisResult {
                task_id: COMPARISON_AI_KEY.to_string(),
                interpretation: result,
            }).await;
        });
    });
}

async fn analyze_comparison(
    comparison_text: &str,
    api_key: Option<String>,
    provider: crate::AiProvider,
    phi_available: bool,
) -> Result<String, String> {
    let use_phi = match provider {
        crate::AiProvider::Auto => phi_available,
        crate::AiProvider::PhiSilica => phi_available,
        crate::AiProvider::OpenAI => false,
    };

    let prompt = format!(
        "Analyze this Windows diagnostic scan comparison. Provide a brief summary (3-4 sentences):\n\
         1. What changed between the scans\n\
         2. Whether the system health improved or degraded\n\
         3. Priority actions if new failures were detected\n\n\
         {}",
        comparison_text
    );

    if use_phi {
        wfdiag_tauri::phi_silica::analyze_with_phi_silica(prompt)
            .await
            .map(|r| r.analysis)
    } else if let Some(key) = api_key {
        if key.is_empty() {
            return Err("OpenAI API key not configured".to_string());
        }
        analyze_generic_with_openai(&key, &prompt).await
    } else {
        Err("No AI provider available".to_string())
    }
}

/// Render comparison AI panel
pub fn render_comparison_ai_panel(ui: &mut egui::Ui, app: &mut WfDiagApp) {
    const COMPARISON_AI_KEY: &str = "__comparison_analysis__";

    let ai_available = is_ai_available(app);
    let is_loading = app.ai_loading.get(COMPARISON_AI_KEY).copied().unwrap_or(false);
    let cached = app.ai_interpretations.get(COMPARISON_AI_KEY).cloned();
    let error = app.ai_errors.get(COMPARISON_AI_KEY).cloned();
    let provider = get_active_provider_name(app).to_string();
    let has_comparison = app.comparison_result.is_some();

    let mut should_analyze = false;
    let mut should_retry = false;

    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(99, 102, 241, 20))
        .corner_radius(6.0)
        .inner_margin(Margin::same(10))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(99, 102, 241, 50)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("✨").size(14.0));
                ui.label(RichText::new("AI Comparison Analysis").size(11.0).strong().color(Color32::from_rgb(99, 102, 241)));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ai_available {
                        ui.label(RichText::new(&provider).size(9.0).weak());
                    }
                });
            });

            ui.add_space(6.0);

            if !app.settings.ai_enabled {
                ui.label(RichText::new("AI analysis is disabled. Enable in Settings.").size(11.0).weak());
            } else if !ai_available {
                ui.label(RichText::new("No AI provider available.").size(11.0).weak());
            } else if !has_comparison {
                ui.label(RichText::new("Select scans to compare for AI analysis.").size(11.0).weak());
            } else if is_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(format!("Analyzing comparison with {}...", provider)).size(11.0).weak());
                });
            } else if let Some(ref interpretation) = cached {
                ui.label(RichText::new(interpretation).size(11.0));
                ui.add_space(4.0);
                if ui.small_button("Refresh Analysis").clicked() {
                    should_analyze = true;
                }
            } else if let Some(ref err) = error {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").size(11.0).color(Color32::from_rgb(220, 100, 100)));
                    ui.label(RichText::new(err).size(11.0).color(Color32::from_rgb(220, 100, 100)));
                });
                if ui.small_button("Retry").clicked() {
                    should_retry = true;
                }
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Get AI summary of scan differences.").size(11.0).weak());
                    if ui.small_button("Analyze Changes").clicked() {
                        should_analyze = true;
                    }
                });
            }
        });

    if should_analyze || should_retry {
        if should_retry {
            app.ai_errors.remove(COMPARISON_AI_KEY);
        }
        app.ai_interpretations.remove(COMPARISON_AI_KEY);
        request_comparison_analysis(app);
    }
}

/// Request AI analysis of a single scan
pub fn request_scan_analysis(app: &mut WfDiagApp) {
    const SCAN_AI_KEY: &str = "__scan_analysis__";

    if app.ai_loading.get(SCAN_AI_KEY).copied().unwrap_or(false) {
        return;
    }

    let scan = match &app.selected_scan {
        Some(s) => s.clone(),
        None => return,
    };

    // Summarize scan results - get all tasks with their categories
    let tasks = wfdiag_tauri::diagnostics::get_all_tasks();

    let mut passed: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for (id, result) in scan.results.iter() {
        let task_info = tasks.iter().find(|t| &t.id == id);
        let name = task_info.map(|t| t.name.clone()).unwrap_or_else(|| id.clone());
        let category = task_info.map(|t| t.category.clone()).unwrap_or_else(|| "Other".to_string());

        if result.success {
            passed.push(format!("{} ({})", name, category));
        } else {
            failed.push(format!("{} ({})", name, category));
        }
    }

    // Group by category for summary
    let mut categories: std::collections::HashMap<String, (usize, usize)> = std::collections::HashMap::new();
    for (id, result) in scan.results.iter() {
        let category = tasks.iter().find(|t| &t.id == id)
            .map(|t| t.category.clone())
            .unwrap_or_else(|| "Other".to_string());
        let entry = categories.entry(category).or_insert((0, 0));
        if result.success {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
    }

    let category_summary: String = categories.iter()
        .map(|(cat, (p, f))| format!("  - {}: {} passed, {} failed", cat, p, f))
        .collect::<Vec<_>>()
        .join("\n");

    let scan_text = format!(
        "Windows Diagnostic Scan Summary:\n\
         Date: {}\n\
         Computer: {}\n\
         OS: {}\n\
         Tasks: {} total ({} passed, {} failed)\n\
         Duration: {}ms\n\n\
         Results by Category:\n{}\n\n\
         Failed checks ({}):\n{}\n\n\
         Sample of passed checks (showing up to 10):\n{}",
        scan.timestamp.format("%Y-%m-%d %H:%M:%S"),
        scan.computer_name,
        scan.os_version,
        scan.task_count,
        scan.success_count,
        scan.failure_count,
        scan.duration_ms,
        category_summary,
        failed.len(),
        if failed.is_empty() { "  None - all checks passed!".to_string() } else { failed.iter().map(|f| format!("  - {}", f)).collect::<Vec<_>>().join("\n") },
        passed.iter().take(10).map(|p| format!("  - {}", p)).collect::<Vec<_>>().join("\n")
    );

    app.ai_loading.insert(SCAN_AI_KEY.to_string(), true);
    app.ai_errors.remove(SCAN_AI_KEY);

    let (tx, rx) = mpsc::channel(1);
    app.ai_analysis_rx = Some(rx);

    let api_key = app.settings.openai_api_key.clone();
    let provider = app.settings.ai_provider;
    let phi_available = app.ai_phi_silica_status.as_ref().map(|s| s.available).unwrap_or(false);
    let runtime = app.runtime.clone();

    std::thread::spawn(move || {
        runtime.block_on(async {
            let result = analyze_scan(&scan_text, api_key, provider, phi_available).await;
            let _ = tx.send(crate::AiAnalysisResult {
                task_id: SCAN_AI_KEY.to_string(),
                interpretation: result,
            }).await;
        });
    });
}

async fn analyze_scan(
    scan_text: &str,
    api_key: Option<String>,
    provider: crate::AiProvider,
    phi_available: bool,
) -> Result<String, String> {
    let use_phi = match provider {
        crate::AiProvider::Auto => phi_available,
        crate::AiProvider::PhiSilica => phi_available,
        crate::AiProvider::OpenAI => false,
    };

    let prompt = format!(
        "Analyze this Windows diagnostic scan. Provide a brief summary (3-4 sentences):\n\
         1. Overall system health assessment\n\
         2. Key issues found (if any)\n\
         3. Recommended actions\n\n\
         {}",
        scan_text
    );

    if use_phi {
        wfdiag_tauri::phi_silica::analyze_with_phi_silica(prompt)
            .await
            .map(|r| r.analysis)
    } else if let Some(key) = api_key {
        if key.is_empty() {
            return Err("OpenAI API key not configured".to_string());
        }
        analyze_generic_with_openai(&key, &prompt).await
    } else {
        Err("No AI provider available".to_string())
    }
}

/// Render scan detail AI panel
pub fn render_scan_ai_panel(ui: &mut egui::Ui, app: &mut WfDiagApp) {
    const SCAN_AI_KEY: &str = "__scan_analysis__";

    let ai_available = is_ai_available(app);
    let is_loading = app.ai_loading.get(SCAN_AI_KEY).copied().unwrap_or(false);
    let cached = app.ai_interpretations.get(SCAN_AI_KEY).cloned();
    let error = app.ai_errors.get(SCAN_AI_KEY).cloned();
    let provider = get_active_provider_name(app).to_string();
    let has_scan = app.selected_scan.is_some();

    let mut should_analyze = false;
    let mut should_retry = false;

    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(99, 102, 241, 20))
        .corner_radius(6.0)
        .inner_margin(Margin::same(10))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(99, 102, 241, 50)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("✨").size(14.0));
                ui.label(RichText::new("AI Scan Summary").size(11.0).strong().color(Color32::from_rgb(99, 102, 241)));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ai_available {
                        ui.label(RichText::new(&provider).size(9.0).weak());
                    }
                });
            });

            ui.add_space(6.0);

            if !app.settings.ai_enabled {
                ui.label(RichText::new("AI analysis is disabled. Enable in Settings.").size(11.0).weak());
            } else if !ai_available {
                ui.label(RichText::new("No AI provider available.").size(11.0).weak());
            } else if !has_scan {
                ui.label(RichText::new("Select a scan to view AI summary.").size(11.0).weak());
            } else if is_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(format!("Analyzing scan with {}...", provider)).size(11.0).weak());
                });
            } else if let Some(ref interpretation) = cached {
                ui.label(RichText::new(interpretation).size(11.0));
                ui.add_space(4.0);
                if ui.small_button("Refresh Analysis").clicked() {
                    should_analyze = true;
                }
            } else if let Some(ref err) = error {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").size(11.0).color(Color32::from_rgb(220, 100, 100)));
                    ui.label(RichText::new(err).size(11.0).color(Color32::from_rgb(220, 100, 100)));
                });
                if ui.small_button("Retry").clicked() {
                    should_retry = true;
                }
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Get AI summary of this scan.").size(11.0).weak());
                    if ui.small_button("Summarize Scan").clicked() {
                        should_analyze = true;
                    }
                });
            }
        });

    if should_analyze || should_retry {
        if should_retry {
            app.ai_errors.remove(SCAN_AI_KEY);
        }
        app.ai_interpretations.remove(SCAN_AI_KEY);
        request_scan_analysis(app);
    }
}
