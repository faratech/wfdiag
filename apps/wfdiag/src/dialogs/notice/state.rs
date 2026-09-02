//! The outcome notice's state and message alphabet.

#![deny(unsafe_code)]

use std::time::Duration;
use windows_reactor::*;

/// How the notice is coloured, and how long it stays.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum NoticeKind {
    Success,
    #[default]
    Info,
    Warning,
    Error,
}

impl NoticeKind {
    /// The Store shell keeps errors on screen longer than good news.
    pub(crate) const fn duration(self) -> Duration {
        match self {
            Self::Success | Self::Info => Duration::from_secs(5),
            Self::Warning | Self::Error => Duration::from_secs(8),
        }
    }
}

/// One notice to raise: a short title and the detail beneath it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NoticeRequest {
    pub(crate) kind: NoticeKind,
    pub(crate) title: String,
    pub(crate) message: String,
}

impl NoticeRequest {
    pub(crate) fn new(
        kind: NoticeKind,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            title: title.into(),
            message: message.into(),
        }
    }
}

/// The transient outcome info bar and its dismissal timer.
#[derive(Default)]
pub(crate) struct NoticeDialog {
    pub(crate) visible: bool,
    pub(crate) kind: NoticeKind,
    pub(crate) title: String,
    pub(crate) message: String,
    pub(crate) epoch: u64,
    /// Bumped whenever the timer is re-armed, so a late callback from the
    /// previous arming cannot dismiss the current notice.
    pub(crate) timer_generation: u64,
    pub(crate) task: Option<ComponentTask>,
    /// A notice was raised somewhere without a Reactor context (an engine
    /// event, a screen effect); the root update arms its timer afterwards.
    pub(crate) arm_pending: bool,
}

/// Everything the notice can report.
#[derive(Clone, Copy)]
pub(crate) enum NoticeMsg {
    Closed { epoch: u64 },
    Expired { epoch: u64, timer_generation: u64 },
    TimerCancelled { epoch: u64, timer_generation: u64 },
    TimerRejected { epoch: u64, timer_generation: u64 },
}
