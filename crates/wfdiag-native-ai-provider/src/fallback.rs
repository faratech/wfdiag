//! Snapshot-based provider fallback planning.
//!
//! This module is deliberately free of probes and UI policy. Callers capture
//! one [`ProviderAvailability`] value, build candidates from that immutable
//! snapshot, and decide how to present or persist cloud-fallback consent.

use crate::{AIProvider, AIProviderPreference, ProviderAvailability};

/// Canonical provider order for an `Auto` request.
///
/// `None` is a status sentinel and is intentionally absent: every value in
/// this array is an executable provider candidate.
pub const AUTO_FALLBACK_ORDER: [AIProvider; 10] = [
    AIProvider::PhiSilica,
    AIProvider::FoundryLocal,
    AIProvider::Ollama,
    AIProvider::CustomOpenAI,
    AIProvider::CodexCli,
    AIProvider::ClaudeCode,
    AIProvider::OpenAI,
    AIProvider::Anthropic,
    AIProvider::Gemini,
    AIProvider::DeepSeek,
];

/// Provider trust zone used to detect the local-to-cloud consent boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTrustZone {
    /// Execution stays on the device or a purpose-built local model server.
    Local,
    /// Execution can send request content to a custom, subscription, or API service.
    Cloud,
}

/// One available and untried provider selected from a captured snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FallbackCandidate {
    /// Provider to try next. This is never [`AIProvider::None`].
    pub provider: AIProvider,
    /// Trust zone of `provider`.
    pub trust_zone: ProviderTrustZone,
    /// Whether moving from the current provider to this candidate crosses
    /// from local execution into cloud execution.
    pub crosses_local_to_cloud: bool,
}

impl ProviderAvailability {
    /// Whether this immutable status snapshot marks `provider` executable.
    ///
    /// [`AIProvider::None`] is a status sentinel and is always unavailable.
    #[must_use]
    pub const fn contains(self, provider: AIProvider) -> bool {
        match provider {
            AIProvider::None => false,
            AIProvider::PhiSilica => self.phi,
            AIProvider::FoundryLocal => self.foundry,
            AIProvider::Ollama => self.ollama,
            AIProvider::CustomOpenAI => self.custom,
            AIProvider::CodexCli => self.codex,
            AIProvider::ClaudeCode => self.claude,
            AIProvider::OpenAI => self.openai,
            AIProvider::Anthropic => self.anthropic,
            AIProvider::Gemini => self.gemini,
            AIProvider::DeepSeek => self.deepseek,
        }
    }
}

/// Classify a provider for local-to-cloud fallback consent.
///
/// Custom `OpenAI` endpoints are conservatively classified as cloud because
/// their URL is user-controlled and may leave the device. Subscription CLIs
/// are also cloud even though their process bridge runs locally.
#[must_use]
pub const fn provider_trust_zone(provider: AIProvider) -> Option<ProviderTrustZone> {
    match provider {
        AIProvider::None => None,
        AIProvider::PhiSilica | AIProvider::FoundryLocal | AIProvider::Ollama => {
            Some(ProviderTrustZone::Local)
        }
        AIProvider::CustomOpenAI
        | AIProvider::CodexCli
        | AIProvider::ClaudeCode
        | AIProvider::OpenAI
        | AIProvider::Anthropic
        | AIProvider::Gemini
        | AIProvider::DeepSeek => Some(ProviderTrustZone::Cloud),
    }
}

/// Whether a provider transition is the exact boundary guarded by the
/// Ask/Never/Allow cloud-fallback policy.
#[must_use]
pub const fn crosses_local_to_cloud(from: AIProvider, to: AIProvider) -> bool {
    matches!(
        (provider_trust_zone(from), provider_trust_zone(to)),
        (
            Some(ProviderTrustZone::Local),
            Some(ProviderTrustZone::Cloud)
        )
    )
}

