//! Environment knobs that select deterministic behaviour for validation.
//!
//! # Invariant (#186, #212)
//!
//! This module is the ONLY place in `apps/wfdiag` that may read the process
//! environment, and every one of those reads is compiled out unless the
//! `validation` cargo feature is enabled. A shipping build therefore contains
//! no knob at all: each accessor below collapses to the production default
//! with no `std::env` call reachable from it, which is asserted by the
//! `production_defaults` test module (compiled only without the feature).
//!
//! The single deliberate environment/command-line read outside this module is
//! `main.rs`'s `--wfdiag-elevated-relaunch` argument check, which is real
//! production behaviour, plus `platform/crash.rs`'s `%LOCALAPPDATA%` lookup
//! for the crash-log directory. Engine crates own their own environment
//! variables (`WFDIAG_LAF_TOKEN`, `WFDIAG_AI_LOG`, `WFDIAG_ACTIVATION_ORDER`,
//! …) and are untouched by this rule.

#![deny(unsafe_code)]

use crate::app::state::Page;
use crate::fixtures::visual::{LiveTestFixture, VisualState};

#[cfg(feature = "validation")]
pub(crate) const LIVE_TEST_FIXTURE_ENV: &str = "WFDIAG_REACTOR_LIVE_TEST_FIXTURE";

#[cfg(feature = "validation")]
const VISUAL_STATE_ENV: &str = "WFDIAG_REACTOR_VISUAL_STATE";

#[cfg(feature = "validation")]
const FIXTURE_ENV: &str = "WFDIAG_REACTOR_FIXTURE";

#[cfg(feature = "validation")]
const PAGE_ENV: &str = "WFDIAG_REACTOR_PAGE";

#[cfg(feature = "validation")]
const THEME_ENV: &str = "WFDIAG_REACTOR_THEME";

#[cfg(feature = "validation")]
const SETTINGS_OPEN_ENV: &str = "WFDIAG_REACTOR_SETTINGS";

#[cfg(feature = "validation")]
const WIDTH_ENV: &str = "WFDIAG_REACTOR_WIDTH";

#[cfg(feature = "validation")]
const HEIGHT_ENV: &str = "WFDIAG_REACTOR_HEIGHT";

#[cfg(feature = "validation")]
const NO_TRAY_ENV: &str = "WFDIAG_NO_TRAY";

#[cfg(feature = "validation")]
const NO_WORKERS_ENV: &str = "WFDIAG_NO_WORKERS";

#[cfg(feature = "settings-test-path")]
const SETTINGS_TEST_PATH_ENV: &str = "WFDIAG_REACTOR_SETTINGS_TEST_PATH";

/// The deterministic screenshot/QA visual state for this run.
///
/// The production default is routed through the same pure parser so the
/// non-`Live` variants stay constructed (and therefore lint-clean) in a build
/// that has no way to select them.
#[cfg(feature = "validation")]
pub(crate) fn visual_state() -> VisualState {
    VisualState::parse(&std::env::var(VISUAL_STATE_ENV).unwrap_or_default())
}

#[cfg(not(feature = "validation"))]
pub(crate) fn visual_state() -> VisualState {
    VisualState::parse("")
}

/// The closed live-path fixture selected for this run, if any.
#[cfg(feature = "validation")]
pub(crate) fn live_test_fixture_from_env() -> Option<LiveTestFixture> {
    LiveTestFixture::parse(&std::env::var(LIVE_TEST_FIXTURE_ENV).unwrap_or_default())
}

#[cfg(not(feature = "validation"))]
pub(crate) fn live_test_fixture_from_env() -> Option<LiveTestFixture> {
    LiveTestFixture::parse("")
}

/// Startup page override; `None` means "use the visual state's own default".
#[cfg(feature = "validation")]
pub(crate) fn initial_page_override() -> Option<Page> {
    std::env::var(PAGE_ENV)
        .ok()
        .as_deref()
        .and_then(Page::from_tag)
}

#[cfg(not(feature = "validation"))]
pub(crate) fn initial_page_override() -> Option<Page> {
    None
}

