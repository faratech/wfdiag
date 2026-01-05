use crate::{ScanState, WfDiagApp};
use eframe::egui::{self, Color32, Margin, RichText};
use super::{ai, colors, components};

/// Get icon for a diagnostic task
fn get_task_icon(task_id: &str) -> &'static str {
    match task_id {
        // Hardware
        "processor" => "🔲",
        "physical_memory" | "device_memory" => "📊",
        "baseboard" => "🖥",
        "bios" => "⚡",
        "npu_info" => "🧠",

        // Storage
        "disk_drive" => "💿",
        "disk_partition" => "📁",
        "logical_disk" => "💾",

        // Network
        "network_adapter" => "📡",
        "ipconfig" => "🌐",

        // System
        "services" => "⚙",
        "system_driver" | "drivers_list" => "🔧",
        "comp_system" | "os_info" | "systeminfo" => "💻",
        "event_logs" => "📋",
        "installed_programs" | "store_apps" => "📦",
        "processes" => "📈",
        "windows_update" => "🔄",

        _ => "📄",
    }
}

pub fn show(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    // Clone results to avoid holding lock
    let (results_map, is_empty) = {
        let results = app.results.lock().unwrap();
        (results.clone(), results.is_empty())
    };

    if is_empty && app.scan_state != ScanState::Running {
        show_empty_state(ui);
        return;
    }

    // Build diagnostic list
    let mut diagnostics: Vec<DiagnosticItem> = Vec::new();
    for (task_id, result) in &results_map {
        if let Some(task) = app.available_tasks.iter().find(|t| &t.id == task_id) {
            let icon = get_task_icon(task_id);
            diagnostics.push(DiagnosticItem {
                id: task_id.clone(),
                name: task.name.clone(),
                category: task.category.clone(),
                icon,
                success: result.success,
                error: result.error.clone(),
                output: result.output.clone(),
                duration_ms: result.duration_ms,
            });
        }
    }

    // Sort by category then name
    diagnostics.sort_by(|a, b| {
        category_order(&a.category)
            .cmp(&category_order(&b.category))
            .then_with(|| a.name.cmp(&b.name))
    });

    // Get selected item
    let selected_id = app
        .expanded_rows
        .iter()
        .find(|(_, expanded)| **expanded)
        .map(|(id, _)| id.clone());

    // Two-panel layout
    let panel_width = 260.0;

    egui::SidePanel::left("diag_list")
        .exact_width(panel_width)
        .resizable(false)
        .show_inside(ui, |ui| {
            render_list_panel(app, ui, &diagnostics, &selected_id);
        });

    // Clone selected item data for the detail panel
    let selected_item = selected_id.as_ref()
        .and_then(|sel_id| diagnostics.iter().find(|d| &d.id == sel_id).cloned());

    egui::CentralPanel::default().show_inside(ui, |ui| {
        if let Some(item) = selected_item {
            render_detail_panel(ui, &item, app);
        } else {
            show_select_prompt(ui);
        }
    });
}

fn show_empty_state(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        ui.label(RichText::new("No Results").size(20.0).strong());
        ui.add_space(12.0);
        ui.label(
            RichText::new("Click Quick or Full to run diagnostics")
                .size(14.0)
                .weak(),
        );
    });
}

fn show_select_prompt(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        ui.label(RichText::new("Select a diagnostic").size(16.0).weak());
        ui.add_space(8.0);
        ui.label(
            RichText::new("Click an item on the left to view details")
                .size(13.0)
                .weak(),
        );
    });
}

