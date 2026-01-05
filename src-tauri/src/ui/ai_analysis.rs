//! AI Analysis Tab - Conversational AI interface for system diagnostics
//!
//! Provides a chat-like interface for interacting with AI providers (Phi Silica/OpenAI)
//! to analyze system diagnostics, ask questions, and get recommendations.

use crate::{AiChatMessage, AiChatRole, AiProvider, WfDiagApp};
use eframe::egui::{self, Color32, Margin, RichText, ScrollArea, TextEdit, Vec2};
use tokio::sync::mpsc;

use super::ai::{get_active_provider_name, is_ai_available, is_phi_available, is_openai_configured, should_use_phi, start_phi_silica_check};

/// Show the AI Analysis tab
pub fn show(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    // Check AI status on first view
    if app.ai_phi_silica_status.is_none() && !app.ai_status_checking {
        start_phi_silica_check(app);
    }

    // Process any pending chat responses
    process_chat_updates(app);

    // Add margins to prevent edge bleeding
    egui::Frame::new()
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
        // Header
        render_header(app, ui);

        ui.add_space(8.0);

        // Main content area
        let available_height = ui.available_height();
        let input_height = 100.0;
        let chat_height = (available_height - input_height - 100.0).max(200.0);

        // Chat messages area
        let mut quick_action_prompt: Option<String> = None;
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .corner_radius(8.0)
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .max_height(chat_height)
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        quick_action_prompt = render_chat_messages(app, ui);
                    });
            });

        // Apply quick action if clicked
        if let Some(prompt) = quick_action_prompt {
            app.ai_chat_input = prompt;
        }

        ui.add_space(8.0);

        // Input area
        render_input_area(app, ui);
    });
}

/// Render the header with AI status and provider info
fn render_header(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading(RichText::new("✨ AI Analysis").strong());

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Provider status badge
            let ai_available = is_ai_available(app);
            let provider = get_active_provider_name(app);

            let (status_color, status_text) = if !app.settings.ai_enabled {
                (Color32::GRAY, "AI Disabled")
            } else if app.ai_status_checking {
                (Color32::from_rgb(200, 150, 50), "Checking...")
            } else if ai_available {
                (Color32::from_rgb(99, 102, 241), provider)
            } else {
                (Color32::from_rgb(200, 100, 100), "Unavailable")
            };

            egui::Frame::new()
                .fill(status_color.linear_multiply(0.2))
                .corner_radius(4.0)
                .inner_margin(Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.label(RichText::new(status_text).size(11.0).color(status_color));
                });

            // Provider selector if both are available
            if is_phi_available(app) && is_openai_configured(app) {
                ui.add_space(8.0);
                egui::ComboBox::from_id_salt("ai_provider_select")
                    .selected_text(match app.settings.ai_provider {
                        AiProvider::Auto => "Auto",
                        AiProvider::PhiSilica => "Phi Silica",
                        AiProvider::OpenAI => "OpenAI",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut app.settings.ai_provider,
                            AiProvider::Auto,
                            "Auto",
                        );
                        ui.selectable_value(
                            &mut app.settings.ai_provider,
                            AiProvider::PhiSilica,
                            "Phi Silica (Local)",
                        );
                        ui.selectable_value(
                            &mut app.settings.ai_provider,
                            AiProvider::OpenAI,
                            "OpenAI (Cloud)",
                        );
                    });
            }

            // Clear chat button
            if !app.ai_chat_messages.is_empty() {
                ui.add_space(8.0);
                if ui.small_button("Clear Chat").clicked() {
                    app.ai_chat_messages.clear();
                    app.ai_chat_error = None;
                }
            }
        });
    });

    ui.separator();

    // Show provider info/warnings
    if !app.settings.ai_enabled {
        ui.horizontal(|ui| {
            ui.label(RichText::new("⚠").color(Color32::from_rgb(200, 150, 50)));
            ui.label(RichText::new("AI analysis is disabled. Enable it in Settings.").weak());
            if ui.small_button("Open Settings").clicked() {
                app.show_settings = true;
            }
        });
    } else if !is_ai_available(app) && !app.ai_status_checking {
        ui.horizontal(|ui| {
            ui.label(RichText::new("ℹ").color(Color32::from_rgb(100, 150, 200)));
            ui.label(
                RichText::new(
                    "Configure OpenAI API key in Settings, or use a Copilot+ PC for local AI.",
                )
                .weak(),
            );
            if ui.small_button("Open Settings").clicked() {
                app.show_settings = true;
            }
        });
    }
}

