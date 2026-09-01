//! Provider-selection gates.
//!
//! Two decisions are pure and belong here: whether Phi Silica may be selected
//! as a preference on this PC, and whether a queued AI intent (a chat message
//! or a report) may proceed with the provider status currently known.

use wfdiag_native_ai_provider::{AIProvider, AIProviderStatus};

/// Whether Phi Silica may be chosen right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhiPreferenceGate {
    /// The provider probe has not finished.
    Checking,
    /// Phi Silica is available and ready.
    Ready,
    /// Phi Silica cannot be used; the string is the user-facing reason.
    Blocked(String),
}

impl PhiPreferenceGate {
    /// The reason a selection must be refused, if it must.
    #[must_use]
    pub fn blocking_reason(&self) -> Option<&str> {
        match self {
            Self::Checking => Some(
                "Checking whether Phi Silica is available on this PC. Wait for the check to finish before selecting it.",
            ),
            Self::Ready => None,
            Self::Blocked(reason) => Some(reason),
        }
    }

    /// Evaluate the gate from the last provider status.
    #[must_use]
    pub fn evaluate(status: Option<&AIProviderStatus>, loading: bool) -> Self {
        if loading {
            return Self::Checking;
        }
        let Some(status) = status else {
            return Self::Checking;
        };
        if status.phi_silica_available && status.phi_silica_ready {
            Self::Ready
        } else {
            Self::Blocked(status.phi_silica_message.clone().unwrap_or_else(|| {
                "Phi Silica is unavailable or not ready on this PC.".to_string()
            }))
        }
    }

    /// Validate one requested preference string against this gate.
    ///
    /// # Errors
    ///
    /// Returns the user-facing reason when Phi Silica is requested but the
    /// gate is not [`Self::Ready`].
    pub fn validate(&self, preference: &str) -> Result<(), String> {
        if preference.eq_ignore_ascii_case("phi_silica")
            && let Some(reason) = self.blocking_reason()
        {
            return Err(reason.to_string());
        }
        Ok(())
    }
}

/// Whether a queued AI intent may proceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingAiProviderGate {
    /// A usable provider is active.
    Ready,
    /// A status probe is in flight; wait for it.
    Waiting,
    /// No status is known; ask for one.
    Refresh,
    /// AI is switched off in settings.
    Disabled,
    /// A status is known and no provider is usable.
    Unavailable,
}

impl PendingAiProviderGate {
    /// Evaluate the gate from settings and the last provider status.
    #[must_use]
    pub fn evaluate(ai_enabled: bool, loading: bool, status: Option<&AIProviderStatus>) -> Self {
        if !ai_enabled {
            Self::Disabled
        } else if loading {
            Self::Waiting
        } else {
            match status {
                Some(status) if status.active_provider != AIProvider::None => Self::Ready,
                Some(_) => Self::Unavailable,
                None => Self::Refresh,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PendingAiProviderGate, PhiPreferenceGate};
    use wfdiag_native_ai_provider::{AIProvider, AIProviderStatus};

    fn status(available: bool, ready: bool, active: AIProvider) -> AIProviderStatus {
        AIProviderStatus {
            preferred_provider: AIProvider::None,
            openai_available: false,
            openai_api_key_set: false,
            phi_silica_available: available,
            phi_silica_ready: ready,
            phi_silica_message: Some("requires the Microsoft Store version".to_string()),
            foundry_local_available: false,
            foundry_local_endpoint: None,
            active_provider: active,
            providers: Vec::new(),
        }
    }

    #[test]
    fn phi_cannot_be_selected_before_or_without_a_ready_probe() {
        assert_eq!(
            PhiPreferenceGate::evaluate(None, true),
            PhiPreferenceGate::Checking
        );
        assert!(
            PhiPreferenceGate::evaluate(None, false)
                .validate("phi_silica")
                .is_err()
        );
        let blocked =
            PhiPreferenceGate::evaluate(Some(&status(true, false, AIProvider::None)), false);
        assert_eq!(
            blocked.validate("phi_silica").unwrap_err(),
            "requires the Microsoft Store version"
        );
        assert!(blocked.validate("openai").is_ok(), "other providers pass");
    }

    #[test]
    fn phi_passes_only_when_available_and_ready() {
        let gate =
            PhiPreferenceGate::evaluate(Some(&status(true, true, AIProvider::PhiSilica)), false);
        assert_eq!(gate, PhiPreferenceGate::Ready);
        assert!(gate.validate("PHI_SILICA").is_ok());
    }

    #[test]
    fn a_queued_intent_waits_refreshes_or_gives_up() {
        assert_eq!(
            PendingAiProviderGate::evaluate(false, false, None),
            PendingAiProviderGate::Disabled
        );
        assert_eq!(
            PendingAiProviderGate::evaluate(true, true, None),
            PendingAiProviderGate::Waiting
        );
        assert_eq!(
            PendingAiProviderGate::evaluate(true, false, None),
            PendingAiProviderGate::Refresh
        );
        assert_eq!(
            PendingAiProviderGate::evaluate(
                true,
                false,
                Some(&status(false, false, AIProvider::None))
            ),
            PendingAiProviderGate::Unavailable
        );
        assert_eq!(
            PendingAiProviderGate::evaluate(
                true,
                false,
                Some(&status(false, false, AIProvider::Ollama))
            ),
            PendingAiProviderGate::Ready
        );
    }
}
