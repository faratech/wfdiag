//! How the AI screen answers its own messages and the engine's events.

#![deny(unsafe_code)]

use crate::app::policy::{provider_display_name, provider_from_wire, rejection_text};
use crate::app::screen::{Effect, ScreenCx};
use crate::app::state::{AiMode, ChatDisplayMessage, ChatDisplayRole, FullScanConsent, Page};
use crate::screens::ai::state::{AiMsg, AiScreen};
use wfdiag_app::{AppCommand, AppEvent, ChatEvent, DispatchOutcome, ProviderEvent, ReportEvent};
use wfdiag_native_ai_provider::AIProvider;
use wfdiag_native_diagnostics::ScanKind;

/// How many rendered chat bubbles are kept.
const MAX_CHAT_DISPLAY_MESSAGES: usize = 200;

impl AiScreen {
    pub(crate) fn update(&mut self, message: AiMsg, cx: &mut ScreenCx<'_>) {
        match message {
            AiMsg::SetMode(mode) => self.mode = mode,
            AiMsg::ChatInputChanged(value) => {
                if !self.interaction_blocked() {
                    self.chat_input = value;
                }
            }
            AiMsg::UsePrompt(value) => {
                if self.streaming || self.interaction_blocked() {
                    cx.status("Finish or cancel the current AI request before starting another");
                } else {
                    self.begin_chat_send(value, cx);
                }
            }
            AiMsg::SendChat => self.begin_chat_send(self.chat_input.clone(), cx),
            AiMsg::CancelChat => self.cancel_chat(cx),
            AiMsg::NewConversation => self.begin_new_conversation(cx),
            AiMsg::AllowCloudFallback => self.answer_cloud_fallback(true, cx),
            AiMsg::NeverCloudFallback => self.answer_cloud_fallback(false, cx),
            AiMsg::ApproveFullScan => self.approve_full_scan(cx),
            AiMsg::DismissFullScan => {
                self.full_scan_consent = None;
                cx.status("Full Scan request dismissed");
            }
            AiMsg::GenerateReport => self.begin_report_generation(false, cx),
            AiMsg::RegenerateReport => self.begin_report_generation(true, cx),
            AiMsg::CancelReport => self.cancel_report(cx),
            AiMsg::CopyReport => {
                let Some(report) = self.report_text.clone() else {
                    cx.status("There is no completed AI report to copy");
                    return;
                };
                cx.effect(Effect::CopyReport(report));
            }
            AiMsg::CancelPendingIntent => self.cancel_pending_intent(cx),
            AiMsg::RetryPendingIntent => self.retry_pending_intent(cx),
            AiMsg::ExplainLatestScan => {
                cx.effect(Effect::Transition(Page::Ai));
                self.mode = AiMode::ScanReport;
                self.begin_report_generation(false, cx);
            }
        }
    }