/// Render chat message history
/// Returns an optional prompt if a quick action was clicked
fn render_chat_messages(app: &WfDiagApp, ui: &mut egui::Ui) -> Option<String> {
    let is_dark = ui.visuals().dark_mode;

    if app.ai_chat_messages.is_empty() {
        // Empty state with helpful prompts
        return render_empty_state(ui);
    }

    for message in &app.ai_chat_messages {
        let (bg_color, text_color, align, label) = match message.role {
            AiChatRole::User => (
                if is_dark {
                    Color32::from_rgb(45, 55, 72)
                } else {
                    Color32::from_rgb(226, 232, 240)
                },
                if is_dark {
                    Color32::from_gray(220)
                } else {
                    Color32::from_gray(30)
                },
                egui::Align::RIGHT,
                "You",
            ),
            AiChatRole::Assistant => (
                if is_dark {
                    Color32::from_rgb(49, 46, 129)
                } else {
                    Color32::from_rgb(224, 231, 255)
                },
                if is_dark {
                    Color32::from_gray(220)
                } else {
                    Color32::from_gray(30)
                },
                egui::Align::LEFT,
                "AI",
            ),
            AiChatRole::System => (
                if is_dark {
                    Color32::from_rgb(40, 60, 40)  // Dark green tint
                } else {
                    Color32::from_rgb(230, 245, 230)  // Light green tint
                },
                if is_dark { Color32::from_rgb(150, 200, 150) } else { Color32::from_rgb(50, 100, 50) },
                egui::Align::Center,
                "Context",
            ),
        };

        // System messages are full-width, others are aligned
        if message.role == AiChatRole::System {
            // Full-width system message (shows context being used)
            egui::Frame::new()
                .fill(bg_color)
                .corner_radius(6.0)
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).size(9.0).strong().color(text_color));
                        ui.label(
                            RichText::new(message.timestamp.format("%H:%M").to_string())
                                .size(8.0)
                                .weak(),
                        );
                    });
                    ui.add(egui::Label::new(
                        RichText::new(&message.content).size(10.0).color(text_color)
                    ).wrap());
                });
        } else {
            // User messages on right, AI on left
            let is_user = message.role == AiChatRole::User;
            let available = ui.available_width();
            let max_width = (available * 0.75).min(500.0);

            ui.horizontal(|ui| {
                if is_user {
                    // Add spacer to push to right
                    ui.add_space(available - max_width - 20.0);
                }

                egui::Frame::new()
                    .fill(bg_color)
                    .corner_radius(12.0)
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(max_width);

                        // Role label
                        ui.horizontal(|ui| {
                            let label_color: Color32 = if message.role == AiChatRole::Assistant {
                                Color32::from_rgb(99, 102, 241)
                            } else {
                                text_color.linear_multiply(0.7)
                            };
                            ui.label(RichText::new(label).size(10.0).strong().color(label_color));
                            ui.label(
                                RichText::new(message.timestamp.format("%H:%M").to_string())
                                    .size(9.0)
                                    .weak(),
                            );
                        });

                        ui.add_space(4.0);

                        // Message content with word wrap
                        ui.add(egui::Label::new(
                            RichText::new(&message.content).size(12.0).color(text_color)
                        ).wrap());
                    });
            });
        }

        ui.add_space(8.0);
    }

    // Show loading indicator
    if app.ai_chat_loading {
        ui.horizontal(|ui| {
            egui::Frame::new()
                .fill(if is_dark {
                    Color32::from_rgb(49, 46, 129)
                } else {
                    Color32::from_rgb(224, 231, 255)
                })
                .corner_radius(12.0)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Thinking...").size(11.0).weak());
                    });
                });
        });
    }

    // Show error if any
    if let Some(ref error) = app.ai_chat_error {
        ui.horizontal(|ui| {
            egui::Frame::new()
                .fill(Color32::from_rgb(100, 40, 40))
                .corner_radius(8.0)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!("⚠ {}", error))
                            .size(11.0)
                            .color(Color32::from_rgb(255, 180, 180)),
                    );
                });
        });
    }

    None
}

