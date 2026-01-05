use eframe::egui::{self, Color32, RichText, Vec2};
use crate::{WfDiagApp, ScanState};
use super::colors;

pub fn show(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.set_height(32.0);
        ui.spacing_mut().item_spacing.x = 4.0;

        // Quick Scan button
        let quick_enabled = app.scan_state != ScanState::Running;
        if ui.add_enabled(
            quick_enabled,
            egui::Button::new(RichText::new("▶ Quick").size(12.0))
                .min_size(Vec2::new(70.0, 24.0))
        ).on_hover_text("Run essential diagnostics (~30 seconds)")
            .clicked()
        {
            app.start_scan(true);
        }

        // Full Scan button
        if ui.add_enabled(
            quick_enabled,
            egui::Button::new(RichText::new("▶ Full").size(12.0))
                .min_size(Vec2::new(60.0, 24.0))
        ).on_hover_text("Run all diagnostics (2-5 minutes)")
            .clicked()
        {
            app.start_scan(false);
        }

        // Stop button - always show but disable when not running
        if ui.add_enabled(
            app.scan_state == ScanState::Running,
            egui::Button::new(RichText::new("⏹ Stop").size(12.0).color(
                if app.scan_state == ScanState::Running {
                    colors::ERROR
                } else {
                    Color32::GRAY
                }
            )).min_size(Vec2::new(60.0, 24.0))
        ).clicked() && app.scan_state == ScanState::Running {
            app.scan_state = ScanState::Idle;
            app.status_message = "Scan stopped".to_string();
        }

        ui.separator();

        // Progress bar - fixed width, always allocated
        ui.allocate_ui(Vec2::new(160.0, 24.0), |ui| {
            if app.scan_state == ScanState::Running {
                let progress = app.progress / 100.0;
                ui.add(egui::ProgressBar::new(progress).show_percentage());
            }
        });

        // Current task label - fixed width
        ui.allocate_ui(Vec2::new(120.0, 24.0), |ui| {
            if app.scan_state == ScanState::Running && !app.current_task.is_empty() {
                let display = if app.current_task.len() > 18 {
                    format!("{}...", &app.current_task[..15])
                } else {
                    app.current_task.clone()
                };
                ui.label(RichText::new(display).size(11.0).weak());
            }
        });

        // Flexible spacer - push remaining buttons to the right
        let available = ui.available_width();
        if available > 280.0 {
            ui.add_space(available - 280.0);
        }

        // Right side buttons
        let has_results = !app.results.lock().unwrap().is_empty();

        // Export button
        let export_clicked = ui.add_enabled(
            has_results,
            egui::Button::new(RichText::new("💾 Export").size(12.0))
                .min_size(Vec2::new(70.0, 24.0))
        ).on_hover_text("Export results to JSON file")
            .clicked();

        if export_clicked {
            app.status_message = "Exporting...".to_string();

            match app.export_results() {
                Some(json_data) => {
                    // Save to Documents folder or fallback to current dir
                    let filename = format!("wfdiag-export-{}.json",
                        chrono::Local::now().format("%Y%m%d-%H%M%S"));

                    let export_path = dirs::document_dir()
                        .or_else(dirs::home_dir)
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join(&filename);

                    match std::fs::write(&export_path, &json_data) {
                        Ok(_) => {
                            app.status_message = format!("Exported to {}", export_path.display());
                        }
                        Err(e) => {
                            app.status_message = format!("Export failed: {}", e);
                        }
                    }
                }
                None => {
                    app.status_message = "No results to export".to_string();
                }
            }
        }

        // Copy button
        if ui.add_enabled(
            has_results,
            egui::Button::new(RichText::new("📋 Copy").size(12.0))
                .min_size(Vec2::new(60.0, 24.0))
        ).on_hover_text("Copy results to clipboard")
            .clicked()
        {
            app.copy_results_to_clipboard();
            app.status_message = "Copied to clipboard".to_string();
        }

        // Clear button
        if ui.add_enabled(
            has_results,
            egui::Button::new(RichText::new("Clear").size(12.0))
                .min_size(Vec2::new(50.0, 24.0))
        ).on_hover_text("Clear results")
            .clicked()
        {
            app.clear_results();
        }

        ui.separator();

        // About button
        if ui.add(
            egui::Button::new(RichText::new("ℹ").size(14.0))
                .min_size(Vec2::new(28.0, 24.0))
        ).on_hover_text("About")
            .clicked()
        {
            app.show_about = true;
        }

        // Settings button
        if ui.add(
            egui::Button::new(RichText::new("⚙").size(14.0))
                .min_size(Vec2::new(28.0, 24.0))
        ).on_hover_text("Settings")
            .clicked()
        {
            app.show_settings = true;
        }
    });
}
