//! The AI screen's own view state and message alphabet.

#![deny(unsafe_code)]

use crate::app::state::{AiMode, ChatDisplayMessage, CloudFallbackConsent, FullScanConsent};
use wfdiag_app::domain::ai_intent::PendingAiIntent;
use wfdiag_native_ai_chat::ProviderUse;
use wfdiag_native_ai_provider::AIProviderStatus;
use windows_reactor::*;

/// Everything the AI page renders: the assistant and the scan report.
#[derive(Default)]
pub(crate) struct AiScreen {
    pub(crate) mode: AiMode,

    // ---- assistant --------------------------------------------------------
    pub(crate) chat_input: String,
    pub(crate) composer_reference: ElementRef<TextBox>,
    pub(crate) focus_revision: u64,
    pub(crate) answer: Option<String>,
    pub(crate) messages: Vec<ChatDisplayMessage>,
    /// Which rendered turn the current stream belongs to. The engine's own
    /// `Auto` retries are invisible, so one turn is one pair of bubbles.
    pub(crate) turn: u64,
    /// Whether the current turn already has its pair of rendered bubbles. An
    /// `Auto` fallback retry is the same logical turn, so it must not push a
    /// second pair.
    pub(crate) turn_open: bool,
    pub(crate) streaming: bool,
    pub(crate) last_prompt: Option<String>,
    pub(crate) full_scan_consent: Option<FullScanConsent>,
    pub(crate) cloud_fallback_consent: Option<CloudFallbackConsent>,

    // ---- scan report ------------------------------------------------------
    pub(crate) report_text: Option<String>,
    pub(crate) report_provider: Option<String>,
    pub(crate) report_provider_use: Option<ProviderUse>,
    pub(crate) report_generating: bool,
    pub(crate) report_error: Option<String>,

    // ---- provider status --------------------------------------------------
    pub(crate) pending_intent: Option<PendingAiIntent>,
    pub(crate) preparation_error: Option<String>,
    pub(crate) provider_status: Option<AIProviderStatus>,
    pub(crate) status_loading: bool,
    pub(crate) status_error: Option<String>,
}

impl AiScreen {
    /// Whether a consent surface is blocking the composer.
    pub(crate) const fn interaction_blocked(&self) -> bool {
        self.pending_intent.is_some()
            || self.full_scan_consent.is_some()
            || self.cloud_fallback_consent.is_some()
    }
}

/// Everything the AI page can ask for.
#[derive(Clone)]
pub(crate) enum AiMsg {
    SetMode(AiMode),
    ChatInputChanged(String),
    UsePrompt(String),
    SendChat,
    CancelChat,
    NewConversation,
    AllowCloudFallback,
    NeverCloudFallback,
    ApproveFullScan,
    DismissFullScan,
    GenerateReport,
    RegenerateReport,
    CancelReport,
    CopyReport,
    CancelPendingIntent,
    RetryPendingIntent,
    /// Jump to the report tab and generate it for the latest scan.
    ExplainLatestScan,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_consent_surface_blocks_the_composer() {
        assert!(!AiScreen::default().interaction_blocked());
        let blocked = AiScreen {
            full_scan_consent: Some(FullScanConsent {
                source_scan_id: "session-1".to_string(),
                reason: "more evidence".to_string(),
                original_prompt: "why is it slow".to_string(),
            }),
            ..AiScreen::default()
        };
        assert!(blocked.interaction_blocked());
        let pending = AiScreen {
            pending_intent: Some(PendingAiIntent::Report {
                force_refresh: false,
            }),
            ..AiScreen::default()
        };
        assert!(pending.interaction_blocked());
    }
}
