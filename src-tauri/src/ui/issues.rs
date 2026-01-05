use eframe::egui::{self, RichText, Margin};
use crate::WfDiagApp;
use super::{colors, components};

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
        // No issues state - use shared empty state component
        let description = if results_empty {
            "Run a scan to check for issues like low disk space,\noutdated drivers, or service problems."
        } else {
            "All diagnostics passed successfully.\nYour system is configured correctly."
        };
        components::empty_state(ui, "✓", "No Issues Detected", description);
        return;
    }

    // Header
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("⚠ {} Issue{} Found",
            failed_items.len(),
            if failed_items.len() == 1 { "" } else { "s" }
        )).size(16.0).strong().color(colors::WARNING));
    });

    components::section_separator(ui);

    // Issues list
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (name, category, error) in &failed_items {
            components::alert_frame(ui, colors::ERROR, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✗").size(14.0).color(colors::ERROR));
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
