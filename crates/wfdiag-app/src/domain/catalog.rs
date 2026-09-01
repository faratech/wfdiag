//! The live model-catalog refresh policy.
//!
//! Provider setup asks for a model list on every keystroke that could change
//! the answer — an API key, an endpoint, a CLI path, a provider selection. The
//! discovery worker admits exactly one request at a time and each one is a real
//! network call or a spawned CLI, so the policy here is a **debounce with a
//! cancel-and-retry latch**, not a rate limiter:
//!
//! * a request inside the debounce window replaces the pending one;
//! * a request while one is already in flight cancels it and re-issues once the
//!   cancellation lands;
//! * a failed refresh keeps the last catalog visible, flagged stale, instead of
//!   blanking a list the user was reading.

use std::time::{Duration, Instant};
use wfdiag_native_ai_provider::{AIProvider, ModelCatalog};

/// How long provider-setup edits are coalesced before discovery runs.
pub const REFRESH_DEBOUNCE: Duration = Duration::from_millis(400);

/// What one provider's catalog looks like to a host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogState {
    /// The last catalog that loaded, if any.
    pub catalog: Option<ModelCatalog>,
    /// A refresh is in flight.
    pub loading: bool,
    /// The last refresh failure.
    pub error: Option<String>,
    /// Why discovery cannot even be attempted (no key, no endpoint).
    pub blocked: Option<String>,
    /// Whether `catalog` predates the current inputs.
    pub stale: bool,
}

impl CatalogState {
    /// Record a successful refresh.
    pub fn loaded(&mut self, catalog: ModelCatalog) {
        self.catalog = Some(catalog);
        self.loading = false;
        self.error = None;
        self.blocked = None;
        self.stale = false;
    }

    /// Record a failure, keeping any catalog already on screen.
    pub fn failed(&mut self, error: impl Into<String>) {
        self.loading = false;
        self.blocked = None;
        self.error = Some(error.into());
        self.stale = self.catalog.is_some();
    }

    /// Record that discovery cannot run with the current inputs.
    pub fn blocked(&mut self, reason: impl Into<String>) {
        self.loading = false;
        self.error = None;
        self.blocked = Some(reason.into());
        self.stale = self.catalog.is_some();
    }
}

/// What a refresh request should do right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshDecision {
    /// Issue the request.
    Refresh,
    /// One ran inside the debounce window; keep the catalog on record.
    Throttled,
    /// Another request is in flight: cancel it and re-issue afterwards.
    CancelAndRetry,
}

/// The debounce clock plus the in-flight latch.
#[derive(Debug, Default)]
pub struct RefreshThrottle {
    last_started: Option<Instant>,
    in_flight: bool,
    retry_after_cancel: bool,
}

impl RefreshThrottle {
    /// A throttle that has never refreshed.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_started: None,
            in_flight: false,
            retry_after_cancel: false,
        }
    }

    /// Whether a refresh is in flight.
    #[must_use]
    pub const fn in_flight(&self) -> bool {
        self.in_flight
    }

    /// Whether a cancelled refresh should be re-issued.
    #[must_use]
    pub const fn retry_pending(&self) -> bool {
        self.retry_after_cancel
    }

    /// Decide what a refresh request at `now` should do.
    ///
    /// `forced` is a user-initiated refresh, which is never throttled — the
    /// debounce exists to absorb typing, not to refuse the Refresh button.
    #[must_use]
    pub fn decide(&self, now: Instant, forced: bool) -> RefreshDecision {
        if self.in_flight {
            return RefreshDecision::CancelAndRetry;
        }
        if forced {
            return RefreshDecision::Refresh;
        }
        match self.last_started {
            Some(last) if now.duration_since(last) < REFRESH_DEBOUNCE => RefreshDecision::Throttled,
            _ => RefreshDecision::Refresh,
        }
    }

    /// Record that a refresh was issued at `now`.
    pub fn started(&mut self, now: Instant) {
        self.last_started = Some(now);
        self.in_flight = true;
        self.retry_after_cancel = false;
    }

    /// Arm the cancel-and-retry latch.
    pub const fn request_retry(&mut self) {
        self.retry_after_cancel = true;
    }

    /// Clear the latch without re-issuing (an explicit cancel).
    pub const fn clear_retry(&mut self) {
        self.retry_after_cancel = false;
    }

    /// Record that the in-flight refresh finished, and report whether a retry
    /// was latched. The latch is consumed.
    pub const fn finished(&mut self) -> bool {
        self.in_flight = false;
        let retry = self.retry_after_cancel;
        self.retry_after_cancel = false;
        retry
    }
}

