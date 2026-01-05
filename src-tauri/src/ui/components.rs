//! Reusable UI components and helpers for consistent styling

use eframe::egui::{self, Color32, Margin, RichText, Vec2};
use super::colors;

// =============================================================================
// Spacing Helpers
// =============================================================================

/// Standard small spacing (8px)
pub const SPACE_SM: f32 = 8.0;

/// Standard medium spacing (12px)
pub const SPACE_MD: f32 = 12.0;

/// Standard large spacing (16px)
pub const SPACE_LG: f32 = 16.0;

/// Standard extra-large spacing (24px)
pub const SPACE_XL: f32 = 24.0;

/// Add standard section separator with spacing
pub fn section_separator(ui: &mut egui::Ui) {
    ui.add_space(SPACE_SM);
    ui.separator();
    ui.add_space(SPACE_SM);
}

// =============================================================================
// Empty State Component
// =============================================================================

/// Render a centered empty state with icon, title, and description
pub fn empty_state(ui: &mut egui::Ui, icon: &str, title: &str, description: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(80.0);
        ui.label(RichText::new(icon).size(48.0).weak());
        ui.add_space(SPACE_LG);
        ui.label(RichText::new(title).size(18.0).strong());
        ui.add_space(SPACE_SM);
        ui.label(RichText::new(description).size(13.0).weak());
    });
}

/// Render empty state with an action button
pub fn empty_state_with_action(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    description: &str,
    button_text: &str,
) -> bool {
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space(80.0);
        ui.label(RichText::new(icon).size(48.0).weak());
        ui.add_space(SPACE_LG);
        ui.label(RichText::new(title).size(18.0).strong());
        ui.add_space(SPACE_SM);
        ui.label(RichText::new(description).size(13.0).weak());
        ui.add_space(SPACE_LG);
        if ui.button(button_text).clicked() {
            clicked = true;
        }
    });
    clicked
}

// =============================================================================
// Status Badge Component
// =============================================================================

/// Render a colored status badge
pub fn status_badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.linear_multiply(0.2))
        .corner_radius(4.0)
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.0).color(color));
        });
}

/// Render a live/active status indicator
pub fn live_badge(ui: &mut egui::Ui, is_live: bool) {
    let (text, color) = if is_live {
        ("● Live", colors::SUCCESS)
    } else {
        ("○ Inactive", Color32::GRAY)
    };
    status_badge(ui, text, color);
}

/// Render a success/failure status indicator
pub fn result_badge(ui: &mut egui::Ui, success: bool) {
    let (text, color) = if success {
        ("✓ Passed", colors::SUCCESS)
    } else {
        ("✗ Failed", colors::ERROR)
    };
    status_badge(ui, text, color);
}

// =============================================================================
// Stat Card Component
// =============================================================================

/// Configuration for a stat card
pub struct StatCardConfig<'a> {
    pub title: &'a str,
    pub value: &'a str,
    pub subtitle: Option<&'a str>,
    pub bg_color: Color32,
    pub icon: Option<&'a str>,
}

/// Render a stat card with value and optional subtitle
pub fn stat_card(ui: &mut egui::Ui, config: StatCardConfig) {
    let frame_color = config.bg_color;

    egui::Frame::new()
        .fill(frame_color)
        .corner_radius(8.0)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                // Header with optional icon
                ui.horizontal(|ui| {
                    if let Some(icon) = config.icon {
                        ui.label(RichText::new(icon).size(14.0));
                    }
                    ui.label(RichText::new(config.title).size(11.0).strong());
                });

                ui.add_space(4.0);

                // Main value
                ui.label(RichText::new(config.value).size(24.0).strong());

                // Optional subtitle
                if let Some(subtitle) = config.subtitle {
                    ui.label(RichText::new(subtitle).size(10.0).weak());
                }
            });
        });
}

// =============================================================================
// Progress Bar Component
// =============================================================================

/// Render a colored progress bar based on utilization
pub fn utilization_bar(ui: &mut egui::Ui, percent: f32, width: f32, height: f32) {
    let bar_color = colors::utilization_color(percent);
    let bg_color = ui.visuals().widgets.inactive.bg_fill;

    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();

        // Background
        painter.rect_filled(rect, 3.0, bg_color);

        // Filled portion
        let filled_width = rect.width() * (percent / 100.0).clamp(0.0, 1.0);
        let filled_rect = egui::Rect::from_min_size(
            rect.min,
            Vec2::new(filled_width, rect.height()),
        );
        painter.rect_filled(filled_rect, 3.0, bar_color);
    }
}

// =============================================================================
// Frame Helpers
// =============================================================================

/// Standard content frame with background
pub fn content_frame(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(8.0)
        .inner_margin(Margin::same(12))
        .show(ui, add_contents);
}

/// Colored info/alert frame
pub fn alert_frame(ui: &mut egui::Ui, color: Color32, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(color.linear_multiply(0.15))
        .corner_radius(6.0)
        .inner_margin(Margin::same(10))
        .stroke(egui::Stroke::new(1.0, color.linear_multiply(0.4)))
        .show(ui, add_contents);
}

/// Error alert with icon
pub fn error_alert(ui: &mut egui::Ui, message: &str) {
    alert_frame(ui, colors::ERROR, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("⚠").color(colors::ERROR));
            ui.label(RichText::new(message).color(colors::ERROR));
        });
    });
}

/// Info alert with icon
pub fn info_alert(ui: &mut egui::Ui, message: &str) {
    alert_frame(ui, colors::INFO, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("ℹ").color(colors::INFO));
            ui.label(RichText::new(message).color(colors::INFO));
        });
    });
}

/// Warning alert with icon
pub fn warning_alert(ui: &mut egui::Ui, message: &str) {
    alert_frame(ui, colors::WARNING, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("⚠").color(colors::WARNING));
            ui.label(RichText::new(message).color(colors::WARNING));
        });
    });
}

// =============================================================================
// Header Component
// =============================================================================

/// Render a page header with title and right-aligned controls
pub fn page_header(
    ui: &mut egui::Ui,
    title: &str,
    right_controls: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.heading(RichText::new(title).strong());

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right_controls);
    });
}

/// Render a page header with icon
pub fn page_header_with_icon(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    right_controls: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(20.0));
        ui.heading(RichText::new(title).strong());

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right_controls);
    });
}

// =============================================================================
// Count/Stats Display
// =============================================================================

/// Render a pass/fail count display
pub fn pass_fail_counts(ui: &mut egui::Ui, passed: usize, failed: usize) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("✓ {}", passed))
                .size(12.0)
                .color(colors::SUCCESS),
        );
        if failed > 0 {
            ui.label(
                RichText::new(format!("✗ {}", failed))
                    .size(12.0)
                    .color(colors::ERROR),
            );
        }
    });
}

/// Format duration in human readable form
pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let mins = ms / 60000;
        let secs = (ms % 60000) / 1000;
        format!("{}m {}s", mins, secs)
    }
}

/// Format bytes in human readable form
pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}
