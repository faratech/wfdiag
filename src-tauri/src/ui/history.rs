//! Scan History Tab - View, compare, and manage past diagnostic scans

use super::diagnostics::{parse_diagnostic_output, render_json_output, render_parsed_output};
use super::{colors, components};
use crate::{HistoryView, WfDiagApp};
use eframe::egui::{self, Color32, Margin, RichText};

pub fn show(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    match app.history_view {
        HistoryView::List => show_list(app, ui),
        HistoryView::Detail => show_detail(app, ui),
        HistoryView::Compare => show_comparison(app, ui),
    }
}

fn show_list(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    // Header
    components::page_header(ui, "📜 Scan History", |ui| {
        if !app.stored_history.is_empty() {
            if ui.button("🗑 Clear All").clicked() {
                app.clear_history();
            }

            ui.add_space(components::SPACE_SM);

            if ui.button("↻ Refresh").clicked() {
                app.load_history();
            }
        }
    });

    components::section_separator(ui);

    // Check if comparing
    if app.compare_scan_id.is_some() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("⚖ Compare Mode: Select second scan to compare")
                    .size(12.0)
                    .color(colors::INFO),
            );
            if ui.small_button("Cancel").clicked() {
                app.compare_scan_id = None;
            }
        });
        ui.add_space(components::SPACE_SM);
    }

    if app.stored_history.is_empty() {
        components::empty_state(
            ui,
            "📜",
            "No Scan History",
            "Complete a diagnostic scan to see history here.\nScans are automatically saved and can be compared.",
        );
        return;
    }

    // Stats summary
    let total_scans = app.stored_history.len();
    let total_tasks: usize = app.stored_history.iter().map(|s| s.task_count).sum();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} scans", total_scans))
                .size(12.0)
                .weak(),
        );
        ui.label(RichText::new("•").size(12.0).weak());
        ui.label(
            RichText::new(format!("{} total tasks run", total_tasks))
                .size(12.0)
                .weak(),
        );
    });

    ui.add_space(8.0);

    // Clone history to avoid borrow conflicts
    let history = app.stored_history.clone();
    let compare_id = app.compare_scan_id.clone();

    // Scan list
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, scan) in history.iter().enumerate() {
            let time_str = scan.timestamp.format("%Y-%m-%d %H:%M:%S");
            let duration_str = components::format_duration(scan.duration_ms);
            let is_selected_for_compare = compare_id.as_ref() == Some(&scan.id);

            let frame_color = if is_selected_for_compare {
                colors::INFO.linear_multiply(0.3)
            } else {
                ui.visuals().extreme_bg_color
            };

            egui::Frame::new()
                .fill(frame_color)
                .corner_radius(4.0)
                .inner_margin(Margin::same(12))
                .stroke(egui::Stroke::new(
                    if is_selected_for_compare { 2.0 } else { 1.0 },
                    if is_selected_for_compare {
                        colors::INFO
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke.color
                    },
                ))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            // Title row
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("Scan #{}", total_scans - i))
                                        .size(13.0)
                                        .strong(),
                                );
                                ui.label(RichText::new(time_str).size(11.0).weak());
                                if is_selected_for_compare {
                                    ui.label(
                                        RichText::new("[Selected]")
                                            .size(10.0)
                                            .color(colors::INFO),
                                    );
                                }
                            });

                            ui.add_space(4.0);

                            // Stats row
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("✓ {}", scan.success_count))
                                        .size(11.0)
                                        .color(colors::SUCCESS),
                                );

                                if scan.failure_count > 0 {
                                    ui.label(
                                        RichText::new(format!("✗ {}", scan.failure_count))
                                            .size(11.0)
                                            .color(colors::ERROR),
                                    );
                                }

                                ui.label(
                                    RichText::new(format!("⏱ {}", duration_str))
                                        .size(11.0)
                                        .weak(),
                                );
                                ui.label(
                                    RichText::new(format!("({} tasks)", scan.task_count))
                                        .size(11.0)
                                        .weak(),
                                );

                                if !scan.computer_name.is_empty() {
                                    ui.label(
                                        RichText::new(format!("💻 {}", scan.computer_name))
                                            .size(11.0)
                                            .weak(),
                                    );
                                }
                            });
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let scan_id = scan.id.clone();

                            // View button
                            if ui.button("View").clicked() {
                                app.view_scan(&scan_id);
                            }

                            // Compare button
                            if compare_id.is_some() {
                                if !is_selected_for_compare {
                                    if ui.button("Compare").clicked() {
                                        app.compare_scans(&scan_id);
                                    }
                                }
                            } else {
                                if ui
                                    .button("⚖")
                                    .on_hover_text("Select for comparison")
                                    .clicked()
                                {
                                    app.select_for_compare(&scan_id);
                                }
                            }
                        });
                    });
                });

            ui.add_space(4.0);
        }
    });
}

