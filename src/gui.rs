use crate::{AppState, diagnostics};
use anyhow::Result;
use eframe::egui;
use egui::{epaint, Margin};
use epaint::StrokeKind;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

pub struct DiagnosticApp {
    state: Arc<Mutex<AppState>>,
    output_dir: PathBuf,
    zip_path: PathBuf,
    task_handle: Option<JoinHandle<Result<()>>>,
    animation_time: f32,
    show_task_details: bool,
    selected_task_index: Option<usize>,
    pulse_animation: f32,
    sparkle_positions: Vec<(f32, f32, f32)>, // x, y, lifetime
}

impl DiagnosticApp {
    pub fn new(state: Arc<Mutex<AppState>>, output_dir: PathBuf, zip_path: PathBuf) -> Self {
        // Initialize sparkle positions
        let mut sparkles = Vec::new();
        for _ in 0..20 {
            sparkles.push((
                rand::random::<f32>() * 1000.0,
                rand::random::<f32>() * 700.0,
                rand::random::<f32>(),
            ));
        }
        
        Self {
            state,
            output_dir,
            zip_path,
            task_handle: None,
            animation_time: 0.0,
            show_task_details: false,
            selected_task_index: None,
            pulse_animation: 0.0,
            sparkle_positions: sparkles,
        }
    }

    fn start_diagnostics(&mut self) {
        let state = Arc::clone(&self.state);
        let output_dir = self.output_dir.clone();
        let zip_path = self.zip_path.clone();

        // Update state to indicate start
        {
            let mut app_state = self.state.lock().unwrap();
            app_state.is_running = true;
            app_state.status_text = "Initializing diagnostics...".to_string();
            app_state.progress = 0.0;
            app_state.current_output = String::new();
            app_state.diagnostics_started = true;
        }

        // Start background task
        self.task_handle = Some(tokio::spawn(async move {
            diagnostics::run_selected_diagnostics(state, output_dir, zip_path).await
        }));
    }