const fn explicit_provider(preference: AIProviderPreference) -> Option<AIProvider> {
    match preference {
        AIProviderPreference::Auto => None,
        AIProviderPreference::OpenAI => Some(AIProvider::OpenAI),
        AIProviderPreference::PhiSilica => Some(AIProvider::PhiSilica),
        AIProviderPreference::FoundryLocal => Some(AIProvider::FoundryLocal),
        AIProviderPreference::Ollama => Some(AIProvider::Ollama),
        AIProviderPreference::CustomOpenAI => Some(AIProvider::CustomOpenAI),
        AIProviderPreference::CodexCli => Some(AIProvider::CodexCli),
        AIProviderPreference::ClaudeCode => Some(AIProvider::ClaudeCode),
        AIProviderPreference::Anthropic => Some(AIProvider::Anthropic),
        AIProviderPreference::Gemini => Some(AIProvider::Gemini),
        AIProviderPreference::DeepSeek => Some(AIProvider::DeepSeek),
    }
}

/// Build all executable candidates from one availability snapshot.
///
/// `Auto` returns every available provider in [`AUTO_FALLBACK_ORDER`]. An
/// explicit preference returns either that one provider or an empty plan;
/// explicit requests never inherit another provider as a fallback.
#[must_use]
pub fn provider_fallback_plan(
    preference: AIProviderPreference,
    availability: ProviderAvailability,
) -> Vec<AIProvider> {
    if preference == AIProviderPreference::Auto {
        return AUTO_FALLBACK_ORDER
            .into_iter()
            .filter(|provider| availability.contains(*provider))
            .collect();
    }

    explicit_provider(preference)
        .filter(|provider| availability.contains(*provider))
        .into_iter()
        .collect()
}

/// Produce the next available provider absent from `tried`.
///
/// Duplicate entries and [`AIProvider::None`] in `tried` are harmless. The
/// returned boundary bit is false for an initial selection and true only for
/// a transition from a local `current` provider to a cloud candidate.
#[must_use]
pub fn next_fallback_candidate(
    preference: AIProviderPreference,
    current: Option<AIProvider>,
    tried: &[AIProvider],
    availability: ProviderAvailability,
) -> Option<FallbackCandidate> {
    let provider = provider_fallback_plan(preference, availability)
        .into_iter()
        .find(|provider| !tried.contains(provider))?;
    let trust_zone = provider_trust_zone(provider)?;
    Some(FallbackCandidate {
        provider,
        trust_zone,
        crosses_local_to_cloud: current.is_some_and(|from| crosses_local_to_cloud(from, provider)),
    })
}

/// Resolve the first provider while preserving the established status API.
#[must_use]
pub const fn route_provider(
    preference: AIProviderPreference,
    availability: ProviderAvailability,
) -> AIProvider {
    if let Some(provider) = explicit_provider(preference) {
        return if availability.contains(provider) {
            provider
        } else {
            AIProvider::None
        };
    }

    let mut index = 0;
    while index < AUTO_FALLBACK_ORDER.len() {
        let provider = AUTO_FALLBACK_ORDER[index];
        if availability.contains(provider) {
            return provider;
        }
        index += 1;
    }
    AIProvider::None
}

