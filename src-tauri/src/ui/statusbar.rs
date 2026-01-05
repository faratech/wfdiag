use eframe::egui::{self, Color32, RichText};
use crate::WfDiagApp;
use super::{colors, components};

pub fn show(app: &WfDiagApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.set_height(22.0);
        ui.spacing_mut().item_spacing.x = 16.0;

        // Status message
        ui.label(RichText::new(&app.status_message).size(11.0));

        ui.separator();

        // Results summary
        let results = app.results.lock().unwrap();
        if !results.is_empty() {
            let passed = results.values().filter(|r| r.success).count();
            let failed = results.len() - passed;

            ui.label(RichText::new(format!("✓ {} Passed", passed))
                .size(11.0)
                .color(colors::SUCCESS));

            if failed > 0 {
                ui.label(RichText::new(format!("✗ {} Failed", failed))
                    .size(11.0)
                    .color(colors::ERROR));
            }

            // Duration
            if let Some(duration) = app.last_scan_duration {
                let duration_str = components::format_duration(duration.as_millis() as u64);
                ui.label(RichText::new(format!("⏱ {}", duration_str)).size(11.0).weak());
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Admin status
            if let Some(ref info) = app.system_info {
                if info.is_admin {
                    ui.label(RichText::new("🛡 Admin")
                        .size(11.0)
                        .color(colors::INFO));
                } else {
                    ui.label(RichText::new("👤 User")
                        .size(11.0)
                        .weak());
                }

                ui.separator();

                // OS info
                ui.label(RichText::new(&info.os_name).size(11.0).weak());
            }
        });
    });
}
