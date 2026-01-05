//! Processes Tab - Full htop-like process viewer

use crate::WfDiagApp;
use eframe::egui::{self, RichText};
use super::components;

pub fn show(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    // Header with controls
    components::page_header(ui, "📋 Processes", |ui| {
        // Status badge
        components::live_badge(ui, app.monitoring_state.is_active);

        ui.add_space(components::SPACE_LG);

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

        ui.add_space(components::SPACE_LG);

        // Process count
        ui.label(
            RichText::new(format!("{} processes", app.all_processes.len()))
                .size(12.0)
                .weak(),
        );
    });

    components::section_separator(ui);

    #[cfg(windows)]
    {
        if !app.monitoring_state.is_active || app.all_processes.is_empty() {
            let (title, desc) = if app.monitoring_state.is_active {
                ("Collecting process data...", "Process list will appear shortly")
            } else {
                ("Process Monitoring Inactive", "Click Start to begin monitoring processes")
            };
            components::empty_state(ui, "📋", title, desc);
            return;
        }

        // AI Analysis Panel at the top
        super::ai::render_process_ai_panel(ui, app);
        ui.add_space(components::SPACE_SM);

        // Full process list
        super::process_list::draw_process_list(ui, &mut app.process_list_state, &app.all_processes);
    }

    #[cfg(not(windows))]
    {
        components::empty_state(ui, "📋", "Process List", "Only available on Windows");
    }
}