fn show_detail(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    // Back button
    ui.horizontal(|ui| {
        if ui.button("← Back to List").clicked() {
            app.history_view = HistoryView::List;
            app.selected_scan = None;
        }
    });

    ui.add_space(8.0);

    // Clone scan data to avoid borrow conflicts
    let scan = match app.selected_scan.clone() {
        Some(s) => s,
        None => {
            ui.label("No scan selected");
            return;
        }
    };

    // Header
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("📋 Scan: {}", scan.id))
                .size(16.0)
                .strong(),
        );
    });

    ui.add_space(4.0);

    // Scan info
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(scan.timestamp.format("%Y-%m-%d %H:%M:%S"))
                .size(12.0)
                .weak(),
        );
        ui.label(RichText::new("•").weak());
        ui.label(RichText::new(&scan.computer_name).size(12.0).weak());
        ui.label(RichText::new("•").weak());
        ui.label(RichText::new(&scan.os_version).size(12.0).weak());
    });

    ui.add_space(4.0);

    // Stats
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("✓ {} passed", scan.success_count))
                .size(12.0)
                .color(colors::SUCCESS),
        );
        ui.label(
            RichText::new(format!("✗ {} failed", scan.failure_count))
                .size(12.0)
                .color(if scan.failure_count > 0 {
                    colors::ERROR
                } else {
                    Color32::GRAY
                }),
        );
        ui.label(
            RichText::new(format!("⏱ {}", components::format_duration(scan.duration_ms)))
                .size(12.0)
                .weak(),
        );
    });

    ui.add_space(8.0);

    // AI Scan Summary Panel
    super::ai::render_scan_ai_panel(ui, app);

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);

    // Results grouped by category
    let mut categories: std::collections::HashMap<
        String,
        Vec<(String, wfdiag_tauri::diagnostics::TaskResult)>,
    > = std::collections::HashMap::new();

    for (task_id, result) in scan.results.iter() {
        let category = get_task_category(task_id);
        categories
            .entry(category)
            .or_default()
            .push((task_id.clone(), result.clone()));
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut sorted_categories: Vec<_> = categories.keys().collect();
        sorted_categories.sort();

        for category in sorted_categories {
            if let Some(tasks) = categories.get(category) {
                let passed = tasks.iter().filter(|(_, r)| r.success).count();
                let failed = tasks.len() - passed;

                let header_id = format!("hist_cat_{}", category);
                let is_expanded = *app.expanded_history_tasks.get(&header_id).unwrap_or(&false);

                ui.horizontal(|ui| {
                    let arrow = if is_expanded { "▼" } else { "▶" };
                    if ui.button(RichText::new(arrow).size(10.0)).clicked() {
                        app.expanded_history_tasks
                            .insert(header_id.clone(), !is_expanded);
                    }

                    ui.label(RichText::new(category).size(13.0).strong());
                    ui.label(
                        RichText::new(format!("({} passed, {} failed)", passed, failed))
                            .size(11.0)
                            .weak(),
                    );
                });

                if is_expanded {
                    ui.indent(&header_id, |ui| {
                        for (task_id, result) in tasks {
                            let task_expanded_id = format!("hist_task_{}", task_id);
                            let task_expanded = *app
                                .expanded_history_tasks
                                .get(&task_expanded_id)
                                .unwrap_or(&false);

                            let status_icon = if result.success { "✓" } else { "✗" };
                            let status_color = if result.success {
                                colors::SUCCESS
                            } else {
                                colors::ERROR
                            };

                            ui.horizontal(|ui| {
                                if ui
                                    .button(
                                        RichText::new(if task_expanded { "▼" } else { "▶" })
                                            .size(9.0),
                                    )
                                    .clicked()
                                {
                                    app.expanded_history_tasks
                                        .insert(task_expanded_id.clone(), !task_expanded);
                                }

                                ui.label(
                                    RichText::new(status_icon).size(11.0).color(status_color),
                                );
                                ui.label(RichText::new(get_task_name(task_id)).size(11.0));
                            });

                            if task_expanded && !result.output.is_empty() {
                                ui.indent(&task_expanded_id, |ui| {
                                    egui::Frame::new()
                                        .fill(ui.visuals().extreme_bg_color)
                                        .corner_radius(4.0)
                                        .inner_margin(Margin::same(8))
                                        .show(ui, |ui| {
                                            // Try direct JSON parsing first
                                            match serde_json::from_str::<serde_json::Value>(
                                                &result.output,
                                            ) {
                                                Ok(json) => {
                                                    render_json_output(ui, &json);
                                                }
                                                Err(_) => {
                                                    // Fallback: try comprehensive parser
                                                    let sections =
                                                        parse_diagnostic_output(&result.output);
                                                    if !sections.is_empty() {
                                                        render_parsed_output(ui, &sections);
                                                    } else {
                                                        // Final fallback to plain text
                                                        let output = if result.output.len() > 2000 {
                                                            format!(
                                                                "{}...\n[truncated]",
                                                                &result.output[..2000]
                                                            )
                                                        } else {
                                                            result.output.clone()
                                                        };
                                                        ui.label(
                                                            RichText::new(&output)
                                                                .size(10.0)
                                                                .monospace(),
                                                        );
                                                    }
                                                }
                                            }
                                        });
                                });
                            }
                        }
                    });
                }

                ui.add_space(4.0);
            }
        }
    });
}

