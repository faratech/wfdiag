//! System Monitoring UI - Real-time system performance monitoring

use crate::WfDiagApp;
use eframe::egui::{self, Color32, Margin, RichText, Stroke, Vec2};
use std::collections::VecDeque;
use super::{colors, components};

/// Maximum number of data points to keep in history
const MAX_HISTORY: usize = 60;

/// System monitoring state
pub struct MonitoringState {
    pub is_active: bool,
    #[cfg(windows)]
    pub stats: Option<wfdiag_tauri::native_monitor::SystemStats>,
    #[cfg(not(windows))]
    pub stats: Option<()>,
    pub cpu_history: VecDeque<f32>,
    pub memory_history: VecDeque<f32>,
    pub network_up_history: VecDeque<f64>,
    pub network_down_history: VecDeque<f64>,
    pub show_connections: bool,
}

impl Default for MonitoringState {
    fn default() -> Self {
        Self {
            is_active: false,
            stats: None,
            cpu_history: VecDeque::with_capacity(MAX_HISTORY),
            memory_history: VecDeque::with_capacity(MAX_HISTORY),
            network_up_history: VecDeque::with_capacity(MAX_HISTORY),
            network_down_history: VecDeque::with_capacity(MAX_HISTORY),
            show_connections: false,
        }
    }
}

impl MonitoringState {
    #[cfg(windows)]
    pub fn update_stats(&mut self, stats: wfdiag_tauri::native_monitor::SystemStats) {
        // Update history
        if self.cpu_history.len() >= MAX_HISTORY {
            self.cpu_history.pop_front();
        }
        self.cpu_history.push_back(stats.cpu_utilization);

        if self.memory_history.len() >= MAX_HISTORY {
            self.memory_history.pop_front();
        }
        self.memory_history.push_back(stats.memory_utilization);

        if self.network_up_history.len() >= MAX_HISTORY {
            self.network_up_history.pop_front();
        }
        self.network_up_history.push_back(stats.network_upload_kb);

        if self.network_down_history.len() >= MAX_HISTORY {
            self.network_down_history.pop_front();
        }
        self.network_down_history
            .push_back(stats.network_download_kb);

        self.stats = Some(stats);
    }

    #[cfg(not(windows))]
    pub fn update_stats(&mut self, _stats: ()) {
        // No-op on non-Windows
    }

    pub fn clear(&mut self) {
        self.stats = None;
        self.cpu_history.clear();
        self.memory_history.clear();
        self.network_up_history.clear();
        self.network_down_history.clear();
        self.show_connections = false;
    }
}

