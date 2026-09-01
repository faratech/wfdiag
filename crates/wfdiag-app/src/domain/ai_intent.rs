//! Queued AI work, and the gate that decides when it may run.
//!
//! A chat message or a report can be asked for before the engine is ready to
//! serve it: the provider probe may still be running, or the request may need
//! scan evidence that does not exist yet. Rather than failing, the request is
//! parked as a [`PendingAiIntent`] and re-evaluated after every prerequisite
//! settles.
//!
//! The gate itself is [`crate::domain::providers::PendingAiProviderGate`];
//! this module turns its outcome, plus the scan and in-flight state, into one
//! instruction the service can act on.

use crate::domain::providers::PendingAiProviderGate;
use wfdiag_native_ai_provider::AIProviderStatus;

/// AI work that was asked for but could not start yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAiIntent {
    /// A chat message.
    Chat {
        /// The user's prompt.
        prompt: String,
    },
    /// The one-click scan report.
    Report {
        /// Whether the cached report must be bypassed.
        force_refresh: bool,
    },
}

impl PendingAiIntent {
    /// Whether this is a chat message.
    #[must_use]
    pub const fn is_chat(&self) -> bool {
        matches!(self, Self::Chat { .. })
    }

    /// Whether this is a report.
    #[must_use]
    pub const fn is_report(&self) -> bool {
        matches!(self, Self::Report { .. })
    }
}

/// Everything the gate needs, captured once per evaluation.
// Six independent facts a caller reads from six different places. Grouping
// them into a state machine would invent a state nothing else has.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug)]
pub struct IntentReadiness<'a> {
    /// Whether AI features are switched on in settings.
    pub ai_enabled: bool,
    /// Whether a provider probe is in flight.
    pub provider_loading: bool,
    /// The last provider status, when one is known.
    pub provider_status: Option<&'a AIProviderStatus>,
    /// Whether a diagnostic scan occupies the engine.
    pub scan_busy: bool,
    /// Whether a chat turn is already streaming.
    pub chat_busy: bool,
    /// Whether a report is already generating.
    pub report_busy: bool,
}

/// What the service should do with a parked intent right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntentAction {
    /// Run it.
    Run,
    /// Keep waiting; the reason is a user-facing status.
    Wait {
        /// What the intent is waiting for.
        reason: String,
    },
    /// Ask for a provider status first, then keep waiting.
    RefreshProviders,
    /// Give up; the reason is a user-facing failure.
    Fail {
        /// Why the intent cannot run.
        reason: String,
    },
}

/// Decide what to do with `intent` given `readiness`.
///
/// The prerequisites are checked before the provider gate, in the shipping
/// shell's order: a scan first (its evidence is what the AI will read), then
/// the domain's own in-flight guard, then provider readiness.
#[must_use]
pub fn evaluate(intent: &PendingAiIntent, readiness: IntentReadiness<'_>) -> IntentAction {
    if readiness.scan_busy {
        return IntentAction::Wait {
            reason: "Waiting for the prerequisite scan before continuing AI…".to_string(),
        };
    }
    if intent.is_chat() && readiness.chat_busy {
        return IntentAction::Wait {
            reason: "Waiting for the current AI response to finish…".to_string(),
        };
    }
    if intent.is_report() && readiness.report_busy {
        return IntentAction::Wait {
            reason: "Waiting for the current AI report to finish…".to_string(),
        };
    }
    match PendingAiProviderGate::evaluate(
        readiness.ai_enabled,
        readiness.provider_loading,
        readiness.provider_status,
    ) {
        PendingAiProviderGate::Ready => IntentAction::Run,
        PendingAiProviderGate::Waiting => IntentAction::Wait {
            reason: "Checking AI providers before continuing…".to_string(),
        },
        PendingAiProviderGate::Refresh => IntentAction::RefreshProviders,
        PendingAiProviderGate::Disabled => IntentAction::Fail {
            reason: "Enable AI insights in Settings before continuing".to_string(),
        },
        PendingAiProviderGate::Unavailable => IntentAction::Fail {
            reason: "Set up an available AI provider before continuing".to_string(),
        },
    }
}

/// Whether a prompt needs scan evidence to be answerable.
///
/// A general Windows question does not justify running diagnostics on the
/// user's machine; a question about *this* PC does. The shipping shell's list
/// of exemptions is reproduced exactly.
#[must_use]
pub fn requires_scan_data(prompt: &str) -> bool {
    let normalized = prompt.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }
    if normalized.contains("in general") || normalized.contains("generally") {
        return false;
    }
    if normalized.starts_with("define ") {
        return false;
    }
    if matches!(normalized.as_str(), "what is windows?" | "what is windows") {
        return false;
    }
    if normalized.starts_with("what does ") && normalized.contains(" mean") {
        return false;
    }
    !matches!(
        normalized.trim_end_matches(['!', '.', '?']),
        "hi" | "hello" | "hey" | "thanks" | "thank you" | "good morning" | "good evening"
    )
}

