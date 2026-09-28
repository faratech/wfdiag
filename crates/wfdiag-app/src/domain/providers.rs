//! Provider-selection gates.
//!
//! Two decisions are pure and belong here: whether on-device AI may be selected
//! as a preference on this PC, and whether a queued AI intent (a chat message
//! or a report) may proceed with the provider status currently known.

use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, AIProviderStatus, parse_provider_preference,
};

/// Whether the requested on-device provider may be chosen right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnDevicePreferenceGate {
    /// The provider probe has not finished.
    Checking,
    /// The selected provider is ready, or is not an on-device provider.
    Ready,
    /// The selected provider cannot be used; includes the user-facing reason.
    Blocked(String),
}

impl OnDevicePreferenceGate {
    /// The reason a selection must be refused, if it must.
    #[must_use]
    pub fn blocking_reason(&self) -> Option<&str> {
        match self {
            Self::Checking => Some(
                "Checking whether on-device AI is available on this PC. Wait for the check to finish before selecting it.",
            ),
            Self::Ready => None,
            Self::Blocked(reason) => Some(reason),
        }
    }

    /// Evaluate the requested preference using that model's own probe result.
    #[must_use]
    pub fn evaluate(preference: &str, status: Option<&AIProviderStatus>, loading: bool) -> Self {
        let preference = parse_provider_preference(preference);
        if !matches!(
            preference,
            AIProviderPreference::AionInstruct | AIProviderPreference::PhiSilica
        ) {
            return Self::Ready;
        }
        if loading {
            return Self::Checking;
        }
        let Some(status) = status else {
            return Self::Checking;
        };
        let (available, ready, message, name) = if preference == AIProviderPreference::AionInstruct
        {
            (
                status.aion_available,
                status.aion_ready,
                &status.aion_message,
                "Aion Instruct",
            )
        } else {
            (
                status.phi_silica_available,
                status.phi_silica_ready,
                &status.phi_silica_message,
                "Phi Silica",
            )
        };
        if available && ready {
            Self::Ready
        } else {
            Self::Blocked(
                message
                    .clone()
                    .unwrap_or_else(|| format!("{name} is unavailable or not ready on this PC.")),
            )
        }
    }

    /// Validate the preference used to evaluate this gate.
    ///
    /// # Errors
    /// Returns the user-facing reason when the selected model is not ready.
    pub fn validate(&self) -> Result<(), String> {
        self.blocking_reason()
            .map_or(Ok(()), |reason| Err(reason.to_string()))
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
    use super::{OnDevicePreferenceGate, PendingAiProviderGate};
    use wfdiag_native_ai_provider::{AIProvider, AIProviderStatus};

    fn status(available: bool, ready: bool, active: AIProvider) -> AIProviderStatus {
        AIProviderStatus {
            preferred_provider: AIProvider::None,
            openai_available: false,
            openai_api_key_set: false,
            phi_silica_available: available,
            phi_silica_ready: ready,
            phi_silica_message: Some("requires the Microsoft Store version".to_string()),
            aion_available: available,
            aion_ready: ready,
            aion_message: Some("requires the Microsoft Store version".to_string()),
            foundry_local_available: false,
            foundry_local_endpoint: None,
            active_provider: active,
            providers: Vec::new(),
        }
    }

    #[test]
    fn phi_cannot_be_selected_before_or_without_a_ready_probe() {
        assert_eq!(
            OnDevicePreferenceGate::evaluate("phi_silica", None, true),
            OnDevicePreferenceGate::Checking
        );
        assert!(
            OnDevicePreferenceGate::evaluate("phi_silica", None, false)
                .validate()
                .is_err()
        );
        let blocked = OnDevicePreferenceGate::evaluate(
            "phi_silica",
            Some(&status(true, false, AIProvider::None)),
            false,
        );
        assert_eq!(
            blocked.validate().unwrap_err(),
            "requires the Microsoft Store version"
        );
        assert!(
            OnDevicePreferenceGate::evaluate("openai", None, true)
                .validate()
                .is_ok(),
            "other providers pass"
        );
    }

    #[test]
    fn phi_passes_only_when_available_and_ready() {
        let gate = OnDevicePreferenceGate::evaluate(
            "phi_silica",
            Some(&status(true, true, AIProvider::PhiSilica)),
            false,
        );
        assert_eq!(gate, OnDevicePreferenceGate::Ready);
        assert!(gate.validate().is_ok());
    }

    #[test]
    fn each_ondevice_preference_requires_its_own_ready_probe() {
        for aion_ready in [false, true] {
            let mut status = status(!aion_ready, !aion_ready, AIProvider::None);
            status.aion_available = aion_ready;
            status.aion_ready = aion_ready;
            for (preference, expected) in [
                ("aion_instruct", aion_ready),
                (" AION ", aion_ready),
                ("phi_silica", !aion_ready),
                ("PHISILICA", !aion_ready),
            ] {
                assert_eq!(
                    OnDevicePreferenceGate::evaluate(preference, Some(&status), false)
                        .validate()
                        .is_ok(),
                    expected
                );
                assert!(
                    OnDevicePreferenceGate::evaluate(preference, Some(&status), true)
                        .validate()
                        .is_err()
                );
                assert!(
                    OnDevicePreferenceGate::evaluate(preference, None, false)
                        .validate()
                        .is_err()
                );
            }
        }
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
