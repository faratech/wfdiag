//! The update notice's own state and message alphabet.

#![deny(unsafe_code)]

use std::time::{Duration, Instant};
use wfdiag_native_update::UpdateInfo;
use windows_reactor::*;

/// The transient "an update is available" info bar and its dismissal timer.
#[derive(Default)]
pub(crate) struct UpdateNoticeDialog {
    /// The update the engine found, also read by the About dialog.
    pub(crate) info: Option<UpdateInfo>,
    pub(crate) visible: bool,
    pub(crate) epoch: u64,
    /// Bumped whenever the timer is re-armed, so a late callback from the
    /// previous arming cannot dismiss the current notice.
    pub(crate) timer_generation: u64,
    pub(crate) task: Option<ComponentTask>,
    pub(crate) started_at: Option<Instant>,
    pub(crate) remaining: Duration,
}

/// Everything the update notice can report.
#[derive(Clone, Copy)]
pub(crate) enum UpdateNoticeMsg {
    Closed { epoch: u64 },
    Expired { epoch: u64, timer_generation: u64 },
    PointerEntered { epoch: u64 },
    PointerExited { epoch: u64 },
    TimerCancelled { epoch: u64, timer_generation: u64 },
    TimerRejected { epoch: u64, timer_generation: u64 },
}