fn show_comparison(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    // Back button
    ui.horizontal(|ui| {
        if ui.button("← Back to List").clicked() {
            app.history_view = HistoryView::List;
            app.comparison_result = None;
        }
    });

    ui.add_space(8.0);

    // Clone comparison data to avoid borrow conflicts
    let comparison = match app.comparison_result.clone() {
        Some(c) => c,
        None => {
            ui.label("No comparison data");
            return;
        }
    };

    // Header
    ui.label(RichText::new("⚖ Scan Comparison").size(18.0).strong());

    ui.add_space(8.0);

    // Comparison summary
    ui.horizontal(|ui| {
        // Previous scan
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .corner_radius(4.0)
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_min_width(200.0);
                ui.label(RichText::new("Previous Scan").size(11.0).weak());
                ui.label(
                    RichText::new(comparison.previous_scan.timestamp.format("%Y-%m-%d %H:%M"))
                        .size(12.0)
                        .strong(),
                );
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("✓ {}", comparison.previous_scan.success_count))
                            .size(11.0)
                            .color(colors::SUCCESS),
                    );
                    ui.label(
                        RichText::new(format!("✗ {}", comparison.previous_scan.failure_count))
                            .size(11.0)
                            .color(colors::ERROR),
                    );
                });
            });

        ui.label(RichText::new("→").size(20.0).weak());

        // Current scan
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .corner_radius(4.0)
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_min_width(200.0);
                ui.label(RichText::new("Current Scan").size(11.0).weak());
                ui.label(
                    RichText::new(comparison.current_scan.timestamp.format("%Y-%m-%d %H:%M"))
                        .size(12.0)
                        .strong(),
                );
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("✓ {}", comparison.current_scan.success_count))
                            .size(11.0)
                            .color(colors::SUCCESS),
                    );
                    ui.label(
                        RichText::new(format!("✗ {}", comparison.current_scan.failure_count))
                            .size(11.0)
                            .color(colors::ERROR),
                    );
                });
            });
    });

    ui.add_space(8.0);

    // Summary stats
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} total changes", comparison.total_changes)).size(12.0),
        );
        ui.label(RichText::new("•").weak());
        ui.label(
            RichText::new(format!("{} new failures", comparison.new_failures.len()))
                .size(12.0)
                .color(if comparison.new_failures.is_empty() {
                    Color32::GRAY
                } else {
                    colors::ERROR
                }),
        );
        ui.label(RichText::new("•").weak());
        ui.label(
            RichText::new(format!("{} new successes", comparison.new_successes.len()))
                .size(12.0)
                .color(if comparison.new_successes.is_empty() {
                    Color32::GRAY
                } else {
                    colors::SUCCESS
                }),
        );
    });

    ui.add_space(8.0);

    // AI Comparison Analysis Panel
    super::ai::render_comparison_ai_panel(ui, app);

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);

    // Clone the data for the scroll area
    let new_failures = comparison.new_failures.clone();
    let new_successes = comparison.new_successes.clone();
    let output_changes: Vec<_> = comparison
        .status_unchanged
        .iter()
        .filter(|c| c.output_changed)
        .cloned()
        .collect();

    egui::ScrollArea::vertical().show(ui, |ui| {
        // New failures (most important)
        if !new_failures.is_empty() {
            ui.label(
                RichText::new("🔴 New Failures")
                    .size(14.0)
                    .strong()
                    .color(colors::ERROR),
            );
            ui.add_space(4.0);

            for change in &new_failures {
                show_change_card(ui, change, app);
            }

            ui.add_space(12.0);
        }

        // New successes
        if !new_successes.is_empty() {
            ui.label(
                RichText::new("🟢 New Successes")
                    .size(14.0)
                    .strong()
                    .color(colors::SUCCESS),
            );
            ui.add_space(4.0);

            for change in &new_successes {
                show_change_card(ui, change, app);
            }

            ui.add_space(12.0);
        }

        // Unchanged with output changes
        if !output_changes.is_empty() {
            ui.label(
                RichText::new("🔵 Output Changed (Status Same)")
                    .size(14.0)
                    .strong()
                    .color(colors::INFO),
            );
            ui.add_space(4.0);

            for change in &output_changes {
                show_change_card(ui, change, app);
            }
        }
    });
}

