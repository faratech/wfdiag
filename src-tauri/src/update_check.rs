//! Lightweight update check for the GitHub-distributed channel.
//!
//! The official tauri-plugin-updater only supports NSIS/MSI installs on
//! Windows — useless for our portable exe — so this just asks the GitHub
//! Releases API whether a newer version exists and lets the UI point the
//! user at the release page. No binary self-replacement, no signing infra.
//! The Microsoft Store channel updates itself and is never prompted.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub html_url: String,
    pub published_at: Option<String>,
    pub notes_excerpt: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

const RELEASES_LATEST_URL: &str = "https://api.github.com/repos/faratech/wfdiag/releases/latest";

/// True when this process is the Microsoft Store install.
///
/// `has_package_identity()` alone is NOT sufficient: a dev sparse-registered
/// exe has package identity too, but reports SignatureKind Developer/None —
/// only a genuine Store install reports Store. On any API failure we err
/// toward "is Store" so Store users are never nagged about updates the Store
/// already delivers.
#[cfg(windows)]
pub fn is_store_install() -> bool {
    use windows::ApplicationModel::{Package, PackageSignatureKind};

    if !crate::sparse_identity::has_package_identity() {
        return false;
    }
    match Package::Current().and_then(|p| p.SignatureKind()) {
        Ok(kind) => kind == PackageSignatureKind::Store,
        Err(_) => true,
    }
}

#[cfg(not(windows))]
pub fn is_store_install() -> bool {
    false
}

/// Pure decision: does this release supersede the running version?
/// Drafts/prereleases never prompt (releases/latest excludes them anyway —
/// kept as defense in depth against API changes).
fn evaluate_release(release: GithubRelease, current: &semver::Version) -> Option<UpdateInfo> {
    if release.draft || release.prerelease {
        return None;
    }
    let remote = semver::Version::parse(release.tag_name.trim().trim_start_matches('v')).ok()?;
    if remote > *current {
        Some(UpdateInfo {
            version: remote.to_string(),
            html_url: release.html_url,
            published_at: release.published_at,
            notes_excerpt: release
                .body
                .map(|b| b.chars().take(300).collect::<String>()),
        })
    } else {
        None
    }
}

async fn fetch_latest_release() -> Option<GithubRelease> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    let response = client
        .get(RELEASES_LATEST_URL)
        .header("User-Agent", concat!("wfdiag/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<GithubRelease>().await.ok()
}

/// Returns Some(update) when a newer GitHub release exists, None otherwise.
/// Silent on every failure mode (offline, rate limit, parse error) — an
/// update check must never surface an error to the user.
#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    if cfg!(debug_assertions) || is_store_install() {
        return Ok(None);
    }
    let Some(release) = fetch_latest_release().await else {
        return Ok(None);
    };
    Ok(evaluate_release(release, &app.package_info().version))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> GithubRelease {
        GithubRelease {
            tag_name: tag.to_string(),
            html_url: "https://github.com/faratech/wfdiag/releases/tag/test".to_string(),
            published_at: Some("2026-06-12T00:00:00Z".to_string()),
            body: Some("notes".to_string()),
            draft: false,
            prerelease: false,
        }
    }

    fn v(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    #[test]
    fn newer_release_prompts() {
        let info = evaluate_release(release("v2.4.0"), &v("2.3.0")).unwrap();
        assert_eq!(info.version, "2.4.0");
    }

    #[test]
    fn same_or_older_release_is_silent() {
        assert!(evaluate_release(release("v2.3.0"), &v("2.3.0")).is_none());
        assert!(evaluate_release(release("v2.2.9"), &v("2.3.0")).is_none());
    }

    #[test]
    fn tag_without_v_prefix_and_whitespace_parses() {
        assert!(evaluate_release(release(" 2.4.1 "), &v("2.3.0")).is_some());
    }

    #[test]
    fn garbage_tag_is_silent_not_fatal() {
        assert!(evaluate_release(release("nightly-build"), &v("2.3.0")).is_none());
    }

    #[test]
    fn drafts_and_prereleases_never_prompt() {
        let mut draft = release("v9.9.9");
        draft.draft = true;
        assert!(evaluate_release(draft, &v("2.3.0")).is_none());
        let mut pre = release("v9.9.9");
        pre.prerelease = true;
        assert!(evaluate_release(pre, &v("2.3.0")).is_none());
    }

    #[test]
    fn notes_excerpt_is_capped() {
        let mut rel = release("v2.4.0");
        rel.body = Some("x".repeat(1000));
        let info = evaluate_release(rel, &v("2.3.0")).unwrap();
        assert_eq!(info.notes_excerpt.unwrap().len(), 300);
    }
}