    fn fluent_colors() -> FluentColors {
        FluentColors {
            background: egui::Color32::from_rgb(18, 18, 18),
            surface: egui::Color32::from_rgb(32, 32, 32),
            surface_light: egui::Color32::from_rgb(42, 42, 42),
            accent: egui::Color32::from_rgb(0, 120, 215),
            accent_light: egui::Color32::from_rgb(40, 160, 255),
            accent_dark: egui::Color32::from_rgb(0, 90, 160),
            text_primary: egui::Color32::from_rgb(255, 255, 255),
            text_secondary: egui::Color32::from_rgb(180, 180, 180),
            text_tertiary: egui::Color32::from_rgb(120, 120, 120),
            success: egui::Color32::from_rgb(16, 124, 16),
            success_light: egui::Color32::from_rgb(48, 208, 48),
            warning: egui::Color32::from_rgb(255, 185, 0),
            warning_light: egui::Color32::from_rgb(255, 210, 80),
            error: egui::Color32::from_rgb(232, 17, 35),
            glass: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 5),
        }
    }

    fn draw_header(&self, ui: &mut egui::Ui, colors: &FluentColors) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            
            // Logo and title
            ui.label(
                egui::RichText::new("⚡")
                    .size(32.0)
                    .color(colors.accent_light)
            );
            
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Windows Diagnostic Suite")
                        .size(24.0)
                        .strong()
                        .color(colors.text_primary)
                );
                ui.label(
                    egui::RichText::new("Professional System Analysis Tool")
                        .size(12.0)
                        .color(colors.text_secondary)
                );
            });
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Admin status badge
                let app_state = self.state.lock().unwrap();
                let (icon, text, color) = if app_state.is_admin {
                    ("🛡️", "Administrator", colors.success_light)
                } else {
                    ("👤", "Standard User", colors.warning_light)
                };
                
                // Glowing badge
                let badge_rect = ui.available_rect_before_wrap();
                let badge_size = egui::vec2(140.0, 32.0);
                let badge_rect = egui::Rect::from_center_size(
                    badge_rect.right_center() - egui::vec2(badge_size.x / 2.0 + 10.0, 0.0),
                    badge_size
                );
                
                // Glow effect
                for i in 0..3 {
                    let glow_alpha = 20 - i * 5;
                    let glow_size = i as f32 * 4.0;
                    ui.painter().rect(
                        badge_rect.expand(glow_size),
                        12.0,
                        egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), glow_alpha),
                        egui::Stroke::NONE,
                        StrokeKind::Middle,
                    );
                }
                
                ui.painter().rect(
                    badge_rect,
                    8.0,
                    color.linear_multiply(0.2),
                    egui::Stroke::new(1.0, color),
                    StrokeKind::Middle,
                );
                
                ui.allocate_ui_at_rect(badge_rect, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{} {}", icon, text))
                                .size(14.0)
                                .color(color)
                        );
                    });
                });
            });
        });
    }

    fn draw_main_content(&mut self, ui: &mut egui::Ui, colors: &FluentColors) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            
            // Left panel - Task selection and status
            ui.vertical(|ui| {
                ui.set_width(400.0);
                self.draw_task_panel(ui, colors);
            });
            
            // Center panel - Progress and visualization
            ui.vertical(|ui| {
                ui.set_width(300.0);
                self.draw_progress_panel(ui, colors);
            });
            
            // Right panel - Live output and results
            ui.vertical(|ui| {
                ui.set_min_width(250.0);
                self.draw_output_panel(ui, colors);
            });
        });
    }

    fn draw_task_panel(&mut self, ui: &mut egui::Ui, colors: &FluentColors) {
        // Panel header
        self.draw_panel_header(ui, "🎯 Diagnostic Tasks", colors);
        
        ui.add_space(10.0);
        
        // Quick actions
        ui.horizontal(|ui| {
            if self.draw_action_button(ui, "✓ All", colors.success, colors).clicked() {
                let mut app_state = self.state.lock().unwrap();
                app_state.selected_tasks.fill(true);
            }
            if self.draw_action_button(ui, "✗ None", colors.error, colors).clicked() {
                let mut app_state = self.state.lock().unwrap();
                app_state.selected_tasks.fill(false);
            }
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let app_state = self.state.lock().unwrap();
                let selected_count = app_state.selected_tasks.iter().filter(|&&x| x).count();
                ui.label(
                    egui::RichText::new(format!("{} selected", selected_count))
                        .size(12.0)
                        .color(colors.text_secondary)
                );
            });
        });
        
        ui.add_space(10.0);
        
        // Task list with modern cards
        egui::ScrollArea::vertical()
            .max_height(350.0)
            .show(ui, |ui| {
                let (is_admin, selected_tasks) = {
                    let app_state = self.state.lock().unwrap();
                    (app_state.is_admin, app_state.selected_tasks.clone())
                };
                let tasks = diagnostics::get_filtered_tasks(is_admin);
                
                for (i, task) in tasks.iter().enumerate() {
                    let mut selected = selected_tasks.get(i).copied().unwrap_or(false);
                    
                    // Modern task card
                    let response = ui.allocate_response(
                        egui::vec2(ui.available_width(), 60.0),
                        egui::Sense::click()
                    );
                    
                    let is_hovered = response.hovered();
                    let bg_color = if selected {
                        colors.accent.linear_multiply(0.15)
                    } else if is_hovered {
                        colors.surface_light
                    } else {
                        colors.surface
                    };
                    
                    // Card background with gradient
                    ui.painter().rect(
                        response.rect,
                        8.0,
                        bg_color,
                        egui::Stroke::new(
                            1.0,
                            if selected { colors.accent } else { colors.glass }
                        ),
                        StrokeKind::Middle,
                    );
                    
                    // Hover glow effect
                    if is_hovered {
                        ui.painter().rect(
                            response.rect.expand(2.0),
                            10.0,
                            egui::Color32::from_rgba_unmultiplied(
                                colors.accent_light.r(),
                                colors.accent_light.g(),
                                colors.accent_light.b(),
                                10
                            ),
                            egui::Stroke::NONE,
                            StrokeKind::Middle,
                        );
                    }
                    
                    ui.allocate_ui_at_rect(response.rect.shrink(15.0), |ui| {
                        ui.horizontal(|ui| {
                            // Animated checkbox
                            let checkbox_size = 20.0;
                            let checkbox_rect = ui.allocate_space(egui::vec2(checkbox_size, checkbox_size)).1;
                            
                            ui.painter().rect(
                                checkbox_rect,
                                4.0,
                                if selected { colors.accent } else { colors.surface_light },
                                egui::Stroke::new(2.0, colors.accent_light),
                                StrokeKind::Middle,
                            );
                            
                            if selected {
                                ui.painter().text(
                                    checkbox_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "✓",
                                    egui::FontId::proportional(16.0),
                                    colors.text_primary,
                                );
                            }
                            
                            ui.add_space(10.0);
                            
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(task.name)
                                        .size(14.0)
                                        .strong()
                                        .color(colors.text_primary)
                                );
                                
                                let icon = self.get_task_icon(task.name);
                                let desc = self.get_task_description(task.name);
                                ui.label(
                                    egui::RichText::new(format!("{} {}", icon, desc))
                                        .size(11.0)
                                        .color(colors.text_secondary)
                                );
                            });
                            
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if task.admin_required && !is_admin {
                                    ui.label(
                                        egui::RichText::new("🔒")
                                            .size(16.0)
                                            .color(colors.warning)
                                    );
                                }
                            });
                        });
                    });
                    
                    if response.clicked() {
                        let mut app_state = self.state.lock().unwrap();
                        if let Some(sel) = app_state.selected_tasks.get_mut(i) {
                            *sel = !*sel;
                        }
                    }
                    
                    ui.add_space(5.0);
                }
            });
        
        ui.add_space(15.0);
        
        // Start button with animation
        let app_state = self.state.lock().unwrap();
        let selected_count = app_state.selected_tasks.iter().filter(|&&x| x).count();
        let can_start = selected_count > 0 && !app_state.is_running && !app_state.diagnostics_started;
        
        if can_start {
            let button_rect = ui.available_rect_before_wrap();
            let button_height = 50.0;
            let button_rect = egui::Rect::from_x_y_ranges(
                button_rect.x_range(),
                button_rect.top()..=button_rect.top() + button_height,
            );
            
            let response = ui.allocate_rect(button_rect, egui::Sense::click());
            
            // Animated gradient background
            let gradient_offset = (self.animation_time * 0.5).sin() * 0.5 + 0.5;
            let gradient_color1 = colors.accent;
            let gradient_color2 = colors.accent_light;
            
            // Button glow
            if response.hovered() {
                for i in 0..3 {
                    let glow_alpha = 15 - i * 5;
                    ui.painter().rect(
                        button_rect.expand(i as f32 * 2.0),
                        10.0,
                        egui::Color32::from_rgba_unmultiplied(
                            gradient_color2.r(),
                            gradient_color2.g(),
                            gradient_color2.b(),
                            glow_alpha
                        ),
                        egui::Stroke::NONE,
                        StrokeKind::Middle,
                    );
                }
            }
            
            ui.painter().rect(
                button_rect,
                10.0,
                gradient_color1.lerp_to_gamma(gradient_color2, gradient_offset),
                egui::Stroke::NONE,
                StrokeKind::Middle,
            );
            
            ui.painter().text(
                button_rect.center(),
                egui::Align2::CENTER_CENTER,
                "🚀 Start Analysis",
                egui::FontId::proportional(18.0),
                colors.text_primary,
            );
            
            if response.clicked() {
                drop(app_state);
                self.start_diagnostics();
            }
        }
    }

    fn draw_progress_panel(&mut self, ui: &mut egui::Ui, colors: &FluentColors) {
        self.draw_panel_header(ui, "📊 Progress Monitor", colors);
        
        ui.add_space(20.0);
        
        let app_state = self.state.lock().unwrap();
        let progress = app_state.progress;
        
        // Circular progress with multiple rings
        let center = ui.available_rect_before_wrap().center();
        let radius = 80.0;
        
        // Outer decorative ring
        ui.painter().circle_stroke(
            center,
            radius + 20.0,
            egui::Stroke::new(1.0, colors.glass),
        );
        
        // Background ring
        ui.painter().circle_stroke(
            center,
            radius,
            egui::Stroke::new(12.0, colors.surface_light),
        );
        
        // Progress ring with gradient effect
        if progress > 0.0 {
            let start_angle = -std::f32::consts::PI / 2.0;
            let sweep = 2.0 * std::f32::consts::PI * progress;
            
            // Draw progress arc as small segments for gradient effect
            let segments = 60;
            for i in 0..segments {
                let t = i as f32 / segments as f32;
                if t <= progress {
                    let angle = start_angle + sweep * t;
                    let color = colors.accent.lerp_to_gamma(colors.accent_light, t);
                    let point = center + radius * egui::vec2(angle.cos(), angle.sin());
                    
                    ui.painter().circle_filled(point, 6.0, color);
                }
            }
        }
        
        // Center content
        ui.painter().text(
            center,
            egui::Align2::CENTER_CENTER,
            format!("{:.0}%", progress * 100.0),
            egui::FontId::proportional(36.0),
            colors.text_primary,
        );
        
        ui.painter().text(
            center + egui::vec2(0.0, 30.0),
            egui::Align2::CENTER_CENTER,
            "Complete",
            egui::FontId::proportional(12.0),
            colors.text_secondary,
        );
        
        ui.allocate_space(egui::vec2(0.0, 180.0));
        ui.add_space(20.0);
        
        // Status information
        if !app_state.status_text.is_empty() {
            ui.label(
                egui::RichText::new(&app_state.status_text)
                    .size(14.0)
                    .color(colors.text_primary)
            );
        }
        
        if !app_state.current_task.is_empty() {
            ui.label(
                egui::RichText::new(format!("▶ {}", app_state.current_task))
                    .size(12.0)
                    .color(colors.accent_light)
            );
        }
        
        if app_state.total_tasks > 0 {
            ui.add_space(10.0);
            
            // Task progress bar
            let tasks_progress = app_state.tasks_completed as f32 / app_state.total_tasks as f32;
            let progress_rect = ui.available_rect_before_wrap();
            let progress_height = 8.0;
            let progress_rect = egui::Rect::from_x_y_ranges(
                progress_rect.x_range(),
                progress_rect.top()..=progress_rect.top() + progress_height,
            );
            
            ui.painter().rect(
                progress_rect,
                4.0,
                colors.surface_light,
                egui::Stroke::NONE,
                StrokeKind::Middle,
            );
            
            let filled_rect = egui::Rect::from_min_size(
                progress_rect.min,
                egui::vec2(progress_rect.width() * tasks_progress, progress_height),
            );
            
            ui.painter().rect(
                filled_rect,
                4.0,
                colors.success_light,
                egui::Stroke::NONE,
                StrokeKind::Middle,
            );
            
            ui.allocate_space(egui::vec2(0.0, progress_height + 5.0));
            
            ui.label(
                egui::RichText::new(format!("{}/{} tasks completed", 
                    app_state.tasks_completed, app_state.total_tasks))
                    .size(11.0)
                    .color(colors.text_secondary)
            );
        }
        
        // Quick stats
        if app_state.is_running {
            ui.add_space(20.0);
            ui.separator();
            ui.add_space(10.0);
            
            ui.label(
                egui::RichText::new("⚡ Live Statistics")
                    .size(14.0)
                    .strong()
                    .color(colors.text_primary)
            );
            
            ui.add_space(10.0);
            
            // Animated stats
            self.draw_stat_card(ui, "CPU Usage", "~", &format!("{}%", (self.animation_time * 10.0 % 100.0) as i32), colors.accent_light, colors);
            self.draw_stat_card(ui, "Memory", "~", "Scanning...", colors.success_light, colors);
            self.draw_stat_card(ui, "Disk I/O", "~", "Active", colors.warning_light, colors);
        }
    }

    fn draw_output_panel(&mut self, ui: &mut egui::Ui, colors: &FluentColors) {
        self.draw_panel_header(ui, "📋 Live Output", colors);
        
        ui.add_space(10.0);
        
        let app_state = self.state.lock().unwrap();
        
        // Terminal-style output window
        let output_rect = ui.available_rect_before_wrap();
        let output_height = 300.0;
        let output_rect = egui::Rect::from_x_y_ranges(
            output_rect.x_range(),
            output_rect.top()..=output_rect.top() + output_height,
        );
        
        // Terminal background
        ui.painter().rect(
            output_rect,
            8.0,
            egui::Color32::from_rgb(12, 12, 12),
            egui::Stroke::new(1.0, colors.surface_light),
            StrokeKind::Middle,
        );
        
        // Terminal header
        let header_rect = egui::Rect::from_x_y_ranges(
            output_rect.x_range(),
            output_rect.top()..=output_rect.top() + 25.0,
        );
        
        ui.painter().rect(
            header_rect,
            egui::CornerRadius {
                nw: 8,
                ne: 8,
                sw: 0,
                se: 0,
            },
            colors.surface,
            egui::Stroke::NONE,
            StrokeKind::Middle,
        );
        
        // Terminal buttons
        let button_y = header_rect.center().y;
        let button_x_start = header_rect.left() + 10.0;
        for (i, color) in [(colors.error, "×"), (colors.warning, "−"), (colors.success, "□")].iter().enumerate() {
            let button_pos = egui::pos2(button_x_start + i as f32 * 20.0, button_y);
            ui.painter().circle_filled(button_pos, 6.0, color.0);
        }
        
        // Output content
        let content_rect = egui::Rect::from_x_y_ranges(
            output_rect.x_range().shrink(10.0),
            (output_rect.top() + 30.0)..=(output_rect.bottom() - 10.0),
        );
        
        egui::ScrollArea::vertical()
            .id_salt("output_scroll")
            .show_viewport(ui, |ui, _viewport| {
                ui.allocate_ui_at_rect(content_rect, |ui| {
                    if !app_state.current_output.is_empty() {
                        ui.label(
                            egui::RichText::new(&app_state.current_output)
                                .size(11.0)
                                .family(egui::FontFamily::Monospace)
                                .color(colors.success_light)
                        );
                    } else if app_state.is_running {
                        ui.label(
                            egui::RichText::new("Waiting for output...")
                                .size(11.0)
                                .family(egui::FontFamily::Monospace)
                                .color(colors.text_tertiary)
                        );
                    }
                });
            });
        
        ui.allocate_space(egui::vec2(0.0, output_height + 20.0));
        
        // Action buttons
        if app_state.progress >= 1.0 && !app_state.is_running {
            ui.separator();
            ui.add_space(10.0);
            
            ui.label(
                egui::RichText::new("✅ Analysis Complete!")
                    .size(16.0)
                    .strong()
                    .color(colors.success_light)
            );
            
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                if self.draw_action_button(ui, "📁 Open Results", colors.accent, colors).clicked() {
                    let _ = std::process::Command::new("explorer")
                        .arg(&self.output_dir)
                        .spawn();
                }
                
                if self.draw_action_button(ui, "📦 Export ZIP", colors.success, colors).clicked() {
                    let _ = std::process::Command::new("explorer")
                        .arg(&self.zip_path)
                        .spawn();
                }
            });
            
            ui.add_space(10.0);
            
            if self.draw_action_button(ui, "🔄 New Analysis", colors.warning, colors).clicked() {
                let mut app_state = self.state.lock().unwrap();
                app_state.diagnostics_started = false;
                app_state.is_running = false;
                app_state.progress = 0.0;
                app_state.current_task = String::new();
                app_state.current_output = String::new();
                app_state.tasks_completed = 0;
            }
        }
    }

    fn draw_panel_header(&self, ui: &mut egui::Ui, title: &str, colors: &FluentColors) {
        let header_rect = ui.available_rect_before_wrap();
        let header_height = 35.0;
        let header_rect = egui::Rect::from_x_y_ranges(
            header_rect.x_range(),
            header_rect.top()..=header_rect.top() + header_height,
        );
        
        // Glass effect header
        ui.painter().rect(
            header_rect,
            8.0,
            colors.glass,
            egui::Stroke::new(1.0, colors.glass),
            StrokeKind::Middle,
        );
        
        ui.allocate_ui_at_rect(header_rect, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add_space(15.0);
                ui.label(
                    egui::RichText::new(title)
                        .size(16.0)
                        .strong()
                        .color(colors.text_primary)
                );
            });
        });
        
        ui.allocate_space(egui::vec2(0.0, header_height));
    }

    fn draw_action_button(&self, ui: &mut egui::Ui, text: &str, color: egui::Color32, colors: &FluentColors) -> egui::Response {
        let (response, painter) = ui.allocate_painter(
            egui::vec2(120.0, 32.0),
            egui::Sense::click()
        );
        
        let bg_color = if response.hovered() {
            color.linear_multiply(0.3)
        } else {
            color.linear_multiply(0.2)
        };
        
        painter.rect(
            response.rect,
            6.0,
            bg_color,
            egui::Stroke::new(1.0, color),
            StrokeKind::Middle,
        );
        
        painter.text(
            response.rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(13.0),
            colors.text_primary,
        );
        
        response
    }

    fn draw_stat_card(&self, ui: &mut egui::Ui, label: &str, icon: &str, value: &str, color: egui::Color32, colors: &FluentColors) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(icon)
                    .size(16.0)
                    .color(color)
            );
            
            ui.label(
                egui::RichText::new(label)
                    .size(11.0)
                    .color(colors.text_secondary)
            );
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(value)
                        .size(12.0)
                        .strong()
                        .color(color)
                );
            });
        });
    }

    fn get_task_icon(&self, task_name: &str) -> &'static str {
        match task_name {
            "Computer System" => "💻",
            "Operating System" => "🖥️",
            "BIOS" => "🔧",
            "BaseBoard" => "🎛️",
            "Processor" => "🎯",
            "Physical Memory" => "🧠",
            "Network Adapter" => "🌐",
            "Disk Drive" => "💾",
            "DXDiag" => "🎮",
            "System Services" => "⚙️",
            "Processes" => "📊",
            "Event Logs" => "📝",
            _ => "📋"
        }
    }

    fn get_task_description(&self, task_name: &str) -> &'static str {
        match task_name {
            "Computer System" => "Hardware and system information",
            "Operating System" => "Windows version and configuration",
            "BIOS" => "Firmware and boot settings",
            "BaseBoard" => "Motherboard specifications",
            "Processor" => "CPU details and capabilities",
            "Physical Memory" => "RAM configuration and usage",
            "Network Adapter" => "Network interfaces and settings",
            "Disk Drive" => "Storage devices and partitions",
            "DXDiag" => "DirectX and graphics diagnostics",
            "System Services" => "Windows services status",
            "Processes" => "Running applications and tasks",
            "Event Logs" => "System and application logs",
            _ => "System diagnostic information"
        }
    }
}