fn show_change_card(
    ui: &mut egui::Ui,
    change: &wfdiag_tauri::results_storage::TaskChange,
    app: &mut WfDiagApp,
) {
    let card_id = format!("change_{}", change.task_id);
    let is_expanded = *app.expanded_history_tasks.get(&card_id).unwrap_or(&false);

    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(4.0)
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Expand button
                if ui
                    .button(RichText::new(if is_expanded { "▼" } else { "▶" }).size(10.0))
                    .clicked()
                {
                    app.expanded_history_tasks
                        .insert(card_id.clone(), !is_expanded);
                }

                // Status indicators
                let prev_icon = if change.previous_success {
                    "✓"
                } else {
                    "✗"
                };
                let curr_icon = if change.current_success { "✓" } else { "✗" };
                let prev_color = if change.previous_success {
                    colors::SUCCESS
                } else {
                    colors::ERROR
                };
                let curr_color = if change.current_success {
                    colors::SUCCESS
                } else {
                    colors::ERROR
                };

                ui.label(RichText::new(prev_icon).size(11.0).color(prev_color));
                ui.label(RichText::new("→").size(11.0).weak());
                ui.label(RichText::new(curr_icon).size(11.0).color(curr_color));

                ui.add_space(8.0);

                ui.vertical(|ui| {
                    ui.label(RichText::new(&change.task_name).size(12.0).strong());
                    ui.label(RichText::new(&change.category).size(10.0).weak());
                });
            });

            if is_expanded {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    // Previous output
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width() / 2.0 - 10.0);
                        ui.label(RichText::new("Previous").size(10.0).weak());
                        egui::Frame::new()
                            .fill(Color32::from_gray(30))
                            .corner_radius(2.0)
                            .inner_margin(Margin::same(4))
                            .show(ui, |ui| {
                                render_output_compact(ui, &change.previous_output);
                            });
                    });

                    ui.add_space(8.0);

                    // Current output
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Current").size(10.0).weak());
                        egui::Frame::new()
                            .fill(Color32::from_gray(30))
                            .corner_radius(2.0)
                            .inner_margin(Margin::same(4))
                            .show(ui, |ui| {
                                render_output_compact(ui, &change.current_output);
                            });
                    });
                });
            }
        });

    ui.add_space(4.0);
}