pub fn show(app: &mut WfDiagApp, ui: &mut egui::Ui) {
    // Header with controls
    components::page_header(ui, "📊 System Monitor", |ui| {
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
    });

    ui.add_space(components::SPACE_LG);
    ui.separator();
    ui.add_space(components::SPACE_LG);

    #[cfg(windows)]
    {
        if !app.monitoring_state.is_active || app.monitoring_state.stats.is_none() {
            let (title, desc) = if app.monitoring_state.is_active {
                ("Initializing...", "Collecting system statistics...")
            } else {
                ("System Monitoring Inactive", "Click Start to begin real-time system monitoring")
            };
            components::empty_state(ui, "📈", title, desc);
            return;
        }

        // AI Analysis Panel at top (before scroll area for visibility)
        super::ai::render_monitoring_ai_panel(ui, app);
        ui.add_space(12.0);

        // Clone stats to avoid borrow conflicts in scroll area
        let stats = app.monitoring_state.stats.clone().unwrap();

        // Main content in a scroll area
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Stats cards row
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;

                    // CPU Card
                    stat_card(
                        ui,
                        "CPU",
                        stats.cpu_utilization,
                        "%",
                        colors::CHART_CPU,
                        Some(&format!("{} MHz", stats.cpu_frequency)),
                    );

                    // Memory Card
                    stat_card(
                        ui,
                        "Memory",
                        stats.memory_utilization,
                        "%",
                        colors::CHART_MEMORY,
                        Some(&format!(
                            "{:.1} / {:.1} GB",
                            stats.memory_used_gb, stats.memory_total_gb
                        )),
                    );

                    // Network Card
                    let net_value = stats.network_download_kb + stats.network_upload_kb;
                    stat_card(
                        ui,
                        "Network",
                        net_value as f32,
                        "KB/s",
                        colors::CHART_UPLOAD,
                        Some(&format!(
                            "↑{:.1} ↓{:.1}",
                            stats.network_upload_kb, stats.network_download_kb
                        )),
                    );

                    // Swap Card
                    stat_card(
                        ui,
                        "Swap",
                        stats.swap_utilization,
                        "%",
                        colors::WARNING,
                        Some(&format!(
                            "{:.1} / {:.1} GB",
                            stats.swap_used_gb, stats.swap_total_gb
                        )),
                    );
                });

                ui.add_space(24.0);

                // NPU Card - always show, with different states
                {
                    let npu_color = colors::CHART_UPLOAD; // Purple for NPU
                    let (border_color, icon_bg_color) = if stats.npu_available {
                        (npu_color.linear_multiply(0.5), npu_color)
                    } else {
                        (Color32::from_gray(80), Color32::from_gray(100))
                    };

                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .corner_radius(8.0)
                        .inner_margin(Margin::same(16))
                        .stroke(Stroke::new(1.0, border_color))
                        .show(ui, |ui| {
                            // Header with icon
                            ui.horizontal(|ui| {
                                egui::Frame::new()
                                    .fill(icon_bg_color)
                                    .corner_radius(8.0)
                                    .inner_margin(Margin::same(8))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new("🧠").size(20.0));
                                    });
                                ui.add_space(12.0);
                                ui.label(RichText::new("NPU").size(16.0).strong());
                                if !stats.npu_available {
                                    ui.add_space(8.0);
                                    ui.label(RichText::new("(Not Detected)").size(12.0).weak());
                                }
                            });

                            ui.add_space(12.0);

                            if stats.npu_available {
                                // NPU Name
                                if let Some(ref name) = stats.npu_name {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("⚙").size(12.0).weak());
                                        ui.add_space(4.0);
                                        ui.label(RichText::new(name).size(11.0).weak());
                                    });
                                    ui.add_space(8.0);
                                }

                                // Utilization or status message
                                if let Some(util) = stats.npu_utilization {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("Usage").size(11.0).weak());
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            let color = colors::utilization_color(util);
                                            ui.label(RichText::new(format!("{:.1}%", util)).size(11.0).color(color).strong());
                                        });
                                    });
                                    ui.add_space(4.0);

                                    // Color-coded progress bar
                                    components::utilization_bar(ui, util, ui.available_width(), 8.0);
                                } else {
                                    // Check if it's a Qualcomm NPU
                                    let is_qualcomm = stats.npu_name.as_ref()
                                        .map(|n| n.to_lowercase().contains("qualcomm") || n.to_lowercase().contains("hexagon"))
                                        .unwrap_or(false);

                                    egui::Frame::new()
                                        .fill(Color32::from_rgba_unmultiplied(100, 116, 139, 25))
                                        .corner_radius(6.0)
                                        .inner_margin(Margin::same(8))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new("ℹ").size(11.0).weak());
                                                ui.add_space(4.0);
                                                ui.label(RichText::new(
                                                    if is_qualcomm {
                                                        "Qualcomm NPU metrics not exposed via Windows APIs"
                                                    } else {
                                                        "Usage metrics not available"
                                                    }
                                                ).size(10.0).weak());
                                            });
                                        });
                                }
                            } else {
                                // Not detected state
                                egui::Frame::new()
                                    .fill(Color32::from_rgba_unmultiplied(100, 116, 139, 25))
                                    .corner_radius(6.0)
                                    .inner_margin(Margin::same(8))
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new("ℹ").size(11.0).weak());
                                                ui.add_space(4.0);
                                                ui.label(RichText::new("No Neural Processing Unit detected").size(10.0).weak());
                                            });
                                            ui.add_space(4.0);
                                            ui.label(RichText::new(
                                                "NPUs are available on Copilot+ PCs with Snapdragon X, Intel Core Ultra, or AMD Ryzen AI processors."
                                            ).size(9.0).weak());
                                        });
                                    });
                            }
                        });
                    ui.add_space(16.0);
                }

                // CPU History Chart
                ui.add_space(8.0);
                ui.label(RichText::new("📈 CPU Usage History").size(14.0).strong());
                ui.add_space(8.0);
                draw_line_chart(
                    ui,
                    &app.monitoring_state.cpu_history,
                    colors::CHART_CPU,
                    100.0,
                );

                ui.add_space(24.0);

                // Memory History Chart
                ui.label(RichText::new("📊 Memory Usage History").size(14.0).strong());
                ui.add_space(8.0);
                draw_line_chart(
                    ui,
                    &app.monitoring_state.memory_history,
                    colors::CHART_MEMORY,
                    100.0,
                );

                ui.add_space(24.0);

                // Per-Core CPU Usage
                if !stats.per_cpu_utilization.is_empty() {
                    ui.label(RichText::new("🔲 CPU Cores").size(14.0).strong());
                    ui.add_space(8.0);
                    draw_bar_chart(ui, &stats.per_cpu_utilization);
                    ui.add_space(24.0);
                }

                // Disk Cards
                if !stats.disks.is_empty() {
                    ui.label(RichText::new("💾 Storage").size(14.0).strong());
                    ui.add_space(8.0);

                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(12.0, 12.0);

                        for disk in &stats.disks {
                            disk_card(ui, disk);
                        }
                    });
                    ui.add_space(24.0);
                }

                // Network Activity Chart
                ui.label(RichText::new("🌐 Network Activity").size(14.0).strong());
                ui.add_space(8.0);
                draw_dual_line_chart(
                    ui,
                    &app.monitoring_state.network_up_history,
                    &app.monitoring_state.network_down_history,
                    colors::CHART_UPLOAD,
                    colors::CHART_DOWNLOAD,
                );
            });
    }

    #[cfg(not(windows))]
    {
        components::empty_state(ui, "📈", "System Monitoring", "Only available on Windows");
    }
}