/// Render empty state with quick action prompts
/// Returns the prompt to set if a quick action was clicked
fn render_empty_state(ui: &mut egui::Ui) -> Option<String> {
    let is_dark = ui.visuals().dark_mode;
    let mut selected_prompt: Option<String> = None;

    ui.vertical_centered(|ui| {
        ui.add_space(40.0);

        ui.label(RichText::new("✨").size(48.0));
        ui.add_space(16.0);

        ui.label(RichText::new("AI System Assistant").size(20.0).strong());
        ui.add_space(8.0);

        ui.label(RichText::new("Ask questions about your system, get help interpreting diagnostic results,\nor request recommendations for improving performance.")
            .size(12.0)
            .weak());

        ui.add_space(24.0);

        // Quick action buttons
        ui.label(RichText::new("Quick Actions").size(11.0).strong());
        ui.add_space(8.0);

        let quick_actions = [
            ("📊 Summarize Last Scan", "Summarize the results of my last diagnostic scan and highlight any issues that need attention."),
            ("🔧 System Health Check", "Analyze my current system health based on available diagnostic data and provide recommendations."),
            ("💡 Performance Tips", "What are the most impactful things I can do to improve my Windows system's performance?"),
            ("🛡️ Security Review", "Review my system's security posture and suggest any improvements."),
        ];

        let btn_bg = if is_dark { Color32::from_rgb(45, 55, 72) } else { Color32::from_rgb(240, 240, 245) };

        for (label, prompt) in quick_actions {
            if ui.add_sized(
                Vec2::new(400.0, 32.0),
                egui::Button::new(RichText::new(label).size(11.0))
                    .fill(btn_bg)
                    .corner_radius(6.0)
            ).clicked() {
                selected_prompt = Some(prompt.to_string());
            }
        }
    });

    selected_prompt
}

/// Render the input area with text box and send button
fn render_input_area(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    let ai_available = is_ai_available(app);
    let is_loading = app.ai_chat_loading;

    egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.weak_bg_fill)
        .corner_radius(8.0)
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Text input
                let response = ui.add_sized(
                    Vec2::new(ui.available_width() - 80.0, 60.0),
                    TextEdit::multiline(&mut app.ai_chat_input)
                        .hint_text("Ask a question about your system...")
                        .desired_rows(2),
                );

                // Handle Enter key (Shift+Enter for newline)
                let enter_pressed = response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);

                ui.vertical(|ui| {
                    // Send button
                    let can_send =
                        ai_available && !is_loading && !app.ai_chat_input.trim().is_empty();

                    let send_btn = ui.add_enabled(
                        can_send,
                        egui::Button::new(RichText::new("Send").size(12.0))
                            .fill(Color32::from_rgb(99, 102, 241))
                            .min_size(Vec2::new(60.0, 28.0)),
                    );

                    if (send_btn.clicked() || enter_pressed) && can_send {
                        send_chat_message(app);
                    }

                    // Loading indicator or status
                    if is_loading {
                        ui.spinner();
                    }
                });
            });

            // Hint text
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Press Enter to send, Shift+Enter for new line")
                        .size(9.0)
                        .weak(),
                );
            });
        });
}