struct FluentColors {
    background: egui::Color32,
    surface: egui::Color32,
    surface_light: egui::Color32,
    accent: egui::Color32,
    accent_light: egui::Color32,
    accent_dark: egui::Color32,
    text_primary: egui::Color32,
    text_secondary: egui::Color32,
    text_tertiary: egui::Color32,
    success: egui::Color32,
    success_light: egui::Color32,
    warning: egui::Color32,
    warning_light: egui::Color32,
    error: egui::Color32,
    glass: egui::Color32,
}

impl eframe::App for DiagnosticApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let colors = Self::fluent_colors();
        self.animation_time += ctx.input(|i| i.stable_dt);
        self.pulse_animation = (self.animation_time * 2.0).sin() * 0.5 + 0.5;
        
        // Update sparkles
        for sparkle in &mut self.sparkle_positions {
            sparkle.2 -= ctx.input(|i| i.stable_dt) * 0.3;
            if sparkle.2 <= 0.0 {
                sparkle.0 = rand::random::<f32>() * 1000.0;
                sparkle.1 = rand::random::<f32>() * 700.0;
                sparkle.2 = 1.0;
            }
        }
        
        // Set dark theme with custom style
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.window_margin = Margin::same(20);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.visuals.dark_mode = true;
        style.visuals.widgets.noninteractive.bg_fill = colors.surface;
        style.visuals.widgets.inactive.bg_fill = colors.surface_light;
        style.visuals.widgets.hovered.bg_fill = colors.accent.linear_multiply(0.2);
        style.visuals.widgets.active.bg_fill = colors.accent.linear_multiply(0.3);
        style.visuals.selection.bg_fill = colors.accent;
        style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, colors.glass);
        ctx.set_style(style);
        
        egui::CentralPanel::default().show(ctx, |ui| {
            // Background with gradient
            let rect = ui.clip_rect();
            let painter = ui.painter();
            
            // Dark gradient background
            painter.rect_filled(rect, 0.0, colors.background);
            
            // Animated sparkles
            for sparkle in &self.sparkle_positions {
                if sparkle.2 > 0.0 {
                    let alpha = (sparkle.2 * 20.0) as u8;
                    painter.circle_filled(
                        egui::pos2(sparkle.0, sparkle.1),
                        2.0 * sparkle.2,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, alpha),
                    );
                }
            }
            
            // Main layout with padding
            ui.add_space(20.0);
            
            ui.vertical(|ui| {
                // Header
                self.draw_header(ui, &colors);
                
                ui.add_space(20.0);
                ui.separator();
                ui.add_space(20.0);
                
                // Main content
                self.draw_main_content(ui, &colors);
            });
        });

        // Clean up finished task handles
        if let Some(handle) = &self.task_handle {
            if handle.is_finished() {
                self.task_handle = None;
            }
        }

        // Always request repaint for animations
        ctx.request_repaint();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Clean up any running tasks
        if let Some(handle) = &self.task_handle {
            handle.abort();
        }
    }
}

// Helper function for random numbers (simple implementation)
mod rand {
    pub fn random<T>() -> T 
    where 
        T: From<f32> 
    {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        T::from((time % 1000) as f32 / 1000.0)
    }
}