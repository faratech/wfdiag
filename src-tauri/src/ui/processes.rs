//! Processes Tab - Full htop-like process viewer

use crate::WfDiagApp;
use eframe::egui::{self, Color32, RichText};

pub fn show(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    // Header with controls
    ui.horizontal(|ui| {
        ui.label(RichText::new("📋 Processes").size(18.0).strong());

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Status badge
            let (status_text, status_color) = if app.monitoring_state.is_active {
                ("● Live", Color32::from_rgb(80, 200, 120))
            } else {
                ("○ Inactive", Color32::GRAY)
            };
            ui.label(RichText::new(status_text).size(12.0).color(status_color));

            ui.add_space(16.0);

            // Start/Stop button
            let button_text = if app.monitoring_state.is_active {
                "⏹ Stop"
            } else {
                "▶ Start"
            };
            if ui.button(RichText::new(button_text).size(12.0)).clicked() {
                if app.monitoring_state.is_active {
                    app.stop_monitoring();
                } else {
                    app.start_monitoring();
                }
            }

            ui.add_space(16.0);

            // Process count
            ui.label(
                RichText::new(format!("{} processes", app.all_processes.len()))
                    .size(12.0)
                    .weak(),
            );
        });
    });

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);

    #[cfg(windows)]
    {
        if !app.monitoring_state.is_active || app.all_processes.is_empty() {
            // Empty state
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.label(RichText::new("📋").size(48.0).weak());
                ui.add_space(16.0);
                ui.label(
                    RichText::new(if app.monitoring_state.is_active {
                        "Collecting process data..."
                    } else {
                        "Process Monitoring Inactive"
                    })
                    .size(18.0)
                    .strong(),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(if app.monitoring_state.is_active {
                        "Process list will appear shortly"
                    } else {
                        "Click Start to begin monitoring processes"
                    })
                    .size(13.0)
                    .weak(),
                );
            });
            return;
        }

        // AI Analysis Panel at the top
        super::ai::render_process_ai_panel(ui, app);
        ui.add_space(8.0);

        // Full process list
        super::process_list::draw_process_list(ui, &mut app.process_list_state, &app.all_processes);
    }

    #[cfg(not(windows))]
    {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.label(RichText::new("📋").size(48.0).weak());
            ui.add_space(16.0);
            ui.label(RichText::new("Process List").size(18.0).strong());
            ui.add_space(8.0);
            ui.label(RichText::new("Only available on Windows").size(13.0).weak());
        });
    }
}