/// Determine which diagnostics to run based on the user's question
fn get_relevant_diagnostics(question: &str) -> Vec<&'static str> {
    let q = question.to_lowercase();
    let mut tasks = Vec::new();

    // Match question to relevant diagnostics
    if q.contains("disk") || q.contains("storage") || q.contains("space") || q.contains("drive") {
        tasks.extend(["logical_disk", "disk_drive"]);
    }
    if q.contains("memory") || q.contains("ram") {
        tasks.extend(["physical_memory"]);
    }
    if q.contains("cpu") || q.contains("processor") || q.contains("performance") {
        tasks.extend(["processor"]);
    }
    if q.contains("network") || q.contains("internet") || q.contains("wifi") || q.contains("ethernet") {
        tasks.extend(["network_adapter", "ipconfig"]);
    }
    if q.contains("system") || q.contains("health") || q.contains("status") || q.contains("overview") {
        tasks.extend(["comp_system", "os_info", "logical_disk", "physical_memory", "processor"]);
    }
    if q.contains("driver") {
        tasks.extend(["system_driver"]);
    }
    if q.contains("service") {
        tasks.extend(["services"]);
    }

    // If no specific match, run basic system diagnostics
    if tasks.is_empty() {
        tasks.extend(["comp_system", "os_info", "logical_disk"]);
    }

    // Remove duplicates
    tasks.sort();
    tasks.dedup();
    tasks.truncate(5); // Limit to 5 tasks max
    tasks
}

/// Send the current chat input
fn send_chat_message(app: &mut WfDiagApp) {
    let input = app.ai_chat_input.trim().to_string();
    if input.is_empty() {
        return;
    }

    // Collect conversation history BEFORE adding the new user message
    // This gives the AI context of the previous conversation
    let conversation_history: Vec<(AiChatRole, String)> = app
        .ai_chat_messages
        .iter()
        .map(|m| (m.role, m.content.clone()))
        .collect();

    // Add user message
    app.ai_chat_messages.push(AiChatMessage {
        role: AiChatRole::User,
        content: input.clone(),
        timestamp: chrono::Local::now(),
    });

    // Determine provider and whether to use keyword-based diagnostics
    let api_key = app.settings.openai_api_key.clone();
    let provider = app.settings.ai_provider;
    let phi_available = is_phi_available(app);

    // Determine if we'll use Phi Silica (which needs keyword-based diagnostics)
    // or OpenAI (which uses AI-driven function calling)
    let use_phi = should_use_phi(provider, phi_available);

    // For Phi Silica, run keyword-based diagnostics first
    // For OpenAI, the AI will decide via function calling
    let tasks_to_run = if use_phi {
        get_relevant_diagnostics(&input)
    } else {
        Vec::new() // OpenAI will use function calling to select diagnostics
    };

    // Show what we're doing
    if use_phi && !tasks_to_run.is_empty() {
        app.ai_chat_messages.push(AiChatMessage {
            role: AiChatRole::System,
            content: format!("🔍 Running diagnostics: {}", tasks_to_run.join(", ")),
            timestamp: chrono::Local::now(),
        });
    } else if !use_phi && api_key.is_some() {
        app.ai_chat_messages.push(AiChatMessage {
            role: AiChatRole::System,
            content: "🤖 AI will select and run relevant diagnostics...".to_string(),
            timestamp: chrono::Local::now(),
        });
    }

    // Clear input
    app.ai_chat_input.clear();
    app.ai_chat_loading = true;
    app.ai_chat_error = None;

    // Start async request
    let (tx, rx) = mpsc::channel(1);

    let runtime = app.runtime.clone();

    // Store the receiver for later polling
    std::thread::spawn({
        let input = input.clone();
        let tasks = tasks_to_run.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let history = conversation_history;
        move || {
            runtime.block_on(async {
                // For Phi Silica, run the diagnostics first (keyword-based)
                // For OpenAI, context stays empty - AI will use function calling
                let mut context = String::new();
                for task_id in &tasks {
                    if let Ok(result) = wfdiag_tauri::diagnostics::run_diagnostic_task_sync(task_id) {
                        let summary = extract_key_data_static(&result.output, task_id);
                        if !summary.is_empty() {
                            context.push_str(&format!("{}: {}\n", task_id, summary.trim()));
                        }
                    }
                }

                // Now analyze with AI - pass conversation history for context
                let result = analyze_chat_message(&input, &context, &history, api_key, provider, phi_available).await;
                let _ = tx.send(result).await;
            });
        }
    });

    // Store the receiver in app state for later polling
    app.ai_chat_rx = Some(rx);
}

