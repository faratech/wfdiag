//! Host-neutral update-presentation policy.
//!
//! The service in [`crate`] decides *whether* a newer release exists. This
//! module owns everything a shell needs around that decision: the once-a-day
//! startup throttle, the notice timings, and the closed set of external links
//! the About surface may open. Launching a browser is the shell's job; every
//! rule that decides *what* may be launched is here so it is testable on any
//! host.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use semver::Version;
use wfdiag_native_settings::{atomic_write_file, shipping_settings_path};

use crate::UpdateInfo;

/// One check per day, matching the shipping Store behavior.
pub const CHECK_INTERVAL: Duration = Duration::from_hours(24);
/// Clock skew this far into the future is treated as an ordinary recent check
/// rather than a corrupt timestamp.
pub const FUTURE_TIMESTAMP_GRACE: Duration = Duration::from_mins(5);
/// How long after launch the passive check may run.
pub const START_DELAY: Duration = Duration::from_secs(5);
/// How long the "update available" notice stays on screen.
pub const NOTICE_DURATION: Duration = Duration::from_secs(5);

const LAST_CHECK_FILENAME: &str = "update-check.last-run";
const WINDOWSFORUM_URL: &str = "https://windowsforum.com/";
const GITHUB_REPOSITORY_URL: &str = "https://github.com/faratech/wfdiag";
const GITHUB_RELEASE_PREFIX: &str = "https://github.com/faratech/wfdiag/releases/tag/";

/// The complete external-link surface exposed by the About dialog.
/// Callers cannot smuggle an arbitrary URL through a component message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AboutExternalAction {
    DownloadUpdate,
    WindowsForum,
    GithubRepository,
}

/// Persistence for the once-per-day startup throttle. The update service
/// itself deliberately remains filesystem-neutral.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateThrottle {
    path: PathBuf,
}

impl UpdateThrottle {
    /// Resolve the throttle file beside the shipping settings file.
    ///
    /// # Errors
    /// Returns the settings-layer diagnostic when the configuration directory
    /// cannot be resolved or created.
    pub fn shipping() -> Result<Self, String> {
        shipping_settings_path()
            .map(|path| Self::beside_settings_file(&path))
            .map_err(|error| error.to_string())
    }

    #[must_use]
    pub fn beside_settings_file(settings_path: &Path) -> Self {
        Self {
            path: settings_path.with_file_name(LAST_CHECK_FILENAME),
        }
    }

    #[must_use]
    pub fn should_check_at(&self, now_millis: u64) -> bool {
        let last_run = fs::read_to_string(&self.path).ok();
        should_check(last_run.as_deref(), now_millis)
    }

    /// Record that a check completed at `now_millis`.
    ///
    /// # Errors
    /// Returns the atomic-write diagnostic. Callers treat persistence failure
    /// as fail-open (the next launch simply checks again).
    pub fn record_at(&self, now_millis: u64) -> Result<(), String> {
        atomic_write_file(&self.path, now_millis.to_string().as_bytes())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Match the Store 2.5.8 throttle, including the five-minute tolerance for
/// harmless clock skew and fail-open handling for corrupt/far-future values.
// The stored value is a JavaScript-authored millisecond epoch, so it is
// compared as `f64` exactly as the shipping hook did; both operands stay far
// below the 2^53 exact-integer boundary.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn should_check(last_run: Option<&str>, now_millis: u64) -> bool {
    let Some(last_run) = last_run else {
        return true;
    };
    let Ok(last_run) = last_run.trim().parse::<f64>() else {
        return true;
    };
    if !last_run.is_finite() {
        return true;
    }

    let now = now_millis as f64;
    let interval = CHECK_INTERVAL.as_millis() as f64;
    let future_grace = FUTURE_TIMESTAMP_GRACE.as_millis() as f64;
    last_run > now + future_grace || now - last_run >= interval
}

/// Milliseconds since the Unix epoch, saturating rather than panicking on an
/// absurd system clock.
#[must_use]
pub fn unix_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

/// Resolve a closed action to a trusted URL. Release URLs must be the exact
/// HTTPS tag page for the normalized semantic version returned by the update
/// service; raw JSON URLs never cross the shell boundary.
///
/// # Errors
/// Returns a user-presentable reason when no update is available or the
/// release URL is not on the allowlist.
pub fn resolve_external_url(
    action: AboutExternalAction,
    update: Option<&UpdateInfo>,
) -> Result<String, String> {
    match action {
        AboutExternalAction::WindowsForum => Ok(WINDOWSFORUM_URL.to_string()),
        AboutExternalAction::GithubRepository => Ok(GITHUB_REPOSITORY_URL.to_string()),
        AboutExternalAction::DownloadUpdate => {
            let update = update.ok_or_else(|| "No update is available".to_string())?;
            trusted_release_url(update)
                .ok_or_else(|| "The release URL did not pass the WFDiag allowlist".to_string())
        }
    }
}

/// The exact GitHub release page for `update`, or `None` when the reported URL
/// is anything else at all.
#[must_use]
pub fn trusted_release_url(update: &UpdateInfo) -> Option<String> {
    let version = Version::parse(&update.version).ok()?;
    if version.to_string() != update.version {
        return None;
    }

    let with_prefix = format!("{GITHUB_RELEASE_PREFIX}v{version}");
    if update.html_url == with_prefix {
        return Some(with_prefix);
    }
    let without_prefix = format!("{GITHUB_RELEASE_PREFIX}{version}");
    if update.html_url == without_prefix {
        return Some(without_prefix);
    }
    None
}

#[cfg(test)]
#[allow(clippy::cast_possible_truncation)]
mod tests {
    use super::*;