fn render_list_panel(
    app: &mut WfDiagApp,
    ui: &mut egui::Ui,
    diagnostics: &[DiagnosticItem],
    selected_id: &Option<String>,
) {
    ui.add_space(8.0);

    // Summary header
    let passed = diagnostics.iter().filter(|d| d.success).count();
    let failed = diagnostics.len() - passed;

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} diagnostics", diagnostics.len()))
                .size(13.0)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if failed > 0 {
                ui.label(
                    RichText::new(format!("{} issues", failed))
                        .size(11.0)
                        .color(colors::ERROR),
                );
            }
        });
    });

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    // Scrollable list grouped by category
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let categories = ["Hardware", "Storage", "Network", "System"];

            for category in categories {
                let items: Vec<&DiagnosticItem> = diagnostics
                    .iter()
                    .filter(|d| {
                        if category == "System" {
                            !["Hardware", "Storage", "Network"].contains(&d.category.as_str())
                        } else {
                            d.category == category
                        }
                    })
                    .collect();

                if items.is_empty() {
                    continue;
                }

                // Category header
                ui.add_space(8.0);
                let cat_icon = match category {
                    "Hardware" => "🖥",
                    "Storage" => "💾",
                    "Network" => "🌐",
                    _ => "⚙",
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(cat_icon).size(11.0));
                    ui.label(RichText::new(category).size(12.0).strong());

                    let cat_failed = items.iter().filter(|d| !d.success).count();
                    if cat_failed > 0 {
                        ui.label(
                            RichText::new(format!("• {}", cat_failed))
                                .size(10.0)
                                .color(colors::ERROR),
                        );
                    }
                });
                ui.add_space(4.0);

                // Items in this category
                for item in items {
                    let is_selected = selected_id.as_ref() == Some(&item.id);

                    // Status indicator
                    let status_color = if item.success {
                        colors::SUCCESS
                    } else {
                        colors::ERROR
                    };

                    // Full-width clickable row
                    let row_height = 32.0;
                    let available_width = ui.available_width();

                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(available_width, row_height),
                        egui::Sense::click(),
                    );

                    // Draw background on hover or selection
                    let is_dark = ui.visuals().dark_mode;
                    if is_selected {
                        // Use stronger background that works in both light and dark mode
                        let bg_color = if is_dark {
                            Color32::from_rgba_unmultiplied(59, 130, 246, 80) // Blue with alpha
                        } else {
                            Color32::from_rgba_unmultiplied(59, 130, 246, 40) // Lighter for light mode
                        };
                        ui.painter().rect_filled(rect, 4.0, bg_color);
                    } else if response.hovered() {
                        let hover_color = if is_dark {
                            Color32::from_rgba_unmultiplied(128, 128, 128, 30)
                        } else {
                            Color32::from_rgba_unmultiplied(0, 0, 0, 15)
                        };
                        ui.painter().rect_filled(rect, 4.0, hover_color);
                    }

                    // Draw content
                    let text_pos = rect.left_center() + egui::vec2(12.0, 0.0);

                    // Status dot
                    ui.painter().circle_filled(
                        rect.left_center() + egui::vec2(16.0, 0.0),
                        4.0,
                        status_color,
                    );

                    // Icon and name - use theme-aware text color
                    let text_color = if is_selected {
                        if is_dark {
                            Color32::WHITE
                        } else {
                            Color32::from_rgb(30, 64, 175) // Dark blue for light mode selection
                        }
                    } else {
                        ui.visuals().text_color()
                    };

                    ui.painter().text(
                        text_pos + egui::vec2(16.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{} {}", item.icon, item.name),
                        egui::FontId::proportional(12.0),
                        text_color,
                    );

                    // Handle click
                    if response.clicked() {
                        // Clear all selections
                        for v in app.expanded_rows.values_mut() {
                            *v = false;
                        }
                        // Select this item
                        app.expanded_rows.insert(item.id.clone(), true);
                    }

                    // Show pointer cursor on hover
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }
            }

            ui.add_space(16.0);
        });
}

