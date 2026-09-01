//! Closed external destinations and locale-string carriers for export/share.
//!
//! A rendered report or a component message must never be able to turn a
//! shell's "open this" helper into an arbitrary URL launcher, so the set of
//! reachable destinations is an enum resolved here rather than a string passed
//! across the UI boundary. Actually launching the target (and asking Windows
//! for the user's locale formats) stays in the host shell.

const WINDOWSFORUM_NEW_THREAD_URL: &str =
    "https://windowsforum.com/forums/windows-help-and-support.302/post-thread";

/// Closed external actions exposed by the export/share surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportExternalAction {
    WindowsForumNewThread,
}

/// Resolve a typed export action to its single trusted HTTPS destination.
#[must_use]
pub const fn resolve_export_external_url(action: ExportExternalAction) -> &'static str {
    match action {
        ExportExternalAction::WindowsForumNewThread => WINDOWSFORUM_NEW_THREAD_URL,
    }
}

/// The two locale-sensitive values consumed by
/// [`ExportMetadata`](crate::ExportMetadata).
///
/// `generated` corresponds to JavaScript's parameterless
/// `Date.prototype.toLocaleString()`. `local_date` corresponds to
/// `Date.prototype.toLocaleDateString()`. Producing them requires the user's
/// language, region, calendar, and clock preferences, so the host resolves the
/// strings and hands this carrier back to the renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportDateStrings {
    pub generated: String,
    pub local_date: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_windowsforum_compose_target_is_resolvable() {
        assert_eq!(
            resolve_export_external_url(ExportExternalAction::WindowsForumNewThread),
            "https://windowsforum.com/forums/windows-help-and-support.302/post-thread"
        );
    }

    #[test]
    fn the_resolved_target_is_an_https_windowsforum_url() {
        let url = resolve_export_external_url(ExportExternalAction::WindowsForumNewThread);
        assert!(url.starts_with("https://windowsforum.com/"), "{url}");
    }
}
