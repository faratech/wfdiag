use eframe::egui::{self, Color32, RichText};
use crate::{AiProvider, WfDiagApp, Theme};
use super::ai;

pub fn show(app: &mut WfDiagApp, ctx: &egui::Context) {
    let mut open = app.show_settings;
    let mut close_clicked = false;

    egui::Window::new("Settings")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .min_width(450.0)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;

            // Appearance section
            ui.label(RichText::new("Appearance").size(14.0).strong());
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Theme:");
                ui.add_space(8.0);

                let themes = [
                    (Theme::System, "System"),
                    (Theme::Light, "Light"),
                    (Theme::Dark, "Dark"),
                ];

                for (theme, label) in themes {
                    if ui.selectable_label(app.settings.theme == theme, label).clicked() {
                        app.settings.theme = theme;
                    }
                }
            });

            ui.add_space(16.0);

            // AI section
            ui.label(RichText::new("AI Integration").size(14.0).strong());
            ui.separator();

            ui.checkbox(&mut app.settings.ai_enabled, "Enable AI-powered analysis");

            if app.settings.ai_enabled {
                ui.add_space(8.0);

                // AI Provider selection
                ui.horizontal(|ui| {
                    ui.label("Provider:");
                    ui.add_space(8.0);

                    let providers = [
                        (AiProvider::Auto, "Auto"),
                        (AiProvider::PhiSilica, "Phi Silica (Local)"),
                        (AiProvider::OpenAI, "OpenAI (Cloud)"),
                    ];

                    for (provider, label) in providers {
                        if ui.selectable_label(app.settings.ai_provider == provider, label).clicked() {
                            app.settings.ai_provider = provider;
                        }
                    }
                });

                ui.add_space(4.0);

                // Phi Silica status
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Phi Silica:").size(11.0).weak());
                    if app.ai_status_checking {
                        ui.spinner();
                        ui.label(RichText::new("Checking...").size(11.0).weak());
                    } else if let Some(ref status) = app.ai_phi_silica_status {
                        if status.available {
                            ui.label(RichText::new("✓ Available").size(11.0).color(Color32::from_rgb(80, 180, 80)));
                        } else {
                            ui.label(RichText::new("✗ Not available").size(11.0).color(Color32::from_rgb(180, 100, 100)));
                        }
                        if let Some(ref msg) = Some(&status.message) {
                            if !msg.is_empty() {
                                ui.label(RichText::new(format!("({})", msg)).size(10.0).weak());
                            }
                        }
                    } else {
                        if ui.small_button("Check").clicked() {
                            ai::start_phi_silica_check(app);
                        }
                    }
                });

                ui.add_space(8.0);

                // OpenAI API Key
                ui.horizontal(|ui| {
                    ui.label("OpenAI API Key:");
                    let mut key = app.settings.openai_api_key.clone().unwrap_or_default();
                    if ui.add(egui::TextEdit::singleline(&mut key)
                        .password(true)
                        .desired_width(250.0))
                        .changed()
                    {
                        app.settings.openai_api_key = if key.is_empty() { None } else { Some(key) };
                    }
                });

                let has_key = app.settings.openai_api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false);
                if has_key {
                    ui.label(RichText::new("✓ API key configured").size(11.0).color(Color32::from_rgb(80, 180, 80)));
                } else {
                    ui.label(RichText::new("Get an API key from platform.openai.com").size(11.0).weak());
                }

                ui.add_space(4.0);

                // Current provider status
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Active:").size(11.0).weak());
                    ai::render_ai_status_badge(ui, app);
                });
            }

            ui.add_space(16.0);

            // Quick Scan section
            ui.label(RichText::new("Quick Scan Tasks").size(14.0).strong());
            ui.separator();

            ui.label(RichText::new("Select which diagnostics to include in Quick Scan:")
                .size(11.0)
                .weak());

            ui.add_space(4.0);

            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                let task_ids: Vec<String> = app.available_tasks.iter().map(|t| t.id.clone()).collect();

                for task in &app.available_tasks {
                    let mut included = app.settings.quick_scan_tasks.contains(&task.id);
                    if ui.checkbox(&mut included, &task.name).changed() {
                        if included {
                            if !app.settings.quick_scan_tasks.contains(&task.id) {
                                app.settings.quick_scan_tasks.push(task.id.clone());
                            }
                        } else {
                            app.settings.quick_scan_tasks.retain(|id| id != &task.id);
                        }
                    }
                }
            });

            ui.add_space(16.0);

            ui.horizontal(|ui| {
                if ui.button("Reset to Defaults").clicked() {
                    app.settings = crate::Settings::default();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close_clicked = true;
                    }
                });
            });
        });

    app.show_settings = open && !close_clicked;
}
