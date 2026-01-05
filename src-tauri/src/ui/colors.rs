//! Shared color constants for consistent UI styling

use eframe::egui::Color32;

// =============================================================================
// Status Colors
// =============================================================================

/// Success/healthy state - green
pub const SUCCESS: Color32 = Color32::from_rgb(80, 180, 80);

/// Error/failure state - red
pub const ERROR: Color32 = Color32::from_rgb(220, 80, 80);

/// Warning/caution state - amber
pub const WARNING: Color32 = Color32::from_rgb(245, 158, 11);

/// Info/neutral state - blue
pub const INFO: Color32 = Color32::from_rgb(59, 130, 246);

// =============================================================================
// Primary/Accent Colors
// =============================================================================

/// Primary accent - indigo
pub const PRIMARY: Color32 = Color32::from_rgb(99, 102, 241);

/// Secondary success - teal/green
pub const SUCCESS_SECONDARY: Color32 = Color32::from_rgb(16, 185, 129);

/// Link/action color - lighter blue
pub const LINK: Color32 = Color32::from_rgb(96, 165, 250);

// =============================================================================
// Progress Bar Colors
// =============================================================================

/// Get color based on utilization percentage (0-100)
/// Returns green for low, amber for medium, red for high usage
pub fn utilization_color(percent: f32) -> Color32 {
    if percent > 90.0 {
        Color32::from_rgb(239, 68, 68) // Red - critical
    } else if percent > 75.0 {
        ERROR // Red-ish - high
    } else if percent > 50.0 {
        WARNING // Amber - medium
    } else {
        SUCCESS_SECONDARY // Green - healthy
    }
}

/// Get color for disk/storage usage
pub fn storage_color(percent: f32) -> Color32 {
    if percent > 95.0 {
        Color32::from_rgb(239, 68, 68) // Red - critical
    } else if percent > 85.0 {
        ERROR
    } else if percent > 70.0 {
        WARNING
    } else {
        SUCCESS_SECONDARY
    }
}

// =============================================================================
// Chart Colors
// =============================================================================

/// CPU chart line color
pub const CHART_CPU: Color32 = Color32::from_rgb(59, 130, 246);

/// Memory chart line color
pub const CHART_MEMORY: Color32 = Color32::from_rgb(16, 185, 129);

/// Network upload color
pub const CHART_UPLOAD: Color32 = Color32::from_rgb(168, 85, 247);

/// Network download color
pub const CHART_DOWNLOAD: Color32 = Color32::from_rgb(34, 211, 238);

// =============================================================================
// Card/Frame Background Colors (functions due to non-const from_rgba_unmultiplied)
// =============================================================================

/// CPU stat card background
#[inline]
pub fn card_cpu_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(59, 130, 246, 25)
}

/// Memory stat card background
#[inline]
pub fn card_memory_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(16, 185, 129, 25)
}

/// Swap stat card background
#[inline]
pub fn card_swap_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(245, 158, 11, 25)
}

/// NPU stat card background
#[inline]
pub fn card_npu_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(168, 85, 247, 25)
}

/// AI panel background
#[inline]
pub fn ai_panel_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(99, 102, 241, 20)
}

/// AI panel stroke
#[inline]
pub fn ai_panel_stroke() -> Color32 {
    Color32::from_rgba_unmultiplied(99, 102, 241, 50)
}

// =============================================================================
// Text/Label Colors (for dark mode compatible usage)
// =============================================================================

/// Dark mode compatible green text
pub const TEXT_SUCCESS_DARK: Color32 = Color32::from_rgb(134, 239, 172);

/// Dark mode compatible red text
pub const TEXT_ERROR_DARK: Color32 = Color32::from_rgb(252, 165, 165);