fn truncate_output(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

/// Render output in compact format suitable for comparison view
fn render_output_compact(ui: &mut egui::Ui, output: &str) {
    if output.is_empty() {
        ui.label(RichText::new("(no output)").size(9.0).weak());
        return;
    }

    // Try JSON parsing first
    match serde_json::from_str::<serde_json::Value>(output) {
        Ok(json) => {
            render_json_compact(ui, &json, 0, 8);
        }
        Err(_) => {
            // Fallback to truncated text
            let truncated = truncate_output(output, 500);
            ui.label(RichText::new(truncated).size(9.0).monospace());
        }
    }
}

/// Render JSON in compact format with limited depth
fn render_json_compact(
    ui: &mut egui::Ui,
    value: &serde_json::Value,
    depth: usize,
    max_items: usize,
) {
    match value {
        serde_json::Value::Null => {
            ui.label(RichText::new("null").size(9.0).weak());
        }
        serde_json::Value::Bool(b) => {
            let (text, color) = if *b {
                ("True", colors::SUCCESS)
            } else {
                ("False", Color32::from_rgb(180, 100, 100))
            };
            ui.label(RichText::new(text).size(9.0).color(color));
        }
        serde_json::Value::Number(n) => {
            ui.label(
                RichText::new(n.to_string())
                    .size(9.0)
                    .color(Color32::from_rgb(100, 149, 237)),
            );
        }
        serde_json::Value::String(s) => {
            let display = if s.len() > 50 {
                format!("{}…", &s[..47])
            } else {
                s.clone()
            };
            ui.label(RichText::new(display).size(9.0));
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                ui.label(RichText::new("[]").size(9.0).weak());
            } else if depth > 2 {
                ui.label(
                    RichText::new(format!("[{} items]", arr.len()))
                        .size(9.0)
                        .weak(),
                );
            } else {
                let show_count = arr.len().min(max_items);
                for (i, item) in arr.iter().take(show_count).enumerate() {
                    if i > 0 {
                        ui.add_space(2.0);
                    }
                    render_json_compact(ui, item, depth + 1, max_items);
                }
                if arr.len() > show_count {
                    ui.label(
                        RichText::new(format!("+{} more", arr.len() - show_count))
                            .size(9.0)
                            .weak(),
                    );
                }
            }
        }
        serde_json::Value::Object(obj) => {
            if obj.is_empty() {
                ui.label(RichText::new("{}").size(9.0).weak());
            } else if depth > 2 {
                ui.label(
                    RichText::new(format!("{{{} fields}}", obj.len()))
                        .size(9.0)
                        .weak(),
                );
            } else {
                let keys: Vec<_> = obj.keys().take(max_items).collect();
                egui::Grid::new(
                    ui.id()
                        .with(format!("compact_grid_{}_{}", depth, obj.len())),
                )
                .num_columns(2)
                .spacing([8.0, 2.0])
                .show(ui, |ui| {
                    for key in &keys {
                        if let Some(val) = obj.get(*key) {
                            ui.label(RichText::new(format!("{}:", key)).size(9.0).weak());
                            render_json_compact(ui, val, depth + 1, max_items);
                            ui.end_row();
                        }
                    }
                    if obj.len() > keys.len() {
                        ui.label(
                            RichText::new(format!("+{} more", obj.len() - keys.len()))
                                .size(9.0)
                                .weak(),
                        );
                        ui.end_row();
                    }
                });
            }
        }
    }
}

fn get_task_category(task_id: &str) -> String {
    // Try to get category from task definitions
    let tasks = wfdiag_tauri::diagnostics::get_all_tasks();
    for task in &tasks {
        if task.id == task_id {
            return task.category.clone();
        }
    }
    "Other".to_string()
}

fn get_task_name(task_id: &str) -> String {
    let tasks = wfdiag_tauri::diagnostics::get_all_tasks();
    for task in &tasks {
        if task.id == task_id {
            return task.name.clone();
        }
    }
    task_id.to_string()
}
