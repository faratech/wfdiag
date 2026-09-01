//! What a screen or dialog is handed when it updates or renders.
//!
//! A screen owns its own state, its own message enum, and nothing else. It
//! reaches the rest of the application through exactly two values:
//!
//! * [`ScreenCx`] during `update` / `on_app_event` — a read-only view of the
//!   shell ([`ShellState`]), one engine handle, and the [`Effect`] queue the
//!   root drains after the screen returns;
//! * [`ShellEnv`] during `view` — the chrome facts a page needs to paint
//!   (palette, theme, layout metrics, host identity, settings, scan progress).
//!
//! Neither lets a screen touch another screen's state. Anything cross-cutting
//! — a page change, the status line, a scan, the clipboard — is an [`Effect`]
//! the root executes in order, which is what keeps the root a dispatcher
//! instead of a god object.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::rejection_text;
use crate::app::shell::ShellState;
use crate::app::state::Page;
use crate::fixtures::visual::VisualState;
use crate::widgets::palette_colors::Palette;
use wfdiag_app::{AppCommand, AppService, DispatchOutcome, RejectReason};
use wfdiag_native_diagnostics::ScanKind;
use wfdiag_native_settings::AppSettings;
use windows_reactor::*;

/// Work a screen asks for and the root performs once `update` returns.
///
/// Effects are applied in emission order, so a screen that pushes two status
/// lines ends with the second one — exactly as sequential assignment would.
pub(crate) enum Effect {
    /// Send one command whose [`DispatchOutcome`] the screen does not read,
    /// **after** the effects queued before it. Screens that branch on
    /// acceptance call [`ScreenCx::dispatch`] instead, which dispatches
    /// immediately and hands back the outcome; this variant exists for the
    /// cases where ordering against another effect is what matters.
    Dispatch(AppCommand),
    /// Replace the status line.
    Status(String),
    /// Change pages without the destination's entry work (workflow jumps that
    /// perform their own sequencing).
    Transition(Page),
    /// Start a scan through the shell's own scan orchestration.
    BeginScan(ScanKind),
    /// Copy the finished AI report to the clipboard.
    CopyReport(String),
    /// Stage the remediation the assistant asked for, through the Issues
    /// page's normal prepare/approve flow.
    StageRemediation {
        remediation_id: String,
        issue_id: Option<String>,
    },
    /// Open the AI assistant on this prompt. The Issues page uses it to hand
    /// one detected issue to the chat without knowing anything about the AI
    /// screen's state.
    AskAi { prompt: String },
    /// Relaunch the process elevated.
    RestartAsAdmin,
}

/// The handle a screen updates through.
pub(crate) struct ScreenCx<'a> {
    /// The chrome's state, read-only. A screen that needs to change it emits
    /// an [`Effect`].
    pub(crate) shell: &'a ShellState,
    /// Whether live telemetry is currently paused. Owned by the Monitor
    /// screen; copied here because the Processes screen rides the same tick.
    pub(crate) live_paused: bool,
    /// The scan facts other screens read. Copied rather than borrowed so the
    /// Diagnostics screen can still be updated through the same handle.
    pub(crate) scan: ScanFacts,
    /// The Reactor context, for the one UI-owned debounce that still needs a
    /// background task.
    pub(crate) context: &'a ComponentContext<WfdiagShell>,
    app: Option<&'a mut AppService>,
    effects: &'a mut Vec<Effect>,
}

impl<'a> ScreenCx<'a> {
    pub(crate) fn new(
        shell: &'a ShellState,
        live_paused: bool,
        scan: ScanFacts,
        app: Option<&'a mut AppService>,
        effects: &'a mut Vec<Effect>,
        context: &'a ComponentContext<WfdiagShell>,
    ) -> Self {
        Self {
            shell,
            live_paused,
            scan,
            context,
            app,
            effects,
        }
    }

    /// Route one command into the engine and hand back its outcome.
    ///
    /// A fixture build has no engine at all, which is exactly why a screenshot
    /// capture cannot start a scan, write settings, or run a remediation.
    pub(crate) fn dispatch(&mut self, command: AppCommand) -> DispatchOutcome {
        let Some(app) = self.app.as_mut() else {
            return DispatchOutcome::Rejected(RejectReason::NotReady {
                detail: "the diagnostic engine is not running".to_string(),
            });
        };
        app.dispatch(command)
    }

    /// Queue an effect for the root to perform after `update` returns.
    pub(crate) fn effect(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    /// Replace the status line.
    pub(crate) fn status(&mut self, text: impl Into<String>) {
        self.effects.push(Effect::Status(text.into()));
    }

    /// Show a refusal in the status line, using the engine's own wording.
    pub(crate) fn report_rejection(&mut self, outcome: &DispatchOutcome) {
        if let Some(reason) = outcome.rejection() {
            self.status(rejection_text(reason));
        }
    }
}

/// The scan facts other screens read while updating.
///
/// Copied, not borrowed: the same [`ScreenCx`] has to work when the screen
/// being updated *is* the Diagnostics screen that owns them.
#[derive(Clone, Default)]
pub(crate) struct ScanFacts {
    pub(crate) busy: bool,
    /// Whether any committed result is on screen.
    pub(crate) has_results: bool,
    /// The session the visible results came from.
    pub(crate) session_id: Option<String>,
}

/// The scan facts other screens read without owning them.
#[derive(Clone, Copy)]
pub(crate) struct ScanEnv<'a> {
    pub(crate) busy: bool,
    pub(crate) cancelling: bool,
    pub(crate) completed: usize,
    pub(crate) total: usize,
    pub(crate) current_task: Option<&'a str>,
    /// Whether any committed result is on screen.
    pub(crate) has_results: bool,
    /// The session the visible results came from, so a screen can grey out a
    /// projection a newer scan has already replaced.
    pub(crate) session_id: Option<&'a str>,
}

/// The chrome facts a page needs to paint itself.
#[derive(Clone, Copy)]
pub(crate) struct ShellEnv<'a> {
    pub(crate) palette: Palette,
    /// The theme actually in effect, with `System` already resolved.
    pub(crate) theme: WindowTheme,
    pub(crate) narrow: bool,
    /// The Diagnostics page's own single-column breakpoint.
    pub(crate) compact: bool,
    pub(crate) pane_expanded: bool,
    pub(crate) window_size: WindowSize,
    pub(crate) deterministic_visual: bool,
    pub(crate) visual_state: VisualState,
    pub(crate) is_admin: bool,
    pub(crate) monitoring_paused: bool,
    pub(crate) settings: &'a AppSettings,
    pub(crate) scan: ScanEnv<'a>,
}

/// Run one screen or dialog update inside a [`ScreenCx`], then apply the
/// effects it queued.
///
/// The macro exists because the borrow is what makes this safe: `shell` and
/// `app` are borrowed as disjoint fields of the root while the screen's own
/// field is borrowed mutably, which no helper method taking `&mut self` could
/// express.
macro_rules! route_screen {
    ($root:expr, $context:expr, $field:ident . $call:ident ( $($arg:expr),* $(,)? )) => {{
        let root = &mut *$root;
        let mut effects = ::std::vec::Vec::new();
        {
            let live_paused = root.monitor.paused;
            let scan = root.scan_facts();
            let mut cx = $crate::app::screen::ScreenCx::new(
                &root.shell,
                live_paused,
                scan,
                root.app.as_mut(),
                &mut effects,
                $context,
            );
            root.$field.$call($($arg,)* &mut cx);
        }
        root.apply_effects(effects, $context);
    }};
}

pub(crate) use route_screen;