fn stat_card(
    ui: &mut egui::Ui,
    label: &str,
    value: f32,
    suffix: &str,
    color: Color32,
    subtitle: Option<&str>,
) {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(8.0)
        .inner_margin(Margin::same(16))
        .stroke(Stroke::new(1.0, color.linear_multiply(0.5)))
        .show(ui, |ui| {
            ui.set_min_size(Vec2::new(140.0, 90.0));
            ui.vertical(|ui| {
                ui.label(RichText::new(label).size(12.0).weak());
                ui.add_space(4.0);

                // Progress bar with utilization coloring
                let progress = (value / 100.0).clamp(0.0, 1.0);
                let bar_color = if suffix == "%" {
                    colors::utilization_color(value)
                } else {
                    color
                };

                ui.add(
                    egui::ProgressBar::new(progress)
                        .fill(bar_color)
                        .desired_width(120.0),
                );

                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{:.1}{}", value, suffix))
                        .size(20.0)
                        .strong()
                        .color(color),
                );

                if let Some(sub) = subtitle {
                    ui.label(RichText::new(sub).size(10.0).weak());
                }
            });
        });
}

#[cfg(windows)]
fn disk_card(ui: &mut egui::Ui, disk: &wfdiag_tauri::native_monitor::DiskInfo) {
    let color = colors::WARNING;
    let bar_color = colors::storage_color(disk.utilization);

    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(8.0)
        .inner_margin(Margin::same(12))
        .stroke(Stroke::new(1.0, color.linear_multiply(0.3)))
        .show(ui, |ui| {
            ui.set_min_width(180.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("💿").size(16.0));
                    ui.add_space(4.0);
                    ui.label(RichText::new(&disk.mount_point).size(14.0).strong());
                });

                ui.label(
                    RichText::new(format!("{} • {}", disk.disk_type, disk.file_system))
                        .size(10.0)
                        .weak(),
                );

                ui.add_space(8.0);

                ui.add(
                    egui::ProgressBar::new(disk.utilization / 100.0)
                        .fill(bar_color)
                        .desired_width(160.0),
                );

                ui.add_space(4.0);

                ui.label(
                    RichText::new(format!("{:.1} / {:.1} GB", disk.used_gb, disk.total_gb))
                        .size(11.0),
                );
                ui.label(
                    RichText::new(format!("{:.1} GB free", disk.available_gb))
                        .size(10.0)
                        .weak(),
                );
            });
        });
}