/// Process pending chat response
fn process_chat_updates(app: &mut WfDiagApp) {
    if let Some(ref mut rx) = app.ai_chat_rx {
        match rx.try_recv() {
            Ok(result) => {
                app.ai_chat_loading = false;
                match result {
                    Ok(response) => {
                        app.ai_chat_messages.push(AiChatMessage {
                            role: AiChatRole::Assistant,
                            content: response,
                            timestamp: chrono::Local::now(),
                        });
                    }
                    Err(error) => {
                        app.ai_chat_error = Some(error);
                    }
                }
                app.ai_chat_rx = None;
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {
                app.ai_chat_loading = false;
                app.ai_chat_rx = None;
            }
        }
    }
}

/// Build context from diagnostic results with actual data
/// Phi Silica has only 4k token context, so keep this VERY concise
fn build_diagnostic_context(app: &WfDiagApp) -> String {
    let results = app.results.lock().unwrap();

    if results.is_empty() {
        return String::new();
    }

    let mut context = String::new();
    let mut found_any = false;

    // Try to find key diagnostics
    let key_diagnostics = [
        ("logical_disk", "Disk"),
        ("physical_memory", "RAM"),
        ("processor", "CPU"),
        ("comp_system", "System"),
        ("os_info", "OS"),
        ("disk_drive", "Drives"),
    ];

    for (task_id, display_name) in key_diagnostics {
        if let Some(result) = results.get(task_id) {
            if !result.output.is_empty() {
                let summary = extract_key_data(&result.output, task_id);
                if !summary.is_empty() {
                    context.push_str(&format!("{}: {}\n", display_name, summary.trim()));
                    found_any = true;
                }
            }
        }
    }

    // If we didn't find the specific keys, just summarize what we have
    if !found_any {
        let task_names: Vec<_> = results.keys()
            .take(5)
            .map(|s| s.as_str())
            .collect();
        context.push_str(&format!("Scanned: {}\n", task_names.join(", ")));
    }

    // Summary
    let total = results.len();
    let passed = results.values().filter(|r| r.success).count();
    let failed = total - passed;

    context.push_str(&format!("Status: {}/{} passed", passed, total));

    // Only list failed if any
    if failed > 0 {
        let failed_names: Vec<_> = results.iter()
            .filter(|(_, r)| !r.success)
            .take(3)
            .map(|(id, _)| {
                app.available_tasks.iter()
                    .find(|t| &t.id == id)
                    .map(|t| t.name.as_str())
                    .unwrap_or(id.as_str())
            })
            .collect();
        context.push_str(&format!(", Failed: {}", failed_names.join(", ")));
    }

    context
}

/// Extract key data from diagnostic output JSON (static version for async)
fn extract_key_data_static(output: &str, task_id: &str) -> String {
    extract_key_data(output, task_id)
}

/// Extract key data from diagnostic output JSON
fn extract_key_data(output: &str, task_id: &str) -> String {
    let trimmed = output.trim();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        // Not JSON, return truncated text
        return if output.len() > 200 {
            format!("{}\n", &output[..200])
        } else {
            format!("{}\n", output)
        };
    }

    let json: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    match task_id {
        "logical_disk" => extract_disk_info(&json),
        "physical_memory" => extract_memory_info(&json),
        "processor" => extract_cpu_info(&json),
        "network_adapter" => extract_network_info(&json),
        "npu_info" => extract_npu_info(&json),
        "comp_system" => extract_comp_system_info(&json),
        "os_info" => extract_os_info(&json),
        _ => extract_generic_info(&json),
    }
}

