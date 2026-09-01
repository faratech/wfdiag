//! AI chat orchestration: sends, attempts, fallbacks, and worker events.

#![deny(unsafe_code)]

use crate::ai::chat_tools::{
    ChatScanSnapshot, ChatToolPorts, ChatToolSnapshot, start_chat_runtime,
};
use crate::app::WfdiagShell;
use crate::app::policy::{
    AiWorkerKind, build_history_scan_record, chat_completion_notice, pending_ai_provider_gate,
    provider_display_name, requires_scan_data, scan_kind_history_tag,
};
use crate::app::state::{
    AiMode, ChatAttempt, ChatDisplayMessage, ChatDisplayRole, CloudFallbackConsent,
    FullScanConsent, Page, PendingAiIntent, PendingAiProviderGate,
    PendingCloudFallbackPolicyUpdate,
};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use wfdiag_native_ai_chat::{ChatToolHistory, ChatWorkerEvent};
use wfdiag_native_ai_provider::{
    AIProvider, FoundryCliEndpointSource, ReqwestOllamaSource, next_fallback_candidate,
    parse_provider_preference,
};
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_issues::projection::advance_nonzero_generation;
use wfdiag_native_settings::{CloudFallbackPolicy, SettingsCommand, SettingsUpdate};
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn ensure_chat_runtime(&mut self) -> Result<(), String> {
        if self.chat_runtime.is_some() {
            return Ok(());
        }
        let settings = self.ai_worker_startup_settings(AiWorkerKind::Chat)?;
        let tools = ChatToolPorts::shipping(self.history_runtime.as_ref().map(Arc::clone));
        let (runtime, receiver) = start_chat_runtime(
            settings,
            Arc::new(FoundryCliEndpointSource::new()),
            Arc::new(ReqwestOllamaSource),
            tools,
        )
        .map_err(|error| format!("Native AI chat could not start: {error}"))?;
        self.chat_receiver = Some(Arc::new(Mutex::new(receiver)));
        self.chat_runtime = Some(runtime);
        Ok(())
    }

    pub(crate) fn resume_chat_wait(&mut self, _context: &ComponentContext<Self>) {
        self.chat_wait = None;
    }

    pub(crate) fn chat_tool_snapshot(&self) -> ChatToolSnapshot {
        let arch = self
            .architecture
            .as_ref()
            .map(|snapshot| snapshot.emulation_status.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        let system_overview = Some(format!(
            "Computer name: {}\nOperating system: {}\nArchitecture: {}\nElevated: {}",
            self.system_info.computer_name,
            self.system_info.os_version,
            arch,
            if self.is_admin { "yes" } else { "no" }
        ));
        let session_id = self
            .diagnostic_results
            .first()
            .map(|result| result.session_id.clone())
            .or_else(|| self.diagnostic_session_id.clone());
        let selected_tasks = if self.diagnostic_expected_task_ids.is_empty() {
            self.diagnostic_results
                .iter()
                .map(|result| result.task_id.clone())
                .collect()
        } else {
            self.diagnostic_expected_task_ids.clone()
        };
        let scan = session_id.as_ref().map(|session_id| ChatScanSnapshot {
            session_id: session_id.clone(),
            started_at: self
                .diagnostic_scan_start
                .and_then(|started| SystemTime::now().checked_sub(started.elapsed()))
                .unwrap_or_else(SystemTime::now),
            scan_kind: self.diagnostic_scan_kind.unwrap_or(ScanKind::Targeted),
            selected_tasks,
            results: self.diagnostic_results.clone(),
            running: self.diagnostics_busy(),
        });
        let current_history_scan = session_id.as_ref().and_then(|session_id| {
            (!self.diagnostic_results.is_empty() && !self.diagnostics_busy()).then(|| {
                build_history_scan_record(
                    session_id.clone(),
                    &self.system_info,
                    &self.diagnostic_results,
                    self.diagnostic_duration_ms,
                    self.diagnostic_scan_kind
                        .map(scan_kind_history_tag)
                        .unwrap_or("Diagnostic Scan")
                        .to_string(),
                )
            })
        });

        ChatToolSnapshot {
            system_overview,
            scan,
            current_history_scan,
            issues: self.issues.clone(),
            history: self.history_summaries.clone(),
            live_stats: self.latest_system_stats.clone(),
            remediations: wfdiag_native_issues::remediation_summaries(),
            network_grounding_enabled: self.settings_snapshot.network_grounding_enabled,
        }
    }

    pub(crate) fn chat_interaction_blocked(&self) -> bool {
        self.pending_ai_intent.is_some()
            || self.full_scan_consent.is_some()
            || self.cloud_fallback_consent.is_some()
            || self.cloud_fallback_policy_update.is_some()
    }

    /// Apply one chat worker event. Events are delivered batched (see
    /// `drain_chat_events`); this per-event body is order-sensitive and may
    /// re-arm fallback attempts, so `chat_pending` is resolved per event.
    pub(crate) fn apply_chat_worker_event(
        &mut self,
        event: ChatWorkerEvent,
        context: &ComponentContext<Self>,
    ) {
        let Some(pending) = self.chat_pending else {
            return;
        };
        if pending != event.request_id() {
            self.resume_chat_wait(context);
            return;
        }
        let logical_request_id = self
            .chat_attempt
            .as_ref()
            .map_or(pending, |attempt| attempt.logical_request_id);
        match event {
            ChatWorkerEvent::Delta { text, .. } => {
                self.chat_answer
                    .get_or_insert_with(String::new)
                    .push_str(&text);
                if let Some(message) = self.chat_assistant_message_mut(logical_request_id) {
                    message.text.push_str(&text);
                }
                self.resume_chat_wait(context);
            }
            ChatWorkerEvent::ToolActivity {
                history, summary, ..
            } => {
                if let Some(message) = self.chat_assistant_message_mut(logical_request_id) {
                    message.tools = history;
                }
                self.status = summary;
                self.resume_chat_wait(context);
            }
            ChatWorkerEvent::Proposal {
                remediation_id,
                issue_id,
                ..
            } => {
                if let Some(message) = self.chat_assistant_message_mut(logical_request_id) {
                    message.proposals.push(format!(
                        "{}{}",
                        remediation_id,
                        issue_id
                            .as_deref()
                            .map_or_else(String::new, |issue| format!(" for {issue}"))
                    ));
                }
                self.prepare_remediation(remediation_id, issue_id, context);
                self.resume_chat_wait(context);
            }
            ChatWorkerEvent::FullScanRequested {
                source_scan_id,
                reason,
                ..
            } => {
                let source_is_current = self
                    .diagnostic_results
                    .first()
                    .is_some_and(|result| result.session_id == source_scan_id);
                if source_is_current && self.diagnostic_scan_kind != Some(ScanKind::Full) {
                    self.full_scan_consent = Some(FullScanConsent {
                        source_scan_id,
                        reason,
                        original_prompt: self.chat_last_prompt.clone().unwrap_or_default(),
                    });
                    self.status =
                        "The AI assistant requested a Full Scan for more evidence".to_string();
                }
                self.resume_chat_wait(context);
            }
            ChatWorkerEvent::Done {
                provider,
                provider_use,
                finish_reason,
                tool_history,
                ..
            } => {
                self.chat_pending = None;
                self.chat_attempt = None;
                let completion_notice = chat_completion_notice(&finish_reason);
                if let Some(message) = self.chat_assistant_message_mut(logical_request_id) {
                    message.provider_use = Some(provider_use);
                    message.finish_reason = Some(finish_reason);
                    message.terminal_message = completion_notice.map(str::to_string);
                    message.tools = tool_history;
                }
                self.status = completion_notice.map_or_else(
                    || format!("AI response complete · {provider}"),
                    |notice| format!("AI response complete · {provider} · {notice}"),
                );
                self.resume_pending_ai_intent(context);
            }
            ChatWorkerEvent::RetryableFailure {
                message,
                tool_history,
                ..
            } => {
                self.chat_pending = None;
                if let Some(display) = self.chat_assistant_message_mut(logical_request_id) {
                    display.tools = tool_history;
                }
                let Some(mut attempt) = self.chat_attempt.take() else {
                    self.finish_chat_attempt_failure(logical_request_id, message);
                    self.resume_pending_ai_intent(context);
                    return;
                };
                if attempt.first_failure.is_none() {
                    attempt.first_failure = Some(message);
                }
                let Some(candidate) = next_fallback_candidate(
                    attempt.preference,
                    Some(attempt.current_provider),
                    &attempt.tried,
                    attempt.availability,
                ) else {
                    let first_failure = attempt
                        .first_failure
                        .unwrap_or_else(|| "No fallback provider is available".to_string());
                    self.finish_chat_attempt_failure(logical_request_id, first_failure);
                    self.resume_pending_ai_intent(context);
                    return;
                };
                let reason = attempt.first_failure.clone().unwrap_or_default();
                let consent = CloudFallbackConsent {
                    previous_request_id: pending,
                    attempt,
                    candidate: candidate.provider,
                    reason,
                };
                if candidate.crosses_local_to_cloud {
                    match self.settings_snapshot.cloud_fallback_policy {
                        CloudFallbackPolicy::Allow => {
                            self.continue_chat_fallback(consent, context);
                        }
                        CloudFallbackPolicy::Never => {
                            self.finish_chat_attempt_failure(
                                logical_request_id,
                                format!(
                                    "{} Cloud fallback is disabled in Settings.",
                                    consent.reason
                                ),
                            );
                            self.resume_pending_ai_intent(context);
                        }
                        CloudFallbackPolicy::Ask => {
                            self.chat_attempt = Some(consent.attempt.clone());
                            self.cloud_fallback_consent = Some(consent);
                            self.status =
                                "Local AI was unavailable · cloud permission required".to_string();
                            self.resume_chat_wait(context);
                        }
                    }
                } else {
                    self.continue_chat_fallback(consent, context);
                }
            }
            ChatWorkerEvent::Failed {
                message,
                finish_reason,
                tool_history,
                ..
            } => {
                self.chat_pending = None;
                self.chat_attempt = None;
                if let Some(display) = self.chat_assistant_message_mut(logical_request_id) {
                    display.finish_reason = Some(finish_reason);
                    display.terminal_message = Some(message.clone());
                    display.tools = tool_history;
                }
                self.status = message;
                self.resume_pending_ai_intent(context);
            }
            ChatWorkerEvent::Cancelled {
                finish_reason,
                tool_history,
                ..
            } => {
                self.chat_pending = None;
                self.chat_attempt = None;
                if let Some(message) = self.chat_assistant_message_mut(logical_request_id) {
                    message.finish_reason = Some(finish_reason);
                    message.terminal_message = Some("Response cancelled".to_string());
                    message.tools = tool_history;
                }
                self.status = "AI response cancelled".to_string();
                self.resume_pending_ai_intent(context);
            }
        }
    }

    pub(crate) fn begin_chat_send(&mut self, prompt: String, context: &ComponentContext<Self>) {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        if self.chat_interaction_blocked() {
            self.status = "Finish or cancel the pending AI request before sending another message"
                .to_string();
            return;
        }
        if self.deterministic_visual {
            self.chat_answer = Some(format!(
                "I checked the fixture scan for “{prompt}”. The urgent finding is low space on C:. Free at least 30 GB, then review the four recent Kernel-Power events."
            ));
            self.chat_last_prompt = Some(prompt);
            self.chat_input.clear();
            self.status = "AI response complete · OpenAI cloud".to_string();
            return;
        }
        if !self.settings_snapshot.ai_enabled {
            self.chat_answer = None;
            self.status = "Enable AI insights in Settings before sending".to_string();
            return;
        }
        if self.chat_pending.is_some() {
            self.status = "A response is already streaming…".to_string();
            return;
        }
        if let Err(error) = self.ensure_chat_runtime() {
            self.chat_answer = None;
            self.status = error;
            return;
        }
        // Mirror the report path: a probe that is loading (or one that still
        // needs to start after settings loaded/cleared the snapshot) defers the
        // send instead of failing it. AiStatusFinished resumes stashed intents
        // through resume_pending_ai_intent, which re-enters this method.
        let gate = pending_ai_provider_gate(
            self.settings_snapshot.ai_enabled,
            self.ai_status_loading,
            self.ai_provider_status.as_ref(),
        );
        match gate {
            PendingAiProviderGate::Ready => {}
            PendingAiProviderGate::Waiting | PendingAiProviderGate::Refresh => {
                if gate == PendingAiProviderGate::Refresh {
                    self.request_ai_provider_status(context);
                }
                self.pending_ai_intent = Some(PendingAiIntent::Chat {
                    prompt: prompt.clone(),
                });
                self.pending_ai_preparation_error = None;
                self.chat_last_prompt = Some(prompt);
                self.chat_input.clear();
                self.status = "Checking AI providers before continuing…".to_string();
                return;
            }
            PendingAiProviderGate::Disabled => {
                self.chat_answer = None;
                self.status = "Enable AI insights in Settings before sending".to_string();
                return;
            }
            PendingAiProviderGate::Unavailable => {
                self.chat_answer = None;
                self.status = "Set up an available AI provider before sending".to_string();
                return;
            }
        }
        let Some(provider_status) = self
            .ai_provider_status
            .as_ref()
            .filter(|status| status.active_provider != AIProvider::None)
        else {
            self.chat_answer = None;
            self.status = "Set up an available AI provider before sending".to_string();
            return;
        };
        if requires_scan_data(&prompt) && self.diagnostic_results.is_empty() {
            self.transition_to_page(Page::Ai);
            self.ai_mode = AiMode::Assistant;
            self.pending_ai_intent = Some(PendingAiIntent::Chat {
                prompt: prompt.clone(),
            });
            self.pending_ai_preparation_error = None;
            self.chat_last_prompt = Some(prompt);
            self.chat_input.clear();
            if self.diagnostics_busy() {
                self.status =
                    "Waiting for the active scan before asking the AI assistant…".to_string();
            } else {
                self.status = "Running a Quick Scan before asking the AI assistant…".to_string();
                self.begin_diagnostic_scan(ScanKind::Quick, context);
            }
            return;
        }
        let preference = parse_provider_preference(&self.settings_snapshot.preferred_ai_provider);
        let availability = provider_status.availability();
        let Some(candidate) = next_fallback_candidate(preference, None, &[], availability) else {
            self.chat_answer = None;
            self.status = "Set up an available AI provider before sending".to_string();
            return;
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.chat_request_id) else {
            self.status = "Native chat request identity was exhausted".to_string();
            return;
        };
        let tools = self.chat_tool_snapshot();
        let attempt = ChatAttempt {
            logical_request_id: request_id,
            prompt: prompt.clone(),
            tools,
            preference,
            availability,
            tried: vec![candidate.provider],
            initial_provider: candidate.provider,
            current_provider: candidate.provider,
            first_failure: None,
        };
        if self.dispatch_chat_attempt(request_id, None, attempt, context) {
            self.chat_answer = None;
            self.chat_last_prompt = Some(prompt.clone());
            self.chat_messages.push(ChatDisplayMessage {
                request_id,
                role: ChatDisplayRole::User,
                text: prompt,
                provider_use: None,
                finish_reason: Some("submitted".to_string()),
                terminal_message: None,
                tools: ChatToolHistory::default(),
                proposals: Vec::new(),
            });
            self.chat_messages.push(ChatDisplayMessage {
                request_id,
                role: ChatDisplayRole::Assistant,
                text: String::new(),
                provider_use: None,
                finish_reason: None,
                terminal_message: None,
                tools: ChatToolHistory::default(),
                proposals: Vec::new(),
            });
            self.chat_input.clear();
            self.bound_chat_messages();
        } else if self.chat_runtime.is_some() {
            self.status = "The native AI chat queue is unavailable".to_string();
        }
    }

    /// Keep the rendered conversation bounded. Only ever called right after a
    /// send pushed two messages, so a single drain drops the oldest complete
    /// turn pair without disturbing the streaming tail.
    pub(crate) fn bound_chat_messages(&mut self) {
        const MAX_CHAT_DISPLAY_MESSAGES: usize = 200;
        let excess = self
            .chat_messages
            .len()
            .saturating_sub(MAX_CHAT_DISPLAY_MESSAGES);
        if excess > 0 {
            self.chat_messages.drain(0..excess);
        }
    }

    pub(crate) fn dispatch_chat_attempt(
        &mut self,
        request_id: u64,
        previous_request_id: Option<u64>,
        attempt: ChatAttempt,
        context: &ComponentContext<Self>,
    ) -> bool {
        if let Err(error) = self.ensure_chat_runtime() {
            self.status = error;
            return false;
        }
        let provider = attempt.current_provider;
        let fallback_from =
            (provider != attempt.initial_provider).then_some(attempt.initial_provider);
        let allow_fallback = next_fallback_candidate(
            attempt.preference,
            Some(provider),
            &attempt.tried,
            attempt.availability,
        )
        .is_some();
        let Some(runtime) = self.chat_runtime.as_ref() else {
            return false;
        };
        let queued = previous_request_id.map_or_else(
            || {
                runtime.send_attempt(
                    request_id,
                    attempt.prompt.clone(),
                    provider,
                    fallback_from,
                    allow_fallback,
                    attempt.tools.clone(),
                )
            },
            |previous_request_id| {
                runtime.retry_attempt(
                    previous_request_id,
                    request_id,
                    attempt.prompt.clone(),
                    provider,
                    fallback_from,
                    allow_fallback,
                    attempt.tools.clone(),
                )
            },
        );
        if queued {
            self.chat_pending = Some(request_id);
            self.chat_attempt = Some(attempt);
            self.cloud_fallback_consent = None;
            self.status = format!(
                "Asking the AI assistant · {}…",
                provider_display_name(provider)
            );
            self.resume_chat_wait(context);
        }
        queued
    }

    pub(crate) fn continue_chat_fallback(
        &mut self,
        mut consent: CloudFallbackConsent,
        context: &ComponentContext<Self>,
    ) {
        consent.attempt.current_provider = consent.candidate;
        consent.attempt.tried.push(consent.candidate);
        let logical_request_id = consent.attempt.logical_request_id;
        let Some(request_id) = advance_nonzero_generation(&mut self.chat_request_id) else {
            self.finish_chat_attempt_failure(
                logical_request_id,
                "Native chat request identity was exhausted".to_string(),
            );
            return;
        };
        if !self.dispatch_chat_attempt(
            request_id,
            Some(consent.previous_request_id),
            consent.attempt,
            context,
        ) {
            let message = if self.chat_runtime.is_none() {
                self.status.clone()
            } else {
                "The native AI chat queue is unavailable".to_string()
            };
            self.finish_chat_attempt_failure(logical_request_id, message);
        }
    }

    pub(crate) fn persist_cloud_fallback_decision(&mut self, policy: CloudFallbackPolicy) {
        let Some(consent) = self.cloud_fallback_consent.take() else {
            return;
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.settings_request_id) else {
            self.cloud_fallback_consent = Some(consent);
            self.status = "Settings request identity was exhausted".to_string();
            return;
        };
        let Some(runtime) = self.settings_runtime.as_ref() else {
            self.cloud_fallback_consent = Some(consent);
            self.status = "Native settings persistence is unavailable".to_string();
            return;
        };
        let command = SettingsCommand::Update {
            request_id,
            update: SettingsUpdate::CloudFallbackPolicy(policy),
        };
        if let Err(error) = runtime.send(command) {
            self.cloud_fallback_consent = Some(consent);
            self.status = format!("Cloud fallback preference was not saved · {error}");
            return;
        }
        self.cloud_fallback_policy_update = Some(PendingCloudFallbackPolicyUpdate {
            request_id,
            policy,
            consent,
        });
        self.status = "Saving the cloud fallback preference…".to_string();
    }

    pub(crate) fn finish_chat_attempt_failure(&mut self, logical_request_id: u64, message: String) {
        self.chat_pending = None;
        self.chat_attempt = None;
        self.cloud_fallback_consent = None;
        if let Some(display) = self.chat_assistant_message_mut(logical_request_id) {
            display.finish_reason = Some("error".to_string());
            display.terminal_message = Some(message.clone());
        }
        self.status = message;
    }

    pub(crate) fn finish_chat_attempt_cancelled(&mut self, logical_request_id: u64) {
        self.chat_pending = None;
        self.chat_attempt = None;
        self.cloud_fallback_consent = None;
        if let Some(display) = self.chat_assistant_message_mut(logical_request_id) {
            display.finish_reason = Some("cancelled".to_string());
            display.terminal_message = Some("Response cancelled".to_string());
        }
        self.status = "AI response cancelled".to_string();
    }

    pub(crate) fn chat_assistant_message_mut(
        &mut self,
        request_id: u64,
    ) -> Option<&mut ChatDisplayMessage> {
        self.chat_messages.iter_mut().rev().find(|message| {
            message.request_id == request_id && message.role == ChatDisplayRole::Assistant
        })
    }

    pub(crate) fn resume_pending_ai_intent(&mut self, context: &ComponentContext<Self>) {
        let Some(intent) = self.pending_ai_intent.as_ref() else {
            return;
        };

        if self.diagnostics_busy() {
            self.status = "Waiting for the prerequisite scan before continuing AI…".to_string();
            return;
        }
        if matches!(intent, PendingAiIntent::Chat { .. }) && self.chat_pending.is_some() {
            self.status = "Waiting for the current AI response to finish…".to_string();
            return;
        }
        if matches!(intent, PendingAiIntent::Report { .. }) && self.report_pending.is_some() {
            self.status = "Waiting for the current AI report to finish…".to_string();
            return;
        }

        match pending_ai_provider_gate(
            self.settings_snapshot.ai_enabled,
            self.ai_status_loading,
            self.ai_provider_status.as_ref(),
        ) {
            PendingAiProviderGate::Ready => {}
            PendingAiProviderGate::Waiting => {
                self.status = "Checking AI providers before continuing…".to_string();
                return;
            }
            PendingAiProviderGate::Disabled => {
                let message = "Enable AI insights in Settings before continuing";
                self.mark_pending_ai_preparation_error(message);
                self.status = message.to_string();
                return;
            }
            PendingAiProviderGate::Unavailable => {
                let message = "Set up an available AI provider before continuing";
                self.mark_pending_ai_preparation_error(message);
                self.status = message.to_string();
                return;
            }
            PendingAiProviderGate::Refresh => {
                self.pending_ai_preparation_error = None;
                self.request_ai_provider_status(context);
                if self.ai_status_loading {
                    self.status = "Checking AI providers before continuing…".to_string();
                } else {
                    let message = self.ai_status_error.clone().unwrap_or_else(|| {
                        "Native AI provider discovery is unavailable".to_string()
                    });
                    self.mark_pending_ai_preparation_error(message.clone());
                    self.status = message;
                }
                return;
            }
        }

        let Some(intent) = self.pending_ai_intent.take() else {
            return;
        };
        self.pending_ai_preparation_error = None;
        match intent {
            PendingAiIntent::Report { force_refresh } => {
                self.transition_to_page(Page::Ai);
                self.ai_mode = AiMode::ScanReport;
                self.begin_report_generation(force_refresh, context);
            }
            PendingAiIntent::Chat { prompt } => {
                self.transition_to_page(Page::Ai);
                self.ai_mode = AiMode::Assistant;
                self.begin_chat_send(prompt, context);
            }
        }
    }

    pub(crate) fn mark_pending_ai_preparation_error(&mut self, message: impl Into<String>) {
        if self.pending_ai_intent.is_some() {
            self.pending_ai_preparation_error = Some(message.into());
        }
    }
}
