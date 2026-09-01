//! The root's dispatcher: screen messages in, [`Effect`]s out.
//!
//! Every per-screen message arm in [`WfdiagShell::update`] is one line that
//! lands here. The screen mutates only its own state through a
//! [`crate::app::screen::ScreenCx`]; anything it needs the shell to do comes
//! back as an [`Effect`] and is applied, in order, by [`WfdiagShell::apply_effects`].

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::screen::{Effect, ScanFacts, route_screen};
use crate::app::state::{AiMode, Page};
use crate::dialogs::action_review::state::ActionReviewMsg;
use crate::platform::external::write_text_to_clipboard;
use crate::screens::ai::state::AiMsg;
use crate::screens::diagnostics::state::DiagnosticsMsg;
use crate::screens::history::state::HistoryMsg;
use crate::screens::issues::state::IssuesMsg;
use crate::screens::monitor::state::MonitorMsg;
use crate::screens::processes::state::{ProcessQueryOrigin, ProcessesMsg};
use wfdiag_app::AppEvent;
use windows_reactor::*;

impl WfdiagShell {
    /// The scan facts every screen may read while it updates.
    pub(crate) fn scan_facts(&self) -> ScanFacts {
        ScanFacts {
            busy: self.diagnostics.busy(),
            has_results: !self.diagnostics.results.is_empty(),
            session_id: self.diagnostics.visible_session_id().map(str::to_string),
        }
    }

    /// Perform the work a screen asked for, in the order it asked.
    pub(crate) fn apply_effects(&mut self, effects: Vec<Effect>, context: &ComponentContext<Self>) {
        for effect in effects {
            match effect {
                Effect::Dispatch(command) => {
                    let _ = self.dispatch(command);
                }
                Effect::Status(text) => self.shell.status = text,
                Effect::Transition(page) => {
                    self.transition_to_page(page);
                }
                Effect::BeginScan(kind) => self.begin_diagnostic_scan(kind),
                Effect::AskAi { prompt } => {
                    self.transition_to_page(Page::Ai);
                    self.ai.mode = AiMode::Assistant;
                    self.begin_chat_send(prompt, context);
                }
                Effect::RestartAsAdmin => self.request_admin_relaunch(),
                Effect::CopyReport(text) => match write_text_to_clipboard(&text) {
                    Ok(()) => {
                        self.shell.status = "AI report copied to the clipboard".to_string();
                    }
                    Err(error) => {
                        self.shell.status = format!("Could not copy the AI report · {error}");
                    }
                },
                Effect::StageRemediation {
                    remediation_id,
                    issue_id,
                } => self.stage_chat_remediation(remediation_id, issue_id, context),
            }
        }
    }

    pub(crate) fn route_monitor(&mut self, message: MonitorMsg, context: &ComponentContext<Self>) {
        route_screen!(self, context, monitor.update(message));
    }

    pub(crate) fn route_processes(
        &mut self,
        message: ProcessesMsg,
        context: &ComponentContext<Self>,
    ) {
        route_screen!(self, context, processes.update(message));
    }

    pub(crate) fn route_ai(&mut self, message: AiMsg, context: &ComponentContext<Self>) {
        route_screen!(self, context, ai.update(message));
    }

    /// Send one prompt to the assistant from shell-owned orchestration.
    pub(crate) fn begin_chat_send(&mut self, prompt: String, context: &ComponentContext<Self>) {
        route_screen!(self, context, ai.begin_chat_send(prompt));
    }

    pub(crate) fn route_diagnostics(
        &mut self,
        message: DiagnosticsMsg,
        context: &ComponentContext<Self>,
    ) {
        route_screen!(self, context, diagnostics.update(message));
    }

    pub(crate) fn route_issues(&mut self, message: IssuesMsg, context: &ComponentContext<Self>) {
        route_screen!(self, context, issues.update(message));
    }

    pub(crate) fn route_action_review(
        &mut self,
        message: ActionReviewMsg,
        context: &ComponentContext<Self>,
    ) {
        route_screen!(self, context, action_review.update(message));
    }

    /// Ask the Issues screen to re-detect from shell-owned orchestration
    /// (the Refresh command and the palette).
    pub(crate) fn request_issue_refresh(&mut self, context: &ComponentContext<Self>) -> bool {
        let mut effects = Vec::new();
        let accepted = {
            let live_paused = self.monitor.paused;
            let scan = ScanFacts {
                busy: self.diagnostics.busy(),
                has_results: !self.diagnostics.results.is_empty(),
                session_id: self.diagnostics.visible_session_id().map(str::to_string),
            };
            let mut cx = crate::app::screen::ScreenCx::new(
                &self.shell,
                live_paused,
                scan,
                self.app.as_mut(),
                &mut effects,
                context,
            );
            self.issues.request_refresh(&mut cx)
        };
        self.apply_effects(effects, context);
        accepted
    }

    /// Stage the remediation the assistant asked for, through the normal
    /// Issues-page prepare/approve flow.
    pub(crate) fn stage_chat_remediation(
        &mut self,
        remediation_id: String,
        issue_id: Option<String>,
        context: &ComponentContext<Self>,
    ) {
        route_screen!(
            self,
            context,
            issues.prepare_remediation(remediation_id, issue_id)
        );
    }

    pub(crate) fn route_history(&mut self, message: HistoryMsg, context: &ComponentContext<Self>) {
        route_screen!(self, context, history.update(message));
    }

    /// Ask the History screen to reload the scan list from shell-owned
    /// orchestration (navigation, Refresh, and a finalized scan).
    pub(crate) fn request_history_list(&mut self, context: &ComponentContext<Self>) {
        route_screen!(self, context, history.request_list());
    }

    /// Ask the Processes screen for a page from shell-owned orchestration
    /// (navigation, the lifecycle resume, and the Refresh command).
    pub(crate) fn request_process_page(
        &mut self,
        context: &ComponentContext<Self>,
        debounce_filter: bool,
    ) {
        route_screen!(
            self,
            context,
            processes.request_page(ProcessQueryOrigin::User, debounce_filter)
        );
    }

    /// Hand one engine fact to every screen that owns part of it.
    ///
    /// A screen sees the whole [`AppEvent`] and matches only the variants it
    /// owns, which is what lets two screens split a single event (the monitor
    /// tick belongs to Monitor; the process page it carries belongs to
    /// Processes) without either knowing the other exists.
    pub(crate) fn fan_out_app_event(&mut self, event: &AppEvent, context: &ComponentContext<Self>) {
        route_screen!(self, context, monitor.on_app_event(event));
        route_screen!(self, context, processes.on_app_event(event));
        route_screen!(self, context, history.on_app_event(event));
        route_screen!(self, context, diagnostics.on_app_event(event));
        route_screen!(self, context, issues.on_app_event(event));
        route_screen!(self, context, ai.on_app_event(event));
    }
}