fn extract_comp_system_info(json: &serde_json::Value) -> String {
    let obj = if let Some(arr) = json.as_array() {
        arr.first().and_then(|v| v.as_object())
    } else {
        json.as_object()
    };

    if let Some(sys) = obj {
        let mut parts = Vec::new();
        if let Some(mfr) = sys.get("Manufacturer").and_then(|v| v.as_str()) {
            parts.push(mfr.to_string());
        }
        if let Some(model) = sys.get("Model").and_then(|v| v.as_str()) {
            parts.push(model.to_string());
        }
        if let Some(name) = sys.get("Name").and_then(|v| v.as_str()) {
            parts.push(format!("({})", name));
        }
        if let Some(mem) = sys.get("TotalPhysicalMemory").and_then(|v| v.as_u64()) {
            parts.push(format!("{:.0}GB RAM", mem as f64 / 1_073_741_824.0));
        }
        return parts.join(" ");
    }
    String::new()
}

fn extract_os_info(json: &serde_json::Value) -> String {
    let obj = if let Some(arr) = json.as_array() {
        arr.first().and_then(|v| v.as_object())
    } else {
        json.as_object()
    };

    if let Some(os) = obj {
        let mut parts = Vec::new();
        if let Some(caption) = os.get("Caption").and_then(|v| v.as_str()) {
            parts.push(caption.to_string());
        }
        if let Some(version) = os.get("Version").and_then(|v| v.as_str()) {
            parts.push(format!("({})", version));
        }
        if let Some(build) = os.get("BuildNumber").and_then(|v| v.as_str()) {
            parts.push(format!("Build {}", build));
        }
        if let Some(arch) = os.get("OSArchitecture").and_then(|v| v.as_str()) {
            parts.push(arch.to_string());
        }
        return parts.join(" ");
    }
    String::new()
}

fn extract_disk_info(json: &serde_json::Value) -> String {
    if let Some(arr) = json.as_array() {
        let mut parts = Vec::new();
        for disk in arr.iter().take(2) { // Only first 2 drives
            if let Some(obj) = disk.as_object() {
                let name = obj.get("DeviceID").and_then(|v| v.as_str()).unwrap_or("?");
                let size = obj.get("Size").and_then(|v| v.as_u64()).unwrap_or(0);
                let free = obj.get("FreeSpace").and_then(|v| v.as_u64()).unwrap_or(0);
                if size > 0 {
                    let used_pct = ((size - free) as f64 / size as f64 * 100.0) as u64;
                    parts.push(format!("{} {}% used ({:.0}GB free)",
                        name, used_pct, free as f64 / 1_073_741_824.0));
                }
            }
        }
        parts.join(", ")
    } else {
        String::new()
    }
}

fn extract_memory_info(json: &serde_json::Value) -> String {
    if let Some(obj) = json.as_object() {
        let total = obj.get("TotalPhysicalMemory").and_then(|v| v.as_u64()).unwrap_or(0);
        let available = obj.get("FreePhysicalMemory").and_then(|v| v.as_u64());

        if total > 0 {
            if let Some(avail) = available {
                let avail_bytes = avail * 1024; // KB to bytes
                let used_pct = ((total - avail_bytes) as f64 / total as f64 * 100.0) as u64;
                return format!("{:.0}GB total, {}% used", total as f64 / 1_073_741_824.0, used_pct);
            }
            return format!("{:.0}GB total", total as f64 / 1_073_741_824.0);
        }
    } else if let Some(arr) = json.as_array() {
        let total: u64 = arr.iter()
            .filter_map(|m| m.get("Capacity").and_then(|v| v.as_u64()))
            .sum();
        if total > 0 {
            return format!("{:.0}GB ({} modules)", total as f64 / 1_073_741_824.0, arr.len());
        }
    }
    String::new()
}

fn extract_cpu_info(json: &serde_json::Value) -> String {
    let obj = if let Some(arr) = json.as_array() {
        arr.first().and_then(|v| v.as_object())
    } else {
        json.as_object()
    };

    if let Some(cpu) = obj {
        let cores = cpu.get("NumberOfCores").and_then(|v| v.as_u64()).unwrap_or(0);
        let logical = cpu.get("NumberOfLogicalProcessors").and_then(|v| v.as_u64()).unwrap_or(0);
        if cores > 0 {
            return format!("{} cores/{} threads", cores, logical);
        }
    }
    String::new()
}