/// Whether the populated-content screenshot fixture is selected.
#[cfg(feature = "validation")]
pub(crate) fn fixture_mode() -> bool {
    std::env::var(FIXTURE_ENV).is_ok_and(|value| value.eq_ignore_ascii_case("populated"))
}

#[cfg(not(feature = "validation"))]
pub(crate) fn fixture_mode() -> bool {
    false
}

/// Whether the Settings dialog should be open at startup.
#[cfg(feature = "validation")]
pub(crate) fn settings_dialog_open_override() -> bool {
    std::env::var(SETTINGS_OPEN_ENV)
        .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

#[cfg(not(feature = "validation"))]
pub(crate) fn settings_dialog_open_override() -> bool {
    false
}

/// Startup theme override in the `light|dark|system` settings vocabulary.
///
/// An empty string means "no override": the caller resolves the theme exactly
/// as it does for a stored setting it could not read.
#[cfg(feature = "validation")]
pub(crate) fn startup_theme_setting() -> String {
    std::env::var(THEME_ENV).unwrap_or_default()
}

#[cfg(not(feature = "validation"))]
pub(crate) fn startup_theme_setting() -> String {
    String::new()
}

/// Startup client width, falling back to the visual state's own default.
#[cfg(feature = "validation")]
pub(crate) fn initial_window_width(fallback: f64) -> f64 {
    parse_window_dimension(std::env::var(WIDTH_ENV).ok().as_deref(), fallback)
}

#[cfg(not(feature = "validation"))]
pub(crate) fn initial_window_width(fallback: f64) -> f64 {
    fallback
}

/// Startup client height, falling back to the visual state's own default.
#[cfg(feature = "validation")]
pub(crate) fn initial_window_height(fallback: f64) -> f64 {
    parse_window_dimension(std::env::var(HEIGHT_ENV).ok().as_deref(), fallback)
}

#[cfg(not(feature = "validation"))]
pub(crate) fn initial_window_height(fallback: f64) -> f64 {
    fallback
}

/// Reject non-finite and absurdly small windows so a typo cannot produce an
/// unusable candidate the capture harness then screenshots.
#[cfg(any(feature = "validation", test))]
fn parse_window_dimension(value: Option<&str>, fallback: f64) -> f64 {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 540.0)
        .unwrap_or(fallback)
}

/// Whether the notification-area icon should be installed.
#[cfg(feature = "validation")]
pub(crate) fn tray_enabled() -> bool {
    std::env::var_os(NO_TRAY_ENV).is_none()
}

#[cfg(not(feature = "validation"))]
pub(crate) fn tray_enabled() -> bool {
    true
}

/// The harness's AI-worker suppression policy; empty means "start everything".
#[cfg(feature = "validation")]
pub(crate) fn ai_worker_policy() -> std::ffi::OsString {
    std::env::var_os(NO_WORKERS_ENV).unwrap_or_default()
}

#[cfg(not(feature = "validation"))]
pub(crate) fn ai_worker_policy() -> std::ffi::OsString {
    std::ffi::OsString::new()
}

/// Isolated settings-store path used by the integration validation suites.
///
/// Only compiled with `settings-test-path`; a shipping build has no accessor
/// for it at all, which is a stronger guarantee than returning `None`.
#[cfg(feature = "settings-test-path")]
pub(crate) fn settings_test_path() -> Option<std::ffi::OsString> {
    std::env::var_os(SETTINGS_TEST_PATH_ENV)
}

/// Name of the isolated settings-store variable, for harness diagnostics.
#[cfg(feature = "settings-test-path")]
pub(crate) fn settings_test_path_env_name() -> &'static str {
    SETTINGS_TEST_PATH_ENV
}

/// Answer `--wfdiag-version-probe` without initializing WinUI.
///
/// Returns true when the probe ran and `main` should exit immediately. The
/// whole entry point is validation-only (#212): a shipping build never
/// inspects its command line here and never writes a probe document.
#[cfg(feature = "validation")]
pub(crate) fn write_version_probe_if_requested() -> bool {
    use crate::app::consts::{VERSION_PROBE_FILE_ENV, VERSION_PROBE_FLAG};

    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new(VERSION_PROBE_FLAG)) {
        return false;
    }

    // The capture harness passes the destination through the environment so
    // paths containing spaces or non-ASCII characters never need shell
    // quoting. Reject additional arguments to keep this probe deterministic.
    if arguments.next().is_some() {
        std::process::exit(2);
    }
    let Some(path) = std::env::var_os(VERSION_PROBE_FILE_ENV).filter(|path| !path.is_empty())
    else {
        std::process::exit(2);
    };
    if std::fs::write(path, version_probe_document()).is_err() {
        std::process::exit(3);
    }
    true
}

