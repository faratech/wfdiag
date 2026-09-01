//! The local-to-cloud fallback consent decision.
//!
//! An `Auto` chat turn that fails cleanly on a local provider may continue on
//! the next available one. When that next provider is a cloud provider the
//! move crosses a trust boundary the user owns, so the decision is gated by
//! the persisted [`CloudFallbackPolicy`].
//!
//! Everything here is pure: the boundary itself comes from
//! [`wfdiag_native_ai_provider::next_fallback_candidate`], and this module only
//! decides what to do about it. The service performs the I/O (queue the retry,
//! persist the preference, emit the prompt).

use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, FallbackCandidate, ProviderAvailability,
    next_fallback_candidate,
};
use wfdiag_native_settings::CloudFallbackPolicy;

/// One logical chat turn, carried across every physical attempt.
///
/// The prompt and the tool evidence are captured **once**, when the turn is
/// first sent: a fallback retry must answer the same question against the same
/// evidence, not a re-read of state that moved on in the meantime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatAttempt {
    /// The turn's stable identity, unchanged by fallback.
    pub turn: u64,
    /// The user's prompt.
    pub prompt: String,
    /// The provider preference this turn was planned with.
    pub preference: AIProviderPreference,
    /// The availability snapshot this turn was planned with.
    pub availability: ProviderAvailability,
    /// Every provider already attempted, in order.
    pub tried: Vec<AIProvider>,
    /// The provider the turn is attributed to.
    pub initial_provider: AIProvider,
    /// The provider running now.
    pub current_provider: AIProvider,
    /// The first attempt's failure, which is the one worth showing.
    pub first_failure: Option<String>,
}

impl ChatAttempt {
    /// Plan the first attempt, or `None` when no provider is available.
    #[must_use]
    pub fn plan(
        turn: u64,
        prompt: String,
        preference: AIProviderPreference,
        availability: ProviderAvailability,
    ) -> Option<Self> {
        let candidate = next_fallback_candidate(preference, None, &[], availability)?;
        Some(Self {
            turn,
            prompt,
            preference,
            availability,
            tried: vec![candidate.provider],
            initial_provider: candidate.provider,
            current_provider: candidate.provider,
            first_failure: None,
        })
    }

    /// The provider this attempt should be attributed back to, if any.
    #[must_use]
    pub fn fallback_from(&self) -> Option<AIProvider> {
        (self.current_provider != self.initial_provider).then_some(self.initial_provider)
    }

    /// Whether a further clean failure could still be retried elsewhere.
    #[must_use]
    pub fn allows_fallback(&self) -> bool {
        next_fallback_candidate(
            self.preference,
            Some(self.current_provider),
            &self.tried,
            self.availability,
        )
        .is_some()
    }

    /// The next candidate after a clean failure.
    #[must_use]
    pub fn next_candidate(&self) -> Option<FallbackCandidate> {
        next_fallback_candidate(
            self.preference,
            Some(self.current_provider),
            &self.tried,
            self.availability,
        )
    }

    /// Move this attempt onto `provider`.
    pub fn advance_to(&mut self, provider: AIProvider) {
        self.current_provider = provider;
        self.tried.push(provider);
    }

    /// Record the first failure only; later ones describe a provider the user
    /// never chose.
    pub fn record_failure(&mut self, message: String) {
        if self.first_failure.is_none() {
            self.first_failure = Some(message);
        }
    }
}

/// What to do after a clean, retryable chat failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FallbackDecision {
    /// Retry immediately on `provider`; no consent is needed.
    Continue {
        /// The provider to try next.
        provider: AIProvider,
    },
    /// The move crosses into cloud execution and the policy is `Ask`.
    Prompt {
        /// The provider that would run next.
        provider: AIProvider,
        /// The local provider's failure, shown with the prompt.
        reason: String,
    },
    /// The policy forbids cloud fallback; the turn ends here.
    Refuse {
        /// The user-facing failure.
        message: String,
    },
    /// No provider is left to try.
    Exhausted {
        /// The user-facing failure.
        message: String,
    },
}