fn render_detail_panel(ui: &mut egui::Ui, item: &DiagnosticItem, app: &mut WfDiagApp) {
    ui.add_space(12.0);

    // Header
    ui.horizontal(|ui| {
        // Icon
        ui.label(RichText::new(item.icon).size(24.0));

        ui.add_space(8.0);

        ui.vertical(|ui| {
            ui.label(RichText::new(&item.name).size(18.0).strong());
            ui.horizontal(|ui| {
                let (status_text, status_color) = if item.success {
                    ("Passed", colors::SUCCESS)
                } else {
                    ("Failed", colors::ERROR)
                };
                ui.label(RichText::new(status_text).size(12.0).color(status_color));
                ui.label(RichText::new("•").size(12.0).weak());
                ui.label(
                    RichText::new(format!("{}ms", item.duration_ms))
                        .size(12.0)
                        .weak(),
                );
            });
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            if ui.button("📋 Copy").clicked() {
                let text = format_output_for_copy(&item.output, &item.error);
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(text);
                }
            }
        });
    });

    ui.add_space(12.0);

    // AI Interpretation Panel
    ai::render_ai_panel(ui, app, &item.id, &item.name, &item.output, item.success);

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    // Content area
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Error display
            if let Some(ref error) = item.error {
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(220, 60, 60, 25))
                    .corner_radius(6.0)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Error")
                                .size(13.0)
                                .strong()
                                .color(colors::ERROR),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(error)
                                .size(12.0)
                                .color(Color32::from_rgb(220, 120, 120)),
                        );
                    });
                return;
            }

            // Empty output
            if item.output.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(RichText::new("No output data").size(14.0).weak());
                });
                return;
            }

            // Try direct JSON parsing first (the output should already be valid JSON)
            match serde_json::from_str::<serde_json::Value>(&item.output) {
                Ok(json) => {
                    render_json_output(ui, &json);
                }
                Err(e) => {
                    // Show parse error for debugging
                    ui.label(
                        RichText::new(format!("⚠ JSON parse error: {}", e))
                            .size(10.0)
                            .color(Color32::from_rgb(200, 150, 50)),
                    );
                    ui.add_space(8.0);

                    // Fallback: try comprehensive parser for mixed formats
                    let sections = parse_diagnostic_output(&item.output);
                    if !sections.is_empty() {
                        render_parsed_output(ui, &sections);
                    } else {
                        // Final fallback to plain text - show first 500 chars
                        let preview = if item.output.len() > 500 {
                            format!("{}...", &item.output[..500])
                        } else {
                            item.output.clone()
                        };
                        egui::Frame::new()
                            .fill(ui.visuals().extreme_bg_color)
                            .corner_radius(4.0)
                            .inner_margin(Margin::same(12))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(preview)
                                        .size(11.0)
                                        .family(egui::FontFamily::Monospace),
                                );
                            });
                    }
                }
            }
        });
}

/// Simple recursive JSON renderer - like the React version
/// Always expands all objects and arrays, no summarization
pub fn render_json_output(ui: &mut egui::Ui, value: &serde_json::Value) {
    render_value_recursive(ui, value, 0);
}

/// Recursively render any JSON value
fn render_value_recursive(ui: &mut egui::Ui, value: &serde_json::Value, depth: usize) {
    match value {
        serde_json::Value::Null => {
            ui.label(RichText::new("null").size(11.0).weak());
        }
        serde_json::Value::Bool(b) => {
            let (text, color) = if *b {
                ("True", colors::SUCCESS)
            } else {
                ("False", Color32::from_rgb(180, 100, 100))
            };
            ui.label(RichText::new(text).size(11.0).color(color));
        }
        serde_json::Value::Number(n) => {
            ui.label(
                RichText::new(n.to_string())
                    .size(11.0)
                    .color(Color32::from_rgb(100, 149, 237)),
            );
        }
        serde_json::Value::String(s) => {
            if s.is_empty() {
                ui.label(RichText::new("(empty)").size(11.0).weak());
            } else {
                // Truncate very long strings
                let display = if s.len() > 200 {
                    format!("{}…", &s[..197])
                } else {
                    s.clone()
                };
                ui.label(RichText::new(display).size(11.0));
            }
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                ui.label(RichText::new("[]").size(11.0).weak());
            } else {
                // Render each array item with indent
                for (i, item) in arr.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(4.0);
                    }

                    ui.horizontal(|ui| {
                        // Left border for visual hierarchy
                        if depth > 0 {
                            ui.add_space(8.0);
                            ui.label(RichText::new("│").size(11.0).weak());
                            ui.add_space(4.0);
                        }

                        ui.vertical(|ui| {
                            render_value_recursive(ui, item, depth + 1);
                        });
                    });
                }
            }
        }
        serde_json::Value::Object(obj) => {
            if obj.is_empty() {
                ui.label(RichText::new("{}").size(11.0).weak());
            } else {
                // Sort keys for consistent display
                let mut keys: Vec<_> = obj.keys().collect();
                keys.sort_by(|a, b| field_priority(a).cmp(&field_priority(b)));

                // Filter out system fields and empty values
                let filtered_keys: Vec<_> = keys
                    .into_iter()
                    .filter(|k| !should_skip_field(k) && !is_empty_value(obj.get(*k).unwrap()))
                    .collect();

                if filtered_keys.is_empty() {
                    ui.label(RichText::new("(no data)").size(11.0).weak());
                    return;
                }

                // Render as a grid for objects
                egui::Grid::new(ui.id().with(format!("obj_grid_{}", depth)))
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for key in filtered_keys {
                            let val = obj.get(key).unwrap();

                            // Key label
                            ui.label(RichText::new(format_field_name(key)).size(11.0).weak());

                            // Value - recursively render
                            ui.vertical(|ui| {
                                render_value_recursive(ui, val, depth + 1);
                            });

                            ui.end_row();
                        }
                    });
            }
        }
    }
}

