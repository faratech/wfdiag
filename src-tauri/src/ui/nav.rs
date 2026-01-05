use eframe::egui::{self, Color32, RichText, Vec2};
use crate::{WfDiagApp, Tab};

pub fn show(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.add_space(8.0);

        let tabs = [
            (Tab::Diagnostics, "🔍", "Diagnostics"),
            (Tab::Monitoring, "📊", "Monitoring"),
            (Tab::Processes, "📋", "Processes"),
            (Tab::Issues, "⚠", "Issues"),
            (Tab::History, "📜", "History"),
        ];

        for (tab, icon, tooltip) in tabs {
            let is_selected = app.selected_tab == tab;
            let btn_color = if is_selected {
                Color32::from_rgb(60, 120, 200)
            } else {
                Color32::TRANSPARENT
            };

            let text_color = if is_selected {
                Color32::WHITE
            } else {
                Color32::from_gray(180)
            };

            let response = ui.add_sized(
                Vec2::new(40.0, 40.0),
                egui::Button::new(RichText::new(icon).size(18.0).color(text_color))
                    .fill(btn_color)
                    .rounding(4.0)
            );

            if response.on_hover_text(tooltip).clicked() {
                app.selected_tab = tab;
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(8.0);

            // Version at bottom
            ui.label(RichText::new("v2.1.8").size(9.0).weak());
        });
    });
}