/// Compatibility helper for request paths that may retry locally but do not
/// implement a cloud-consent flow.
#[must_use]
pub fn next_auto_local_route(
    preference: AIProviderPreference,
    tried: &[AIProvider],
    availability: ProviderAvailability,
) -> Option<AIProvider> {
    if preference != AIProviderPreference::Auto {
        return None;
    }
    provider_fallback_plan(preference, availability)
        .into_iter()
        .take_while(|provider| provider_trust_zone(*provider) == Some(ProviderTrustZone::Local))
        .find(|provider| !tried.contains(provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_AVAILABLE: ProviderAvailability = ProviderAvailability {
        phi: true,
        foundry: true,
        ollama: true,
        custom: true,
        codex: true,
        claude: true,
        openai: true,
        anthropic: true,
        gemini: true,
        deepseek: true,
    };

    const EXPLICIT_PAIRS: [(AIProviderPreference, AIProvider); 10] = [
        (AIProviderPreference::PhiSilica, AIProvider::PhiSilica),
        (AIProviderPreference::FoundryLocal, AIProvider::FoundryLocal),
        (AIProviderPreference::Ollama, AIProvider::Ollama),
        (AIProviderPreference::CustomOpenAI, AIProvider::CustomOpenAI),
        (AIProviderPreference::CodexCli, AIProvider::CodexCli),
        (AIProviderPreference::ClaudeCode, AIProvider::ClaudeCode),
        (AIProviderPreference::OpenAI, AIProvider::OpenAI),
        (AIProviderPreference::Anthropic, AIProvider::Anthropic),
        (AIProviderPreference::Gemini, AIProvider::Gemini),
        (AIProviderPreference::DeepSeek, AIProvider::DeepSeek),
    ];

    fn availability_for(provider: AIProvider) -> ProviderAvailability {
        let mut availability = ProviderAvailability::default();
        match provider {
            AIProvider::None => {}
            AIProvider::PhiSilica => availability.phi = true,
            AIProvider::FoundryLocal => availability.foundry = true,
            AIProvider::Ollama => availability.ollama = true,
            AIProvider::CustomOpenAI => availability.custom = true,
            AIProvider::CodexCli => availability.codex = true,
            AIProvider::ClaudeCode => availability.claude = true,
            AIProvider::OpenAI => availability.openai = true,
            AIProvider::Anthropic => availability.anthropic = true,
            AIProvider::Gemini => availability.gemini = true,
            AIProvider::DeepSeek => availability.deepseek = true,
        }
        availability
    }

    #[test]
    fn auto_plan_is_exhaustive_ordered_and_never_contains_none() {
        assert_eq!(
            provider_fallback_plan(AIProviderPreference::Auto, ALL_AVAILABLE),
            AUTO_FALLBACK_ORDER
        );
        assert!(!AUTO_FALLBACK_ORDER.contains(&AIProvider::None));
        for (index, provider) in AUTO_FALLBACK_ORDER.iter().enumerate() {
            assert_eq!(
                AUTO_FALLBACK_ORDER
                    .iter()
                    .filter(|candidate| *candidate == provider)
                    .count(),
                1,
                "provider at index {index} must occur exactly once"
            );
        }
    }

    #[test]
    fn auto_plan_filters_only_the_captured_availability_snapshot() {
        for provider in AUTO_FALLBACK_ORDER {
            assert_eq!(
                provider_fallback_plan(AIProviderPreference::Auto, availability_for(provider)),
                vec![provider]
            );
        }
        let availability = ProviderAvailability {
            foundry: true,
            claude: true,
            deepseek: true,
            ..ProviderAvailability::default()
        };
        assert_eq!(
            provider_fallback_plan(AIProviderPreference::Auto, availability),
            vec![
                AIProvider::FoundryLocal,
                AIProvider::ClaudeCode,
                AIProvider::DeepSeek
            ]
        );
    }

    #[test]
    fn every_explicit_preference_is_single_provider_and_never_falls_back() {
        for (preference, provider) in EXPLICIT_PAIRS {
            assert_eq!(
                provider_fallback_plan(preference, ALL_AVAILABLE),
                vec![provider]
            );
            let another_available = AUTO_FALLBACK_ORDER
                .into_iter()
                .find(|candidate| *candidate != provider)
                .expect("there is another provider");
            assert!(
                provider_fallback_plan(preference, availability_for(another_available)).is_empty()
            );
        }
    }

    #[test]
    fn next_candidate_skips_deduplicated_tried_values() {
        let candidate = next_fallback_candidate(
            AIProviderPreference::Auto,
            Some(AIProvider::PhiSilica),
            &[
                AIProvider::None,
                AIProvider::PhiSilica,
                AIProvider::PhiSilica,
                AIProvider::FoundryLocal,
            ],
            ALL_AVAILABLE,
        )
        .expect("Ollama remains available");
        assert_eq!(candidate.provider, AIProvider::Ollama);
        assert_eq!(candidate.trust_zone, ProviderTrustZone::Local);
        assert!(!candidate.crosses_local_to_cloud);

        assert!(
            next_fallback_candidate(
                AIProviderPreference::Gemini,
                Some(AIProvider::Gemini),
                &[AIProvider::Gemini, AIProvider::Gemini],
                ALL_AVAILABLE,
            )
            .is_none()
        );
    }

    #[test]
    fn trust_zone_and_local_to_cloud_boundary_are_exhaustive() {
        for provider in [
            AIProvider::PhiSilica,
            AIProvider::FoundryLocal,
            AIProvider::Ollama,
        ] {
            assert_eq!(
                provider_trust_zone(provider),
                Some(ProviderTrustZone::Local)
            );
        }
        for provider in [
            AIProvider::CustomOpenAI,
            AIProvider::CodexCli,
            AIProvider::ClaudeCode,
            AIProvider::OpenAI,
            AIProvider::Anthropic,
            AIProvider::Gemini,
            AIProvider::DeepSeek,
        ] {
            assert_eq!(
                provider_trust_zone(provider),
                Some(ProviderTrustZone::Cloud)
            );
        }
        assert_eq!(provider_trust_zone(AIProvider::None), None);

        assert!(crosses_local_to_cloud(
            AIProvider::Ollama,
            AIProvider::CustomOpenAI
        ));
        assert!(!crosses_local_to_cloud(
            AIProvider::PhiSilica,
            AIProvider::FoundryLocal
        ));
        assert!(!crosses_local_to_cloud(
            AIProvider::CustomOpenAI,
            AIProvider::CodexCli
        ));
        assert!(!crosses_local_to_cloud(
            AIProvider::OpenAI,
            AIProvider::FoundryLocal
        ));
        assert!(!crosses_local_to_cloud(
            AIProvider::None,
            AIProvider::OpenAI
        ));
    }

    #[test]
    fn next_candidate_marks_only_an_actual_local_to_cloud_transition() {
        let tried_local = [
            AIProvider::PhiSilica,
            AIProvider::FoundryLocal,
            AIProvider::Ollama,
        ];
        let crossing = next_fallback_candidate(
            AIProviderPreference::Auto,
            Some(AIProvider::Ollama),
            &tried_local,
            ALL_AVAILABLE,
        )
        .expect("custom provider is the first cloud candidate");
        assert_eq!(crossing.provider, AIProvider::CustomOpenAI);
        assert_eq!(crossing.trust_zone, ProviderTrustZone::Cloud);
        assert!(crossing.crosses_local_to_cloud);

        let initial_cloud = next_fallback_candidate(
            AIProviderPreference::Auto,
            None,
            &[],
            availability_for(AIProvider::OpenAI),
        )
        .expect("OpenAI is available");
        assert!(!initial_cloud.crosses_local_to_cloud);

        let within_cloud = next_fallback_candidate(
            AIProviderPreference::Auto,
            Some(AIProvider::CustomOpenAI),
            &[
                AIProvider::PhiSilica,
                AIProvider::FoundryLocal,
                AIProvider::Ollama,
                AIProvider::CustomOpenAI,
            ],
            ALL_AVAILABLE,
        )
        .expect("Codex is next");
        assert_eq!(within_cloud.provider, AIProvider::CodexCli);
        assert!(!within_cloud.crosses_local_to_cloud);
    }

    #[test]
    fn empty_snapshot_and_exhausted_plan_have_no_candidate() {
        assert!(
            provider_fallback_plan(AIProviderPreference::Auto, ProviderAvailability::default())
                .is_empty()
        );
        assert!(
            next_fallback_candidate(
                AIProviderPreference::Auto,
                None,
                &AUTO_FALLBACK_ORDER,
                ALL_AVAILABLE,
            )
            .is_none()
        );
        assert!(
            next_fallback_candidate(
                AIProviderPreference::OpenAI,
                None,
                &[],
                ProviderAvailability::default(),
            )
            .is_none()
        );
    }
}