fn render_json_object(ui: &mut egui::Ui, value: &serde_json::Value, index: usize) {
    let obj = match value.as_object() {
        Some(o) => o,
        None => {
            ui.label(value.to_string());
            return;
        }
    };

    // Get a title for this object
    let title = get_object_title(obj);

    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(6.0)
        .inner_margin(Margin::same(12))
        .stroke(egui::Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .show(ui, |ui| {
            // Title row
            if !title.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("#{}", index + 1)).size(10.0).weak());
                    ui.label(RichText::new(&title).size(13.0).strong());
                });
                ui.add_space(8.0);
            }

            // Check if any field is a nested object/array that should be expanded
            let mut fields: Vec<_> = obj.iter().collect();
            fields.sort_by(|(a, _), (b, _)| field_priority(a).cmp(&field_priority(b)));

            // Separate simple fields from complex ones
            let (simple_fields, complex_fields): (Vec<_>, Vec<_>) = fields
                .into_iter()
                .filter(|(key, val)| !should_skip_field(key) && !is_empty_value(val))
                .partition(|(_, val)| {
                    // Check if this is a simple (primitive) value or a simple array
                    match val {
                        serde_json::Value::String(_)
                        | serde_json::Value::Number(_)
                        | serde_json::Value::Bool(_)
                        | serde_json::Value::Null => true,
                        serde_json::Value::Array(arr) => arr.iter().all(|v| !v.is_object()),
                        serde_json::Value::Object(_) => false,
                    }
                });

            // Render simple fields in a grid
            if !simple_fields.is_empty() {
                egui::Grid::new(format!("obj_grid_{}", index))
                    .num_columns(2)
                    .spacing([20.0, 6.0])
                    .show(ui, |ui| {
                        for (key, val) in &simple_fields {
                            ui.label(RichText::new(format_field_name(key)).size(11.0).weak());
                            render_field_value(ui, key, val);
                            ui.end_row();
                        }
                    });
            }

            // Render complex fields (nested objects/arrays) expanded
            for (key, val) in complex_fields {
                if !simple_fields.is_empty() {
                    ui.add_space(8.0);
                }
                ui.label(RichText::new(format_field_name(key)).size(12.0).strong());
                ui.add_space(4.0);
                render_json_output(ui, val);
            }
        });
}

