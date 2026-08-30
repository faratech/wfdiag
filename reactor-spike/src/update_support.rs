use semver::Version;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wfdiag_native_settings::{atomic_write_file, shipping_settings_path};
use wfdiag_native_update::UpdateInfo;

pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
pub const FUTURE_TIMESTAMP_GRACE: Duration = Duration::from_secs(5 * 60);
pub const START_DELAY: Duration = Duration::from_secs(5);
pub const NOTICE_DURATION: Duration = Duration::from_secs(5);

const LAST_CHECK_FILENAME: &str = "update-check.last-run";
const WINDOWSFORUM_URL: &str = "https://windowsforum.com/";
const GITHUB_REPOSITORY_URL: &str = "https://github.com/faratech/wfdiag";
const GITHUB_RELEASE_PREFIX: &str = "https://github.com/faratech/wfdiag/releases/tag/";

/// The complete external-link surface exposed by Reactor's About dialog.
/// Callers cannot smuggle an arbitrary URL through a component message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AboutExternalAction {
    DownloadUpdate,
    WindowsForum,
    GithubRepository,
}

/// Shell-owned persistence for the once-per-day startup throttle. The update
/// service itself deliberately remains filesystem-neutral.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateThrottle {
    path: PathBuf,
}

impl UpdateThrottle {
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

    pub fn record_at(&self, now_millis: u64) -> Result<(), String> {
        atomic_write_file(&self.path, now_millis.to_string().as_bytes())
    }

    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }
}

/// Match the Store 2.5.8 throttle, including the five-minute tolerance for
/// harmless clock skew and fail-open handling for corrupt/far-future values.
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

/// Open one typed About action through the Windows shell.
///
/// The passive update path never calls this function: launching a browser is
/// possible only after an explicit button activation.
pub fn launch_external_action(
    action: AboutExternalAction,
    update: Option<&UpdateInfo>,
) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    let url = resolve_external_url(action, update)?;
    let verb: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    let target: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        Err(format!(
            "Windows could not open the link (ShellExecute code {})",
            result.0 as isize
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
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
        assert_eq!(CHECK_INTERVAL, Duration::from_secs(24 * 60 * 60));
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