    fn update(version: &str, url: &str) -> UpdateInfo {
        UpdateInfo {
            version: version.to_string(),
            html_url: url.to_string(),
            published_at: None,
            notes_excerpt: None,
        }
    }

    #[test]
    fn throttle_matches_the_shipping_boundaries() {
        assert_eq!(START_DELAY, Duration::from_secs(5));
        assert_eq!(NOTICE_DURATION, Duration::from_secs(5));
        assert_eq!(CHECK_INTERVAL, Duration::from_hours(24));
        let now = 1_750_000_000_000;
        let interval = CHECK_INTERVAL.as_millis() as u64;
        assert!(should_check(None, now));
        assert!(!should_check(Some(&(now - 60_000).to_string()), now));
        assert!(!should_check(Some(&(now - interval + 1).to_string()), now));
        assert!(should_check(Some(&(now - interval).to_string()), now));
        assert!(should_check(Some(&(now - 2 * interval).to_string()), now));
        assert!(should_check(Some("not-a-number"), now));
    }

    #[test]
    fn throttle_tolerates_small_clock_skew_but_recovers_from_far_future_values() {
        let now = 1_750_000_000_000;
        assert!(!should_check(Some(&(now + 60_000).to_string()), now));
        assert!(!should_check(
            Some(&(now + FUTURE_TIMESTAMP_GRACE.as_millis() as u64).to_string()),
            now
        ));
        assert!(should_check(
            Some(&(now + FUTURE_TIMESTAMP_GRACE.as_millis() as u64 + 1).to_string()),
            now
        ));
        assert!(should_check(Some("NaN"), now));
        assert!(should_check(Some("inf"), now));
    }

    #[test]
    fn throttle_uses_a_dedicated_file_beside_settings() {
        // Built from components so the sibling contract is checked on every
        // host; a Windows-literal string is one path segment on Linux.
        let directory = Path::new("roaming").join("com.windowsforum.diagnostics");
        let throttle = UpdateThrottle::beside_settings_file(&directory.join("settings.json"));
        assert_eq!(throttle.path(), directory.join("update-check.last-run"));
    }

    #[cfg(windows)]
    #[test]
    fn throttle_path_matches_the_shipping_windows_location() {
        let throttle = UpdateThrottle::beside_settings_file(Path::new(
            r"C:\Users\person\AppData\Roaming\com.windowsforum.diagnostics\settings.json",
        ));
        assert_eq!(
            throttle.path(),
            Path::new(
                r"C:\Users\person\AppData\Roaming\com.windowsforum.diagnostics\update-check.last-run"
            )
        );
    }

    #[test]
    fn only_exact_wfdiag_github_release_pages_are_trusted() {
        for url in [
            "https://github.com/faratech/wfdiag/releases/tag/v2.6.0",
            "https://github.com/faratech/wfdiag/releases/tag/2.6.0",
        ] {
            assert_eq!(
                trusted_release_url(&update("2.6.0", url)).as_deref(),
                Some(url)
            );
        }

        for url in [
            "http://github.com/faratech/wfdiag/releases/tag/v2.6.0",
            "https://github.com.evil.example/faratech/wfdiag/releases/tag/v2.6.0",
            "https://github.com/other/wfdiag/releases/tag/v2.6.0",
            "https://github.com/faratech/wfdiag/releases/tag/v9.9.9",
            "https://github.com/faratech/wfdiag/releases/tag/v2.6.0?download=1",
            "https://github.com/faratech/wfdiag/releases/tag/v2.6.0#notes",
            "https://github.com/faratech/wfdiag/releases/tag/v2.6.0/",
        ] {
            assert!(
                trusted_release_url(&update("2.6.0", url)).is_none(),
                "{url}"
            );
        }
        assert!(trusted_release_url(&update("../2.6.0", "https://github.com/")).is_none());
    }

    #[test]
    fn typed_actions_resolve_only_static_links_or_a_valid_current_update() {
        assert_eq!(
            resolve_external_url(AboutExternalAction::WindowsForum, None).unwrap(),
            WINDOWSFORUM_URL
        );
        assert_eq!(
            resolve_external_url(AboutExternalAction::GithubRepository, None).unwrap(),
            GITHUB_REPOSITORY_URL
        );
        assert!(resolve_external_url(AboutExternalAction::DownloadUpdate, None).is_err());
        assert!(
            resolve_external_url(
                AboutExternalAction::DownloadUpdate,
                Some(&update("2.6.0", "https://example.com/v2.6.0")),
            )
            .is_err()
        );
    }
}