fn draw_line_chart(ui: &mut egui::Ui, data: &VecDeque<f32>, color: Color32, max_value: f32) {
    let available_width = ui.available_width().min(800.0);
    let height = 120.0;

    let (rect, _response) =
        ui.allocate_exact_size(Vec2::new(available_width, height), egui::Sense::hover());

    // Background
    ui.painter()
        .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    ui.painter().rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );

    if data.is_empty() {
        return;
    }

    // Grid lines
    for i in 1..4 {
        let y = rect.top() + (height * i as f32 / 4.0);
        ui.painter().line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            Stroke::new(0.5, Color32::from_gray(60)),
        );
    }

    // Draw the line
    let points: Vec<egui::Pos2> = data
        .iter()
        .enumerate()
        .map(|(i, &val)| {
            let x = rect.left() + (i as f32 / MAX_HISTORY as f32) * rect.width();
            let y = rect.bottom() - (val / max_value) * (height - 10.0);
            egui::pos2(x, y.clamp(rect.top() + 5.0, rect.bottom() - 5.0))
        })
        .collect();

    if points.len() >= 2 {
        // Draw filled area
        let mut fill_points = points.clone();
        fill_points.push(egui::pos2(points.last().unwrap().x, rect.bottom()));
        fill_points.push(egui::pos2(points.first().unwrap().x, rect.bottom()));

        ui.painter().add(egui::Shape::convex_polygon(
            fill_points,
            color.linear_multiply(0.2),
            Stroke::NONE,
        ));

        // Draw line
        for i in 1..points.len() {
            ui.painter()
                .line_segment([points[i - 1], points[i]], Stroke::new(2.0, color));
        }
    }

    // Current value label
    if let Some(&last) = data.back() {
        let text = format!("{:.1}%", last);
        ui.painter().text(
            rect.right_top() + Vec2::new(-8.0, 8.0),
            egui::Align2::RIGHT_TOP,
            text,
            egui::FontId::proportional(11.0),
            color,
        );
    }
}

fn draw_dual_line_chart(
    ui: &mut egui::Ui,
    data1: &VecDeque<f64>,
    data2: &VecDeque<f64>,
    color1: Color32,
    color2: Color32,
) {
    let available_width = ui.available_width().min(800.0);
    let height = 120.0;

    let (rect, _response) =
        ui.allocate_exact_size(Vec2::new(available_width, height), egui::Sense::hover());

    // Background
    ui.painter()
        .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    ui.painter().rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );

    // Find max value for scaling
    let max1 = data1.iter().cloned().fold(1.0f64, f64::max);
    let max2 = data2.iter().cloned().fold(1.0f64, f64::max);
    let max_value = (max1.max(max2) * 1.2).max(10.0);

    // Draw lines
    for (data, color) in [(data1, color1), (data2, color2)] {
        if data.len() >= 2 {
            let points: Vec<egui::Pos2> = data
                .iter()
                .enumerate()
                .map(|(i, &val)| {
                    let x = rect.left() + (i as f32 / MAX_HISTORY as f32) * rect.width();
                    let y = rect.bottom() - ((val / max_value) as f32) * (height - 10.0);
                    egui::pos2(x, y.clamp(rect.top() + 5.0, rect.bottom() - 5.0))
                })
                .collect();

            for i in 1..points.len() {
                ui.painter()
                    .line_segment([points[i - 1], points[i]], Stroke::new(2.0, color));
            }
        }
    }

    // Legend
    ui.painter().text(
        rect.left_top() + Vec2::new(8.0, 8.0),
        egui::Align2::LEFT_TOP,
        format!("↑ {:.1} KB/s", data1.back().unwrap_or(&0.0)),
        egui::FontId::proportional(10.0),
        color1,
    );
    ui.painter().text(
        rect.left_top() + Vec2::new(8.0, 22.0),
        egui::Align2::LEFT_TOP,
        format!("↓ {:.1} KB/s", data2.back().unwrap_or(&0.0)),
        egui::FontId::proportional(10.0),
        color2,
    );
}

fn draw_bar_chart(ui: &mut egui::Ui, data: &[f32]) {
    let available_width = ui.available_width().min(800.0);
    let height = 80.0;
    let bar_spacing = 2.0;

    let (rect, _response) =
        ui.allocate_exact_size(Vec2::new(available_width, height), egui::Sense::hover());

    // Background
    ui.painter()
        .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);

    if data.is_empty() {
        return;
    }

    let bar_width = (rect.width() - bar_spacing * (data.len() as f32 + 1.0)) / data.len() as f32;

    for (i, &val) in data.iter().enumerate() {
        let bar_height = (val / 100.0) * (height - 20.0);
        let x = rect.left() + bar_spacing + (bar_width + bar_spacing) * i as f32;
        let y = rect.bottom() - bar_height - 10.0;

        let color = if val > 80.0 {
            Color32::from_rgb(239, 68, 68)
        } else if val > 50.0 {
            Color32::from_rgb(245, 158, 11)
        } else {
            Color32::from_rgb(16, 185, 129)
        };

        let bar_rect =
            egui::Rect::from_min_size(egui::pos2(x, y), Vec2::new(bar_width, bar_height));
        ui.painter().rect_filled(bar_rect, 2.0, color);

        // Core label
        ui.painter().text(
            egui::pos2(x + bar_width / 2.0, rect.bottom() - 2.0),
            egui::Align2::CENTER_BOTTOM,
            format!("{}", i),
            egui::FontId::proportional(8.0),
            Color32::GRAY,
        );
    }
}
