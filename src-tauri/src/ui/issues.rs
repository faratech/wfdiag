use eframe::egui::{self, Color32, RichText, Margin};
use crate::WfDiagApp;

fn margin_same(v: i8) -> Margin {
    Margin::same(v)
}

pub fn show(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    // Clone the data we need upfront and drop the lock
    let (failed_items, results_empty) = {
        let results = app.results.lock().unwrap();
        let is_empty = results.is_empty();

        let items: Vec<(String, String, Option<String>)> = results.iter()
            .filter(|(_, r)| !r.success)
            .map(|(id, r)| {
                let task = app.available_tasks.iter().find(|t| &t.id == id);
                let name = task.map(|t| t.name.clone()).unwrap_or_else(|| id.clone());
                let category = task.map(|t| t.category.clone()).unwrap_or_else(|| "Unknown".to_string());
                (name, category, r.error.clone())
            })
            .collect();

        (items, is_empty)
    };

    if failed_items.is_empty() {
        // No issues state
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.label(RichText::new("✓").size(48.0).color(Color32::from_rgb(80, 180, 80)));
            ui.add_space(16.0);
            ui.label(RichText::new("No Issues Detected").size(18.0).strong());
            ui.add_space(8.0);
            if results_empty {
                ui.label(RichText::new("Run a scan to check for issues like low disk space,\noutdated drivers, or service problems.").size(13.0).weak());
            } else {
                ui.label(RichText::new("All diagnostics passed successfully.\nYour system is configured correctly.").size(13.0).weak());
            }
        });
        return;
    }

    // Header
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("⚠ {} Issue{} Found",
            failed_items.len(),
            if failed_items.len() == 1 { "" } else { "s" }
        )).size(16.0).strong().color(Color32::from_rgb(220, 140, 60)));
    });

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);

    // Issues list
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (name, category, error) in &failed_items {
            egui::Frame::none()
                .fill(Color32::from_rgba_unmultiplied(220, 80, 80, 15))
                .rounding(4.0)
                .inner_margin(margin_same(12))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 80, 80).linear_multiply(0.3)))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("✗").size(14.0).color(Color32::from_rgb(220, 80, 80)));
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(name).size(13.0).strong());
                                ui.label(RichText::new(format!("({})", category)).size(11.0).weak());
                            });
                            if let Some(err) = error {
                                ui.add_space(4.0);
                                ui.label(RichText::new(err).size(11.0).weak());
                            }
                        });
                    });
                });
            ui.add_space(4.0);
        }
    });
}