fn render_field_value(ui: &mut egui::Ui, key: &str, value: &serde_json::Value) {
    match value {
        serde_json::Value::Bool(b) => {
            let (text, color) = if *b {
                ("Yes", colors::SUCCESS)
            } else {
                ("No", Color32::from_rgb(180, 100, 100))
            };
            ui.label(RichText::new(text).size(11.0).color(color));
        }
        serde_json::Value::Number(n) => {
            let text = format_number(key, n);
            ui.label(RichText::new(text).size(11.0));
        }
        serde_json::Value::String(s) => {
            let display = format_string_value(key, s);
            ui.label(RichText::new(display).size(11.0));
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                ui.label(RichText::new("—").size(11.0).weak());
            } else {
                // Show array contents inline (only for simple arrays)
                let items: Vec<String> = arr
                    .iter()
                    .take(10)
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => if *b { "Yes" } else { "No" }.to_string(),
                        _ => v.to_string(),
                    })
                    .collect();

                let display = if arr.len() > 10 {
                    format!("{}, +{} more", items.join(", "), arr.len() - 10)
                } else {
                    items.join(", ")
                };
                ui.label(RichText::new(display).size(11.0));
            }
        }
        // Objects should NOT reach here - they should be handled by complex_fields path
        // But as a fallback, show a brief summary
        serde_json::Value::Object(obj) => {
            if obj.is_empty() {
                ui.label(RichText::new("—").size(11.0).weak());
            } else {
                // Try to get a meaningful summary
                let summary: Vec<String> = obj
                    .iter()
                    .take(3)
                    .filter_map(|(k, v)| match v {
                        serde_json::Value::String(s) if !s.is_empty() && s.len() < 30 => {
                            Some(format!("{}: {}", k, s))
                        }
                        serde_json::Value::Number(n) => Some(format!("{}: {}", k, n)),
                        serde_json::Value::Bool(b) => {
                            Some(format!("{}: {}", k, if *b { "Yes" } else { "No" }))
                        }
                        _ => None,
                    })
                    .collect();

                if !summary.is_empty() {
                    ui.label(RichText::new(summary.join(", ")).size(11.0));
                } else {
                    ui.label(
                        RichText::new(format!("({} properties)", obj.len()))
                            .size(11.0)
                            .weak(),
                    );
                }
            }
        }
        serde_json::Value::Null => {
            ui.label(RichText::new("—").size(11.0).weak());
        }
    }
}

/// Comprehensive parser for diagnostic output - handles JSON, mixed formats, key-value pairs
pub fn parse_diagnostic_output(output: &str) -> Vec<OutputSection> {
    let mut sections = Vec::new();
    let mut pos = 0;
    let text = output.trim();

    if text.is_empty() {
        return sections;
    }

    // First, try parsing whole thing as JSON
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
        sections.push(OutputSection::Json(None, json));
        return sections;
    }

    // Scan for JSON objects/arrays with optional preceding keys
    while pos < text.len() {
        let remaining = &text[pos..];

        // Find next { or [
        let next_json = remaining
            .chars()
            .enumerate()
            .find(|(_, c)| *c == '{' || *c == '[')
            .map(|(i, c)| (i, c));

        if let Some((brace_idx, brace_char)) = next_json {
            // Text before the brace
            let before = &remaining[..brace_idx];

            // Parse text before as key-value lines, extract last word as JSON label
            let (kv_lines, json_label) = parse_text_before_json(before);
            for (k, v) in kv_lines {
                sections.push(OutputSection::KeyValue(k, v));
            }

            // Find matching closing brace
            let close_char = if brace_char == '{' { '}' } else { ']' };
            let json_text = &remaining[brace_idx..];

            if let Some(end_idx) = find_json_end(json_text, brace_char, close_char) {
                let json_str = &json_text[..=end_idx];

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                    sections.push(OutputSection::Json(json_label, json));
                    pos += brace_idx + end_idx + 1;
                    continue;
                }
            }

            // JSON parsing failed, treat the label + brace as text
            if let Some(label) = json_label {
                // Find end of this "line" (next newline after the failed JSON start)
                let line_end = remaining[brace_idx..]
                    .find('\n')
                    .map(|n| brace_idx + n)
                    .unwrap_or(remaining.len());
                let content = &remaining[..line_end];
                sections.push(OutputSection::KeyValue(label, content.to_string()));
                pos += line_end;
                continue;
            }
        }

        // No more JSON, parse remaining as key-value lines
        for line in remaining.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((k, v)) = parse_kv_line(line) {
                sections.push(OutputSection::KeyValue(k, v));
            } else {
                sections.push(OutputSection::Text(line.to_string()));
            }
        }
        break;
    }

    sections
}

/// Parsed section of diagnostic output
pub enum OutputSection {
    Json(Option<String>, serde_json::Value),
    KeyValue(String, String),
    Text(String),
}