#[cfg(test)]
mod tests {
    use super::{IntentAction, IntentReadiness, PendingAiIntent, evaluate, requires_scan_data};
    use wfdiag_native_ai_provider::{AIProvider, AIProviderStatus};

    fn status(active: AIProvider) -> AIProviderStatus {
        AIProviderStatus {
            preferred_provider: AIProvider::None,
            openai_available: false,
            openai_api_key_set: false,
            phi_silica_available: false,
            phi_silica_ready: false,
            phi_silica_message: None,
            foundry_local_available: false,
            foundry_local_endpoint: None,
            active_provider: active,
            providers: Vec::new(),
        }
    }

    fn ready(status: &AIProviderStatus) -> IntentReadiness<'_> {
        IntentReadiness {
            ai_enabled: true,
            provider_loading: false,
            provider_status: Some(status),
            scan_busy: false,
            chat_busy: false,
            report_busy: false,
        }
    }

    #[test]
    fn a_running_scan_outranks_every_other_prerequisite() {
        let status = status(AIProvider::None);
        let readiness = IntentReadiness {
            scan_busy: true,
            ai_enabled: false,
            ..ready(&status)
        };
        assert_eq!(
            evaluate(
                &PendingAiIntent::Report {
                    force_refresh: false
                },
                readiness
            ),
            IntentAction::Wait {
                reason: "Waiting for the prerequisite scan before continuing AI…".to_string()
            }
        );
    }

    #[test]
    fn each_domain_waits_only_on_its_own_in_flight_work() {
        let status = status(AIProvider::Ollama);
        let chat = PendingAiIntent::Chat {
            prompt: "why".to_string(),
        };
        let report = PendingAiIntent::Report {
            force_refresh: false,
        };
        let chat_busy = IntentReadiness {
            chat_busy: true,
            ..ready(&status)
        };
        assert!(matches!(
            evaluate(&chat, chat_busy),
            IntentAction::Wait { .. }
        ));
        assert_eq!(evaluate(&report, chat_busy), IntentAction::Run);

        let report_busy = IntentReadiness {
            report_busy: true,
            ..ready(&status)
        };
        assert!(matches!(
            evaluate(&report, report_busy),
            IntentAction::Wait { .. }
        ));
        assert_eq!(evaluate(&chat, report_busy), IntentAction::Run);
    }

    #[test]
    fn the_provider_gate_maps_onto_run_wait_refresh_and_fail() {
        let chat = PendingAiIntent::Chat {
            prompt: "why".to_string(),
        };
        let usable = status(AIProvider::Ollama);
        assert_eq!(evaluate(&chat, ready(&usable)), IntentAction::Run);

        let none = status(AIProvider::None);
        assert_eq!(
            evaluate(&chat, ready(&none)),
            IntentAction::Fail {
                reason: "Set up an available AI provider before continuing".to_string()
            }
        );
        assert_eq!(
            evaluate(
                &chat,
                IntentReadiness {
                    provider_status: None,
                    ..ready(&none)
                }
            ),
            IntentAction::RefreshProviders
        );
        assert!(matches!(
            evaluate(
                &chat,
                IntentReadiness {
                    provider_loading: true,
                    ..ready(&none)
                }
            ),
            IntentAction::Wait { .. }
        ));
        assert_eq!(
            evaluate(
                &chat,
                IntentReadiness {
                    ai_enabled: false,
                    ..ready(&usable)
                }
            ),
            IntentAction::Fail {
                reason: "Enable AI insights in Settings before continuing".to_string()
            }
        );
    }

    #[test]
    fn only_questions_about_this_pc_justify_running_diagnostics() {
        assert!(requires_scan_data("why is my disk full"));
        assert!(requires_scan_data("is my driver signed?"));
        assert!(!requires_scan_data("  "));
        assert!(!requires_scan_data("Hello!"));
        assert!(!requires_scan_data("thanks"));
        assert!(!requires_scan_data(
            "How does Windows Update work in general?"
        ));
        assert!(!requires_scan_data("define TPM"));
        assert!(!requires_scan_data("What is Windows?"));
        assert!(!requires_scan_data("what does BSOD mean"));
    }
}
