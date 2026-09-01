//! Environment knobs that select deterministic behaviour for validation.

#![deny(unsafe_code)]

use crate::fixtures::visual::LiveTestFixture;

#[cfg(feature = "settings-test-path")]
pub(crate) const LIVE_TEST_FIXTURE_ENV: &str = "WFDIAG_REACTOR_LIVE_TEST_FIXTURE";

#[cfg(feature = "settings-test-path")]
pub(crate) fn live_test_fixture_from_env() -> Option<LiveTestFixture> {
    std::env::var(LIVE_TEST_FIXTURE_ENV)
        .ok()
        .as_deref()
        .and_then(LiveTestFixture::parse)
}

#[cfg(not(feature = "settings-test-path"))]
pub(crate) fn live_test_fixture_from_env() -> Option<LiveTestFixture> {
    None
}

pub(crate) fn initial_window_dimension(name: &str, fallback: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 540.0)
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wfdiag_native_remediation::broker::ActionRequest;

    #[test]
    fn live_test_fixture_parser_and_action_allowlist_are_closed() {
        #[cfg(feature = "settings-test-path")]
        {
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
}