    pub(crate) fn begin_chat_send(&mut self, prompt: String, cx: &mut ScreenCx<'_>) {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        if self.interaction_blocked() {
            cx.status(
                "Finish or cancel the pending AI request before sending another message"
                    .to_string(),
            );
            return;
        }
        if cx.shell.deterministic_visual {
            self.answer = Some(format!(
                "I checked the fixture scan for “{prompt}”. The urgent finding is low space on C:. Free at least 30 GB, then review the four recent Kernel-Power events."
            ));
            self.last_prompt = Some(prompt);
            self.chat_input.clear();
            cx.status("AI response complete · OpenAI cloud".to_string());
            return;
        }
        self.last_prompt = Some(prompt.clone());
        match cx.dispatch(AppCommand::ChatSend { prompt }) {
            DispatchOutcome::Accepted { .. } => {
                self.answer = None;
                self.chat_input.clear();
            }
            DispatchOutcome::Ignored { .. } => {}
            outcome => {
                self.answer = None;
                cx.report_rejection(&outcome);
            }
        }
    }

    pub(crate) fn cancel_chat(&mut self, cx: &mut ScreenCx<'_>) {
        match cx.dispatch(AppCommand::ChatCancel) {
            DispatchOutcome::Accepted { .. } => {
                cx.status("Cancelling the AI response…".to_string());
            }
            outcome => cx.report_rejection(&outcome),
        }
    }

    pub(crate) fn begin_new_conversation(&mut self, cx: &mut ScreenCx<'_>) {
        if self.streaming || self.interaction_blocked() {
            cx.status(
                "Finish or cancel the current AI request before starting a new conversation"
                    .to_string(),
            );
            return;
        }
        self.chat_input.clear();
        self.focus_revision = self.focus_revision.wrapping_add(1);
        self.last_prompt = None;
        self.full_scan_consent = None;
        if cx.shell.deterministic_visual {
            self.messages.clear();
            self.answer = None;
            cx.status("New AI conversation started".to_string());
            return;
        }
        let outcome = cx.dispatch(AppCommand::ChatReset);
        if outcome.rejection().is_some() {
            cx.status("The native AI conversation could not be reset".to_string());
        }
    }

    pub(crate) fn answer_cloud_fallback(&mut self, allow: bool, cx: &mut ScreenCx<'_>) {
        match cx.dispatch(AppCommand::CloudFallbackDecision { allow }) {
            DispatchOutcome::Accepted { .. } => {
                cx.status("Saving the cloud fallback preference…".to_string());
            }
            outcome => cx.report_rejection(&outcome),
        }
    }

    /// Accept the assistant's Full Scan request.
    ///
    /// Nothing was started by the engine: it reported that the model asked.
    /// The scan is dispatched here and the original prompt is re-sent, which
    /// the engine parks behind the scan and resumes when evidence lands.
    pub(crate) fn approve_full_scan(&mut self, cx: &mut ScreenCx<'_>) {
        let Some(consent) = self.full_scan_consent.take() else {
            return;
        };
        if self.streaming {
            self.full_scan_consent = Some(consent);
            cx.status(
                "Wait for the current AI response to finish before starting the Full Scan"
                    .to_string(),
            );
            return;
        }
        if cx.scan.busy {
            self.full_scan_consent = Some(consent);
            cx.status("Wait for the active scan to finish before starting a Full Scan".to_string());
            return;
        }
        if cx.scan.session_id.as_deref() != Some(consent.source_scan_id.as_str()) {
            cx.status("The scan changed; ask again before running a Full Scan".to_string());
            return;
        }
        // Both go through the effect queue so the scan is dispatched BEFORE
        // the prompt: the engine parks the chat behind the running scan and
        // resumes it when evidence lands.
        cx.effect(Effect::BeginScan(ScanKind::Full));
        if !consent.original_prompt.is_empty() {
            cx.effect(Effect::Dispatch(AppCommand::ChatSend {
                prompt: consent.original_prompt,
            }));
        }
    }

    // ---- AI report and one-shot analysis -----------------------------------------

    pub(crate) fn begin_report_generation(&mut self, force_refresh: bool, cx: &mut ScreenCx<'_>) {
        if cx.shell.deterministic_visual {
            cx.status("Visual fixture mode · report generation is disabled".to_string());
            return;
        }
        match cx.dispatch(AppCommand::GenerateReport { force_refresh }) {
            DispatchOutcome::Accepted { .. } => {}
            DispatchOutcome::Ignored { .. } => {}
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    let message = rejection_text(reason);
                    self.report_error = Some(message.clone());
                    cx.status(message);
                }
            }
        }
    }

    pub(crate) fn cancel_report(&mut self, cx: &mut ScreenCx<'_>) {
        // #199: cancellation reaches the generation runtime's own token
        // instead of only clearing the shell's local pending slot.
        match cx.dispatch(AppCommand::CancelReport) {
            DispatchOutcome::Accepted { .. } => {
                cx.status("Cancelling the AI report…".to_string());
            }
            outcome => cx.report_rejection(&outcome),
        }
    }

    pub(crate) fn cancel_pending_intent(&mut self, cx: &mut ScreenCx<'_>) {
        if self.pending_intent.is_none() {
            return;
        }
        let busy = cx.scan.busy;
        // A parked intent is dropped by cancelling whichever turn owns it.
        let _ = cx.dispatch(AppCommand::ChatCancel);
        let _ = cx.dispatch(AppCommand::CancelReport);
        cx.status(if busy {
            "AI request cancelled · the active diagnostic scan will continue".to_string()
        } else {
            "AI request cancelled".to_string()
        });
    }

    pub(crate) fn retry_pending_intent(&mut self, cx: &mut ScreenCx<'_>) {
        let Some(intent) = self.pending_intent.clone() else {
            return;
        };
        match intent {
            wfdiag_app::domain::ai_intent::PendingAiIntent::Chat { prompt } => {
                let _ = cx.dispatch(AppCommand::ChatSend { prompt });
            }
            wfdiag_app::domain::ai_intent::PendingAiIntent::Report { force_refresh } => {
                let _ = cx.dispatch(AppCommand::GenerateReport { force_refresh });
            }
        }
        if !cx.scan.has_results && !cx.scan.busy {
            cx.status("Retrying the prerequisite Quick Scan…".to_string());
        } else if cx.scan.busy {
            cx.status("Waiting for the active scan before continuing AI…".to_string());
        }
    }

    fn on_chat_event(&mut self, event: ChatEvent, cx: &mut ScreenCx<'_>) {
        match event {
            ChatEvent::Started { provider } => {
                if !self.turn_open {
                    self.turn_open = true;
                    let prompt = self.last_prompt.clone().unwrap_or_default();
                    self.push_turn(prompt);
                }
                cx.status(format!(
                    "Asking the AI assistant · {}…",
                    provider_display_name(provider_from_wire(&provider))
                ));
            }
            ChatEvent::Deferred { reason } => cx.status(reason),
            ChatEvent::Delta { text } => {
                self.answer.get_or_insert_with(String::new).push_str(&text);
                let turn = self.turn;
                if let Some(message) = self.assistant_message_mut(turn) {
                    message.text.push_str(&text);
                }
            }
            ChatEvent::ToolActivity { activity, history } => {
                let summary = activity.compatibility_summary();
                let turn = self.turn;
                if let Some(message) = self.assistant_message_mut(turn) {
                    message.tools = *history;
                }
                cx.status(summary);
            }
            ChatEvent::ProposalStaged {
                remediation_id,
                issue_id,
            } => {
                let turn = self.turn;
                let label = format!(
                    "{remediation_id}{}",
                    issue_id
                        .as_deref()
                        .map_or_else(String::new, |issue| format!(" for {issue}"))
                );
                if let Some(message) = self.assistant_message_mut(turn) {
                    message.proposals.push(label);
                }
                // The engine staged nothing: it reported that the model asked.
                // The normal prepare/approve flow still runs from here.
                cx.effect(Effect::StageRemediation {
                    remediation_id,
                    issue_id,
                });
            }
            ChatEvent::FullScanRequested {
                source_scan_id,
                reason,
            } => {
                self.full_scan_consent = Some(FullScanConsent {
                    source_scan_id,
                    reason,
                    original_prompt: self.last_prompt.clone().unwrap_or_default(),
                });
                cx.status("The AI assistant requested a Full Scan for more evidence");
            }
            ChatEvent::CloudFallbackRequired { .. } => {
                cx.status("Local AI was unavailable · cloud permission required");
            }
            ChatEvent::Done {
                provider,
                provider_use,
                finish_reason,
                tool_history,
            } => {
                let notice = crate::app::policy::chat_completion_notice(&finish_reason);
                let turn = self.turn;
                if let Some(message) = self.assistant_message_mut(turn) {
                    message.provider_use = Some(*provider_use);
                    message.finish_reason = Some(finish_reason);
                    message.terminal_message = notice.map(str::to_string);
                    message.tools = *tool_history;
                }
                self.turn_open = false;
                cx.status(notice.map_or_else(
                    || format!("AI response complete · {provider}"),
                    |notice| format!("AI response complete · {provider} · {notice}"),
                ));
            }
            ChatEvent::Failed { message } => {
                self.turn_open = false;
                let turn = self.turn;
                if let Some(display) = self.assistant_message_mut(turn) {
                    display.finish_reason = Some("error".to_string());
                    display.terminal_message = Some(message.clone());
                }
                cx.status(message);
            }
            ChatEvent::Cancelled => {
                self.turn_open = false;
                let turn = self.turn;
                if let Some(display) = self.assistant_message_mut(turn) {
                    display.finish_reason = Some("cancelled".to_string());
                    display.terminal_message = Some("Response cancelled".to_string());
                }
                cx.status("AI response cancelled".to_string());
            }
            ChatEvent::SessionReset => {
                self.turn_open = false;
                self.messages.clear();
                self.answer = None;
                cx.status("New AI conversation started".to_string());
            }
            _ => {}
        }
    }

    /// Append the user and assistant bubbles for one submitted turn.
    pub(crate) fn push_turn(&mut self, prompt: String) {
        self.turn = self.turn.wrapping_add(1);
        let turn = self.turn;
        self.messages.push(ChatDisplayMessage {
            turn,
            role: ChatDisplayRole::User,
            text: prompt,
            provider_use: None,
            finish_reason: Some("submitted".to_string()),
            terminal_message: None,
            tools: wfdiag_native_ai_chat::ChatToolHistory::default(),
            proposals: Vec::new(),
        });
        self.messages.push(ChatDisplayMessage {
            turn,
            role: ChatDisplayRole::Assistant,
            text: String::new(),
            provider_use: None,
            finish_reason: None,
            terminal_message: None,
            tools: wfdiag_native_ai_chat::ChatToolHistory::default(),
            proposals: Vec::new(),
        });
        let excess = self
            .messages
            .len()
            .saturating_sub(MAX_CHAT_DISPLAY_MESSAGES);
        if excess > 0 {
            self.messages.drain(0..excess);
        }
    }

    fn assistant_message_mut(&mut self, turn: u64) -> Option<&mut ChatDisplayMessage> {
        self.messages
            .iter_mut()
            .rev()
            .find(|message| message.turn == turn && message.role == ChatDisplayRole::Assistant)
    }

    fn on_report_event(event: &ReportEvent, cx: &mut ScreenCx<'_>) {
        match event {
            ReportEvent::Started { .. } => cx.status("Preparing AI report…"),
            ReportEvent::Deferred { reason } => cx.status(reason.clone()),
            ReportEvent::Cached { provider, .. } => {
                cx.status(format!("AI report ready · {provider} · cached"));
            }
            ReportEvent::Done { provider, .. } => {
                cx.status(format!("AI report ready · {provider}"));
            }
            ReportEvent::Failed { message } => cx.status(message.clone()),
            ReportEvent::Cancelled => cx.status("AI report cancelled"),
            _ => {}
        }
    }

    fn on_provider_event(&mut self, event: &ProviderEvent, cx: &mut ScreenCx<'_>) {
        match event {
            ProviderEvent::Status(status) | ProviderEvent::PreferenceApplied { status, .. } => {
                self.status_error = None;
                if cx.shell.page == Page::Ai {
                    cx.status(if status.active_provider == AIProvider::None {
                        "AI provider check complete · no provider is ready".to_string()
                    } else {
                        format!("AI provider ready · {}", status.active_provider)
                    });
                }
            }
            ProviderEvent::Failed { error } => {
                self.status_error = Some(error.clone());
                if cx.shell.page == Page::Ai {
                    cx.status(format!("AI provider check failed · {error}"));
                }
            }
            _ => {}
        }
    }

    pub(crate) fn on_app_event(&mut self, event: &AppEvent, cx: &mut ScreenCx<'_>) {
        match event {
            AppEvent::Chat(event) => self.on_chat_event(event.clone(), cx),
            AppEvent::Report(event) => Self::on_report_event(event, cx),
            AppEvent::Provider(event) => self.on_provider_event(event, cx),
            _ => {}
        }
    }
}