fn extract_network_info(json: &serde_json::Value) -> String {
    let mut result = String::new();
    if let Some(arr) = json.as_array() {
        let active: Vec<_> = arr.iter()
            .filter(|a| a.get("NetConnectionStatus").and_then(|v| v.as_u64()) == Some(2))
            .collect();
        result.push_str(&format!("  {} adapter(s), {} connected\n", arr.len(), active.len()));
        for adapter in active.iter().take(2) {
            if let Some(name) = adapter.get("Name").and_then(|v| v.as_str()) {
                let short_name = if name.len() > 40 { &name[..40] } else { name };
                result.push_str(&format!("  - {}\n", short_name));
            }
        }
    }
    result
}

fn extract_npu_info(json: &serde_json::Value) -> String {
    let mut result = String::new();
    if let Some(obj) = json.as_object() {
        let available = obj.get("npu_available").and_then(|v| v.as_bool()).unwrap_or(false);
        if available {
            if let Some(name) = obj.get("npu_name").and_then(|v| v.as_str()) {
                result.push_str(&format!("  NPU: {}\n", name));
            }
            if let Some(tops) = obj.get("tops") {
                result.push_str(&format!("  Performance: {} TOPS\n", tops));
            }
        } else {
            result.push_str("  No NPU detected\n");
        }
    }
    result
}

fn extract_generic_info(json: &serde_json::Value) -> String {
    // For other diagnostics, just show count or brief summary
    if let Some(arr) = json.as_array() {
        format!("  {} items\n", arr.len())
    } else if let Some(obj) = json.as_object() {
        let status = obj.get("Status").or(obj.get("State")).and_then(|v| v.as_str());
        if let Some(s) = status {
            format!("  Status: {}\n", s)
        } else {
            String::new()
        }
    } else {
        String::new()
    }
}