#[cfg(not(feature = "validation"))]
pub(crate) fn write_version_probe_if_requested() -> bool {
    false
}

/// The probe document the capture harness parses.
#[cfg(any(feature = "validation", test))]
pub(crate) fn version_probe_document() -> String {
    format!(
        "{{\"schema\":1,\"application_version\":\"{}\",\"settings_test_path\":{}}}\n",
        crate::app::consts::APP_VERSION,
        cfg!(feature = "settings-test-path")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wfdiag_native_remediation::broker::ActionRequest;

    #[test]
    fn live_test_fixture_parser_and_action_allowlist_are_closed() {
        assert_eq!(
            LiveTestFixture::parse("device-manager"),
            Some(LiveTestFixture::DeviceManager)
        );
        assert_eq!(
            LiveTestFixture::parse("export-fallback"),
            Some(LiveTestFixture::ExportFallback)
        );
        assert_eq!(
            LiveTestFixture::parse("admin-relaunch"),
            Some(LiveTestFixture::AdminRelaunch)
        );
        for rejected in [
            "",
            "Device-Manager",
            "open_device_manager",
            "device-manager ",
            "network-reset",
        ] {
            assert_eq!(LiveTestFixture::parse(rejected), None, "{rejected:?}");
        }

        let device_manager = ActionRequest {
            remediation_id: "open_device_manager".to_string(),
            issue_id: Some("device_manager_errors".to_string()),
        };
        let disallowed = ActionRequest {
            remediation_id: "flush_dns".to_string(),
            issue_id: None,
        };
        assert!(
            LiveTestFixture::DeviceManager.permits_actions(std::slice::from_ref(&device_manager))
        );
        assert!(!LiveTestFixture::DeviceManager.permits_actions(&[]));
        assert!(!LiveTestFixture::DeviceManager.permits_actions(&[device_manager, disallowed]));
        assert!(!LiveTestFixture::ExportFallback.permits_actions(&[]));
        assert!(!LiveTestFixture::AdminRelaunch.permits_actions(&[]));
    }

    #[test]
    fn window_dimensions_reject_unusable_and_malformed_values() {
        assert!((parse_window_dimension(Some("1440"), 1200.0) - 1440.0).abs() < f64::EPSILON);
        for rejected in [None, Some(""), Some("nope"), Some("539.9"), Some("NaN")] {
            assert!(
                (parse_window_dimension(rejected, 1200.0) - 1200.0).abs() < f64::EPSILON,
                "{rejected:?}"
            );
        }
    }

    #[test]
    fn version_probe_document_uses_the_canonical_build_version() {
        assert_eq!(
            version_probe_document(),
            format!(
                "{{\"schema\":1,\"application_version\":\"{}\",\"settings_test_path\":{}}}\n",
                env!("WFDIAG_APP_VERSION"),
                cfg!(feature = "settings-test-path")
            )
        );
    }
}

/// Compiled only for a shipping-shaped build: every knob must resolve to its
/// production default with no environment access at all (#186, #212).
#[cfg(all(test, not(feature = "validation")))]
mod production_defaults {
    use super::*;

    #[test]
    fn knobs_are_production_defaults_without_the_validation_feature() {
        assert_eq!(visual_state(), VisualState::Live);
        assert_eq!(live_test_fixture_from_env(), None);
        assert_eq!(initial_page_override(), None);
        assert!(!fixture_mode());
        assert!(!settings_dialog_open_override());
        assert!(startup_theme_setting().is_empty());
        assert!((initial_window_width(1200.0) - 1200.0).abs() < f64::EPSILON);
        assert!((initial_window_height(800.0) - 800.0).abs() < f64::EPSILON);
        assert!(tray_enabled());
        assert!(ai_worker_policy().is_empty());
        assert!(!write_version_probe_if_requested());
    }
}