/// Parse text before a JSON object, return key-value pairs and optional JSON label
fn parse_text_before_json(text: &str) -> (Vec<(String, String)>, Option<String>) {
    let mut pairs = Vec::new();
    let text = text.trim();

    if text.is_empty() {
        return (pairs, None);
    }

    // Split by newlines
    let lines: Vec<&str> = text.lines().collect();

    if lines.is_empty() {
        // Single line - might be just the label
        let label = extract_identifier(text);
        if !label.is_empty() {
            let prefix = text.strip_suffix(label).unwrap_or("").trim();
            if !prefix.is_empty() {
                if let Some((k, v)) = parse_kv_line(prefix) {
                    pairs.push((k, v));
                }
            }
            return (pairs, Some(label.to_string()));
        }
        return (pairs, None);
    }

    // Multiple lines - last line might contain the label
    let last_line = lines.last().unwrap().trim();
    let label = extract_identifier(last_line);

    // Parse all complete lines as key-value pairs
    for (i, line) in lines.iter().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if i == lines.len() - 1 && !label.is_empty() {
            // Last line - check if there's content before the label
            let prefix = line.strip_suffix(label).unwrap_or("").trim();
            if !prefix.is_empty() {
                if let Some((k, v)) = parse_kv_line(prefix) {
                    pairs.push((k, v));
                }
            }
        } else {
            if let Some((k, v)) = parse_kv_line(line) {
                pairs.push((k, v));
            }
        }
    }

    let json_label = if !label.is_empty() {
        Some(label.to_string())
    } else {
        None
    };
    (pairs, json_label)
}

/// Extract identifier (last word that looks like a variable name)
fn extract_identifier(s: &str) -> &str {
    let s = s.trim();
    let end = s.len();
    let mut start = end;

    for (i, c) in s.char_indices().rev() {
        if c.is_alphanumeric() || c == '_' {
            start = i;
        } else if start != end {
            break;
        }
    }

    &s[start..end]
}

/// Find the end of a JSON object/array, handling nested structures
fn find_json_end(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    let mut in_string = false;
    let mut escape = false;

    for (i, c) in s.chars().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_string {
            if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse a single line as key-value
fn parse_kv_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Try colon separator
    if let Some(pos) = line.find(": ") {
        let k = line[..pos].trim();
        let v = line[pos + 2..].trim();
        if !k.is_empty() && !v.is_empty() {
            return Some((k.to_string(), v.to_string()));
        }
    }

    // Try equals separator
    if let Some(pos) = line.find(" = ") {
        let k = line[..pos].trim();
        let v = line[pos + 3..].trim();
        if !k.is_empty() && !v.is_empty() {
            return Some((k.to_string(), v.to_string()));
        }
    }

    // Try space separator (key must be identifier-like)
    if let Some(pos) = line.find(' ') {
        let k = &line[..pos];
        let v = line[pos + 1..].trim();
        if !k.is_empty()
            && !v.is_empty()
            && k.chars().all(|c| c.is_alphanumeric() || c == '_')
            && k.chars().any(|c| c.is_alphabetic())
        {
            return Some((k.to_string(), v.to_string()));
        }
    }

    None
}

/// Render parsed output sections with proper formatting
pub fn render_parsed_output(ui: &mut egui::Ui, sections: &[OutputSection]) {
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            ui.add_space(8.0);
        }

        match section {
            OutputSection::Json(label, json) => {
                if let Some(name) = label {
                    ui.label(RichText::new(format_field_name(name)).size(13.0).strong());
                    ui.add_space(4.0);
                }
                render_json_output(ui, json);
            }
            OutputSection::KeyValue(key, value) => {
                // Check if value looks like it might be JSON
                let trimmed = value.trim();
                if (trimmed.starts_with('{') || trimmed.starts_with('[')) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                        ui.label(RichText::new(format_field_name(key)).size(12.0).strong());
                        ui.add_space(4.0);
                        render_json_output(ui, &json);
                        continue;
                    }
                }

                // Regular key-value display
                egui::Frame::new()
                    .fill(ui.visuals().extreme_bg_color)
                    .corner_radius(4.0)
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format_field_name(key)).size(11.0).weak());
                            // Truncate very long values
                            let display_val = if value.len() > 100 {
                                format!("{}…", &value[..97])
                            } else {
                                value.clone()
                            };
                            ui.label(RichText::new(display_val).size(11.0));
                        });
                    });
            }
            OutputSection::Text(text) => {
                ui.label(RichText::new(text).size(11.0));
            }
        }
    }
}