/// Analyze chat message with AI
/// Includes conversation history for context
async fn analyze_chat_message(
    message: &str,
    context: &str,
    conversation_history: &[(AiChatRole, String)], // Previous messages for context
    api_key: Option<String>,
    provider: AiProvider,
    phi_available: bool,
) -> Result<String, String> {
    let use_phi = should_use_phi(provider, phi_available);

    // Build conversation context from history (last 4 exchanges max for Phi Silica)
    let max_history = if use_phi { 4 } else { 10 };
    let history_text: String = conversation_history
        .iter()
        .rev()
        .take(max_history)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter(|(role, _)| *role != AiChatRole::System) // Skip system messages
        .map(|(role, content)| {
            let role_name = match role {
                AiChatRole::User => "User",
                AiChatRole::Assistant => "Assistant",
                AiChatRole::System => "System",
            };
            format!("{}: {}", role_name, content)
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Build prompt with context and conversation history
    // Use specific instructions so AI actually analyzes the data
    let analysis_instructions = "Analyze the diagnostic data above. Include SPECIFIC values (e.g., '85% disk used', '16GB RAM'). Mention any concerns (high usage, errors). Give 2-3 actionable recommendations.";

    let prompt = if !history_text.is_empty() && !context.is_empty() {
        format!(
            "Previous conversation:\n{}\n\nDIAGNOSTIC DATA:\n{}\n\nUser question: {}\n\n{}",
            history_text, context, message, analysis_instructions
        )
    } else if !history_text.is_empty() {
        format!(
            "Previous conversation:\n{}\n\nUser: {}\n\nRespond based on the conversation. If asked about system status without data, say you need to run diagnostics first.",
            history_text, message
        )
    } else if !context.is_empty() {
        format!(
            "DIAGNOSTIC DATA:\n{}\n\nUser question: {}\n\n{}",
            context, message, analysis_instructions
        )
    } else {
        format!(
            "User asked: {}\n\nNo diagnostic data available. Provide general Windows advice or suggest running a scan first.",
            message
        )
    };

    if use_phi {
        // Phi Silica has 4k token context (NOT 128k - that was incorrect)
        // ~4 chars per token = ~16k chars max, but need room for output
        // Use 2500 chars for prompt to leave ~1500 chars for response
        const PHI_MAX_PROMPT_CHARS: usize = 2500;

        let truncated_prompt = if prompt.len() > PHI_MAX_PROMPT_CHARS {
            format!("{}...", &prompt[..PHI_MAX_PROMPT_CHARS])
        } else {
            prompt
        };

        wfdiag_tauri::phi_silica::analyze_with_phi_silica(truncated_prompt)
            .await
            .map(|r| r.analysis)
    } else if let Some(key) = api_key {
        if key.is_empty() {
            return Err(
                "OpenAI API key not configured. Go to Settings to add your key.".to_string(),
            );
        }
        analyze_with_openai(&key, &prompt).await
    } else {
        Err(
            "No AI provider available. Configure OpenAI API key in Settings or use a Copilot+ PC."
                .to_string(),
        )
    }
}

/// Analyze with OpenAI using function calling
/// This allows the AI to decide which diagnostics to run
async fn analyze_with_openai(api_key: &str, prompt: &str) -> Result<String, String> {
    use async_openai::types::responses::{CreateResponseArgs, InputParam, FunctionToolArgs};
    use async_openai::{config::OpenAIConfig, Client};
    use serde_json::json;

    let config = OpenAIConfig::new().with_api_key(api_key);
    let client = Client::with_config(config);

    // Get available diagnostic tasks
    let available_tasks = wfdiag_tauri::diagnostics::get_all_tasks();

    // Define the diagnostic tool for function calling
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

    // System instructions
    let instructions = format!(
        "You are a Windows system diagnostic assistant. When the user asks about their system, \
        IMMEDIATELY run relevant diagnostics using the run_diagnostic function. \
        Run at least 3-5 diagnostics for any system question. \
        Available diagnostics: {}",
        available_tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")
    );

    // Create request with function calling
    let request = CreateResponseArgs::default()
        .model(wfdiag_tauri::openai_integration::OPENAI_MODEL)
        .instructions(instructions)
        .input(InputParam::Text(prompt.to_string()))
        .tools(vec![diagnostic_tool.into()])
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .responses()
        .create(request)
        .await
        .map_err(|e| format!("API error: {}", e))?;

    // Process function calls if present
    let mut diagnostics_run = Vec::new();
    let mut diagnostic_context = String::new();

    for item in response.output.iter() {
        if let async_openai::types::responses::OutputItem::FunctionCall(func_call) = item {
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(&func_call.arguments) {
                if let Some(task_id) = args.get("task_id").and_then(|v| v.as_str()) {
                    diagnostics_run.push(task_id.to_string());

                    // Run the diagnostic
                    if let Ok(result) = wfdiag_tauri::diagnostics::run_diagnostic_task_sync(task_id) {
                        let summary = extract_key_data_static(&result.output, task_id);
                        if !summary.is_empty() {
                            diagnostic_context.push_str(&format!("{}: {}\n", task_id, summary.trim()));
                        }
                    }
                }
            }
        }
    }

    // If AI ran diagnostics via function calls, make a follow-up request with the results
    if !diagnostics_run.is_empty() {
        let follow_up = format!(
            "{}\n\nDiagnostic results:\n{}\n\nBased on these results, provide a helpful analysis.",
            prompt, diagnostic_context
        );

        let follow_up_request = CreateResponseArgs::default()
            .model(wfdiag_tauri::openai_integration::OPENAI_MODEL)
            .input(InputParam::Text(follow_up))
            .build()
            .map_err(|e| e.to_string())?;

        let follow_up_response = client
            .responses()
            .create(follow_up_request)
            .await
            .map_err(|e| format!("API error: {}", e))?;

        let analysis = follow_up_response.output_text().unwrap_or_default();
        let ran_list = diagnostics_run.join(", ");
        return Ok(format!("[Ran diagnostics: {}]\n\n{}", ran_list, analysis));
    }

    // No function calls - return the direct response
    Ok(response.output_text().unwrap_or_default())
}