/// Whether a provider's catalog may be discovered automatically.
///
/// Phi Silica has no catalog at all — the model is the operating system's.
/// Claude Code is excluded because its catalog probe runs `npx -y`, which can
/// download and cache a package: a material side effect that belongs behind an
/// explicit Refresh, not behind a keystroke.
#[must_use]
pub const fn auto_discovery_allowed(provider: AIProvider) -> bool {
    !matches!(provider, AIProvider::PhiSilica | AIProvider::ClaudeCode)
}

#[cfg(test)]
mod tests {
    use super::{
        AIProvider, CatalogState, REFRESH_DEBOUNCE, RefreshDecision, RefreshThrottle,
        auto_discovery_allowed,
    };
    use std::time::{Duration, Instant};
    use wfdiag_native_ai_provider::{ModelCatalog, ModelCatalogEntry};

    fn catalog(id: &str) -> ModelCatalog {
        ModelCatalog {
            models: vec![ModelCatalogEntry::from_id(id)],
            default_model: Some(id.to_string()),
        }
    }

    #[test]
    fn typing_is_debounced_but_the_refresh_button_never_is() {
        let mut throttle = RefreshThrottle::new();
        let start = Instant::now();
        assert_eq!(throttle.decide(start, false), RefreshDecision::Refresh);
        throttle.started(start);
        assert!(throttle.in_flight());
        assert!(!throttle.finished());

        let soon = start + REFRESH_DEBOUNCE / 2;
        assert_eq!(throttle.decide(soon, false), RefreshDecision::Throttled);
        assert_eq!(
            throttle.decide(soon, true),
            RefreshDecision::Refresh,
            "an explicit refresh is never throttled"
        );
        let later = start + REFRESH_DEBOUNCE + Duration::from_millis(1);
        assert_eq!(throttle.decide(later, false), RefreshDecision::Refresh);
    }

    #[test]
    fn a_second_request_cancels_the_first_and_is_re_issued_once_it_lands() {
        let mut throttle = RefreshThrottle::new();
        let start = Instant::now();
        throttle.started(start);
        assert_eq!(
            throttle.decide(start, true),
            RefreshDecision::CancelAndRetry,
            "the worker admits one request at a time"
        );
        throttle.request_retry();
        assert!(throttle.retry_pending());
        assert!(throttle.finished(), "the latched retry is reported once");
        assert!(!throttle.retry_pending());
        assert!(!throttle.in_flight());
    }

    #[test]
    fn an_explicit_cancel_clears_the_retry_latch() {
        let mut throttle = RefreshThrottle::new();
        throttle.started(Instant::now());
        throttle.request_retry();
        throttle.clear_retry();
        assert!(!throttle.finished());
    }

    #[test]
    fn a_failed_refresh_keeps_the_last_catalog_visible_and_flags_it_stale() {
        let mut state = CatalogState::default();
        state.failed("offline");
        assert!(!state.stale, "there was nothing to keep");

        state.loaded(catalog("llama3"));
        assert!(!state.stale && state.error.is_none());
        state.failed("offline");
        assert!(state.stale, "the last good list stays on screen");
        assert_eq!(state.error.as_deref(), Some("offline"));
        assert!(state.catalog.is_some());

        state.blocked("Enter an API key to load the available models.");
        assert!(state.error.is_none() && state.blocked.is_some() && state.stale);
    }

    #[test]
    fn providers_with_material_probe_side_effects_are_never_auto_discovered() {
        assert!(!auto_discovery_allowed(AIProvider::PhiSilica));
        assert!(!auto_discovery_allowed(AIProvider::ClaudeCode));
        assert!(auto_discovery_allowed(AIProvider::Ollama));
        assert!(auto_discovery_allowed(AIProvider::OpenAI));
    }
}