// Helper structs and functions

#[derive(Clone)]
struct DiagnosticItem {
    id: String,
    name: String,
    category: String,
    icon: &'static str,
    success: bool,
    error: Option<String>,
    output: String,
    duration_ms: u64,
}

fn category_order(cat: &str) -> u8 {
    match cat {
        "Hardware" => 0,
        "Storage" => 1,
        "Network" => 2,
        _ => 3,
    }
}

fn get_object_title(obj: &serde_json::Map<String, serde_json::Value>) -> String {
    for key in ["Name", "Caption", "DisplayName", "DeviceID", "FriendlyName"] {
        if let Some(serde_json::Value::String(s)) = obj.get(key) {
            if !s.is_empty() && s.len() < 80 {
                return s.clone();
            }
        }
    }
    String::new()
}

fn should_skip_field(key: &str) -> bool {
    key.starts_with("__")
        || matches!(
            key,
            "PSComputerName"
                | "Scope"
                | "Options"
                | "ClassPath"
                | "Properties"
                | "SystemProperties"
                | "Qualifiers"
                | "Site"
                | "Container"
                | "CSName"
                | "SystemName"
                | "PSShowComputerName"
                | "CimClass"
                | "CimInstanceProperties"
                | "CimSystemProperties"
                | "CreationClassName"
                | "SystemCreationClassName"
        )
}

fn is_empty_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        serde_json::Value::Array(arr) => arr.is_empty(),
        _ => false,
    }
}

fn field_priority(key: &str) -> u8 {
    match key {
        "Name" | "Caption" | "DisplayName" => 0,
        "Description" => 1,
        "Status" | "State" => 2,
        "Version" | "DriverVersion" => 3,
        "Size" | "Capacity" | "FreeSpace" => 4,
        _ => 50,
    }
}

fn format_field_name(key: &str) -> String {
    // Common mappings
    let mappings = [
        ("IPAddress", "IP Address"),
        ("MACAddress", "MAC"),
        ("DHCPEnabled", "DHCP"),
        ("FreeSpace", "Free"),
        ("MaxClockSpeed", "Max Speed"),
        ("CurrentClockSpeed", "Speed"),
        ("NumberOfCores", "Cores"),
        ("NumberOfLogicalProcessors", "Threads"),
        ("DriverVersion", "Version"),
        ("TotalPhysicalMemory", "Total RAM"),
    ];

    for (k, v) in mappings {
        if key == k {
            return v.to_string();
        }
    }

    // Convert CamelCase to spaced
    let mut result = String::new();
    for (i, c) in key.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push(' ');
        }
        result.push(c);
    }
    result
}

fn format_number(key: &str, n: &serde_json::Number) -> String {
    if let Some(i) = n.as_u64() {
        // Size fields
        if key.contains("Size")
            || key.contains("Space")
            || key.contains("Capacity")
            || key.contains("Memory")
        {
            return format_bytes(i);
        }
        // Speed fields
        if key.contains("Speed") {
            if i >= 1_000_000_000 {
                return format!("{:.1} Gbps", i as f64 / 1_000_000_000.0);
            } else if i >= 1_000_000 {
                return format!("{} Mbps", i / 1_000_000);
            }
        }
        if key.contains("Clock") {
            return format!("{} MHz", i);
        }
    }
    n.to_string()
}

fn format_string_value(key: &str, s: &str) -> String {
    // Format dates
    if key.contains("Date") && s.len() >= 8 && s.chars().take(8).all(|c| c.is_ascii_digit()) {
        return format!("{}-{}-{}", &s[0..4], &s[4..6], &s[6..8]);
    }
    // Truncate long strings
    if s.len() > 60 {
        return format!("{}…", &s[..57]);
    }
    s.to_string()
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn format_output_for_copy(output: &str, error: &Option<String>) -> String {
    if let Some(e) = error {
        return format!("Error: {}", e);
    }
    if output.is_empty() {
        return "No output".to_string();
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
        return serde_json::to_string_pretty(&json).unwrap_or_else(|_| output.to_string());
    }
    output.to_string()
}