/// Decide what a clean failure on `attempt` leads to under `policy`.
#[must_use]
pub fn decide_fallback(attempt: &ChatAttempt, policy: CloudFallbackPolicy) -> FallbackDecision {
    let Some(candidate) = attempt.next_candidate() else {
        return FallbackDecision::Exhausted {
            message: attempt
                .first_failure
                .clone()
                .unwrap_or_else(|| "No fallback provider is available".to_string()),
        };
    };
    if !candidate.crosses_local_to_cloud {
        return FallbackDecision::Continue {
            provider: candidate.provider,
        };
    }
    let reason = attempt.first_failure.clone().unwrap_or_default();
    match policy {
        CloudFallbackPolicy::Allow => FallbackDecision::Continue {
            provider: candidate.provider,
        },
        CloudFallbackPolicy::Never => FallbackDecision::Refuse {
            message: format!("{reason} Cloud fallback is disabled in Settings."),
        },
        CloudFallbackPolicy::Ask => FallbackDecision::Prompt {
            provider: candidate.provider,
            reason,
        },
    }
}

/// A prompt awaiting the user's answer, plus the turn it will resume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingConsent {
    /// The attempt to resume once the answer arrives.
    pub attempt: ChatAttempt,
    /// The provider the user is being asked about.
    pub candidate: AIProvider,
    /// The local provider's failure.
    pub reason: String,
}

/// What a host's answer to a consent prompt means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentAnswer {
    /// Continue on the cloud provider and remember that.
    Allow,
    /// Refuse, and remember that too.
    Never,
}

impl ConsentAnswer {
    /// The answer as a persisted policy.
    #[must_use]
    pub const fn policy(self) -> CloudFallbackPolicy {
        match self {
            Self::Allow => CloudFallbackPolicy::Allow,
            Self::Never => CloudFallbackPolicy::Never,
        }
    }

    /// Answer a boolean host decision.
    #[must_use]
    pub const fn from_allow(allow: bool) -> Self {
        if allow { Self::Allow } else { Self::Never }
    }
}

/// A consent answer whose preference write is still in flight.
///
/// The answer is applied to the turn only after the write lands, so a failed
/// write re-arms the prompt instead of silently losing the pending message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPolicyWrite {
    /// The policy being persisted.
    pub policy: CloudFallbackPolicy,
    /// The consent the write belongs to.
    pub consent: PendingConsent,
}

/// What a completed preference write means for the waiting turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyWriteOutcome {
    /// Resume the turn on the cloud candidate.
    Continue(Box<PendingConsent>),
    /// End the turn.
    Refuse {
        /// The consent, so the host can attribute the failure.
        consent: Box<PendingConsent>,
        /// The user-facing failure.
        message: String,
    },
    /// Show the prompt again (the write failed, or the policy is still `Ask`).
    Reprompt(Box<PendingConsent>),
}

