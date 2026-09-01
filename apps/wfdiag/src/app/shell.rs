//! The chrome's own view state: everything that is not a screen or a dialog.
//!
//! [`ShellState`] is what is left of the root component once each screen and
//! each dialog owns its own struct: which page is open, how the window is
//! painted and sized, who the host is, and the one status line. Screens read
//! it through [`crate::app::screen::ScreenCx`] and never write it — a screen
//! that needs the status line or a page change emits an
//! [`crate::app::screen::Effect`] instead.

#![deny(unsafe_code)]

use crate::app::state::Page;
use crate::fixtures::visual::{LiveTestFixture, VisualState};
use wfdiag_native_settings::AppSettings;
use wfdiag_native_system::{ArchitectureSnapshot, SystemInfo};
use windows_reactor::*;

/// The shell chrome's view state.
// Independent presentational facts, each read by a different surface.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct ShellState {
    // ---- chrome and layout --------------------------------------------
    pub(crate) page: Page,
    pub(crate) live_test_fixture: Option<LiveTestFixture>,
    pub(crate) theme: WindowTheme,
    pub(crate) effective_color_scheme: ColorScheme,
    pub(crate) window_size: WindowSize,
    pub(crate) requested_client_width: f64,
    pub(crate) requested_client_height: f64,
    pub(crate) pane_open: bool,
    pub(crate) deterministic_visual: bool,
    pub(crate) visual_state: VisualState,
    pub(crate) status: String,
    /// One-shot latch for #206 (toast failure) and #207 (degraded instance
    /// watch): both are session-level facts, so the status line states each
    /// at most once instead of re-announcing it on every scan or wake.
    pub(crate) notification_failure_reported: bool,
    pub(crate) degraded_instance_watch_reported: bool,

    /// The persisted settings document, as the engine last published it.
    /// Every surface reads it; only the Settings dialog edits a draft of it.
    pub(crate) settings: AppSettings,

    // ---- host identity -------------------------------------------------
    pub(crate) system_info: SystemInfo,
    pub(crate) architecture: Option<ArchitectureSnapshot>,
    pub(crate) system_error: Option<String>,
    pub(crate) is_admin: bool,

    // ---- window integration ---------------------------------------------
    pub(crate) window_hook_installed: bool,
    pub(crate) window_hook_retry_failures: u8,
    pub(crate) window_hook_retry_task: Option<ComponentTask>,
    pub(crate) window_lifecycle_revision: u64,
    pub(crate) window_usable: bool,
    pub(crate) instance_wait: Option<ComponentTask>,
}