/// Interpret the settings reply for a pending consent write.
#[must_use]
pub fn apply_policy_write(
    pending: PendingPolicyWrite,
    persisted: Result<CloudFallbackPolicy, ()>,
) -> PolicyWriteOutcome {
    let PendingPolicyWrite { policy, consent } = pending;
    if persisted.is_err() {
        return PolicyWriteOutcome::Reprompt(Box::new(consent));
    }
    match policy {
        CloudFallbackPolicy::Allow => PolicyWriteOutcome::Continue(Box::new(consent)),
        CloudFallbackPolicy::Never => PolicyWriteOutcome::Refuse {
            consent: Box::new(consent),
            message: "The local provider failed, and cloud fallback was declined.".to_string(),
        },
        CloudFallbackPolicy::Ask => PolicyWriteOutcome::Reprompt(Box::new(consent)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatAttempt, ConsentAnswer, FallbackDecision, PendingConsent, PendingPolicyWrite,
        PolicyWriteOutcome, apply_policy_write, decide_fallback,
    };
    use wfdiag_native_ai_provider::{AIProvider, AIProviderPreference, ProviderAvailability};
    use wfdiag_native_settings::CloudFallbackPolicy;

    fn availability() -> ProviderAvailability {
        ProviderAvailability {
            ollama: true,
            openai: true,
            ..ProviderAvailability::default()
        }
    }

    fn attempt() -> ChatAttempt {
        ChatAttempt::plan(
            1,
            "why is my disk full".to_string(),
            AIProviderPreference::Auto,
            availability(),
        )
        .expect("a provider is available")
    }

    #[test]
    fn auto_starts_on_the_first_local_provider_and_keeps_attribution() {
        let attempt = attempt();
        assert_eq!(attempt.current_provider, AIProvider::Ollama);
        assert_eq!(attempt.fallback_from(), None);
        assert!(attempt.allows_fallback(), "OpenAI is still untried");
    }

    #[test]
    fn a_local_to_cloud_move_is_gated_by_the_persisted_policy() {
        let mut failed = attempt();
        failed.record_failure("Ollama is not running".to_string());

        assert_eq!(
            decide_fallback(&failed, CloudFallbackPolicy::Allow),
            FallbackDecision::Continue {
                provider: AIProvider::OpenAI
            }
        );
        assert_eq!(
            decide_fallback(&failed, CloudFallbackPolicy::Ask),
            FallbackDecision::Prompt {
                provider: AIProvider::OpenAI,
                reason: "Ollama is not running".to_string(),
            }
        );
        assert_eq!(
            decide_fallback(&failed, CloudFallbackPolicy::Never),
            FallbackDecision::Refuse {
                message: "Ollama is not running Cloud fallback is disabled in Settings."
                    .to_string(),
            }
        );
    }

    #[test]
    fn only_the_first_failure_is_kept_and_exhaustion_reports_it() {
        let mut failed = attempt();
        failed.record_failure("first".to_string());
        failed.record_failure("second".to_string());
        failed.advance_to(AIProvider::OpenAI);
        assert_eq!(failed.fallback_from(), Some(AIProvider::Ollama));
        assert!(!failed.allows_fallback());
        assert_eq!(
            decide_fallback(&failed, CloudFallbackPolicy::Allow),
            FallbackDecision::Exhausted {
                message: "first".to_string()
            }
        );
    }

    #[test]
    fn a_cloud_to_cloud_move_never_prompts() {
        let availability = ProviderAvailability {
            openai: true,
            anthropic: true,
            ..ProviderAvailability::default()
        };
        let mut attempt = ChatAttempt::plan(
            2,
            "hello".to_string(),
            AIProviderPreference::Auto,
            availability,
        )
        .expect("openai is available");
        attempt.record_failure("quota".to_string());
        assert_eq!(
            decide_fallback(&attempt, CloudFallbackPolicy::Ask),
            FallbackDecision::Continue {
                provider: AIProvider::Anthropic
            },
            "the consent boundary is local-to-cloud, not every provider change"
        );
    }

    #[test]
    fn an_explicit_preference_never_falls_back() {
        let attempt = ChatAttempt::plan(
            3,
            "hello".to_string(),
            AIProviderPreference::Ollama,
            availability(),
        )
        .expect("ollama is available");
        assert!(!attempt.allows_fallback());
        assert!(matches!(
            decide_fallback(&attempt, CloudFallbackPolicy::Allow),
            FallbackDecision::Exhausted { .. }
        ));
    }

    #[test]
    fn a_failed_preference_write_reprompts_instead_of_losing_the_turn() {
        let consent = PendingConsent {
            attempt: attempt(),
            candidate: AIProvider::OpenAI,
            reason: "down".to_string(),
        };
        assert_eq!(ConsentAnswer::from_allow(true), ConsentAnswer::Allow);
        assert_eq!(
            ConsentAnswer::from_allow(false).policy(),
            CloudFallbackPolicy::Never
        );

        let write = PendingPolicyWrite {
            policy: CloudFallbackPolicy::Allow,
            consent: consent.clone(),
        };
        assert!(matches!(
            apply_policy_write(write.clone(), Err(())),
            PolicyWriteOutcome::Reprompt(_)
        ));
        assert!(matches!(
            apply_policy_write(write, Ok(CloudFallbackPolicy::Allow)),
            PolicyWriteOutcome::Continue(_)
        ));
        assert!(matches!(
            apply_policy_write(
                PendingPolicyWrite {
                    policy: CloudFallbackPolicy::Never,
                    consent,
                },
                Ok(CloudFallbackPolicy::Never)
            ),
            PolicyWriteOutcome::Refuse { .. }
        ));
    }
}
