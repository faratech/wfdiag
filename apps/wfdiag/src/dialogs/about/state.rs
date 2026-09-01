//! The About dialog's own state and message alphabet.

#![deny(unsafe_code)]

use wfdiag_native_update::policy::AboutExternalAction;
use windows_reactor::*;

/// Everything the About dialog renders.
#[derive(Default)]
pub(crate) struct AboutDialog {
    pub(crate) open: bool,
    pub(crate) close_reference: ElementRef<Button>,
    pub(crate) epoch: u64,
    pub(crate) action_error: Option<String>,
    /// The in-flight `ShellExecute`, so a second click cannot queue a second
    /// browser launch.
    pub(crate) launch_task: Option<ComponentTask>,
}

/// Everything the About dialog can report.
#[derive(Clone)]
pub(crate) enum AboutMsg {
    Open,
    Closed {
        epoch: u64,
    },
    ExternalRequested {
        epoch: u64,
        action: AboutExternalAction,
    },
    ExternalFinished {
        epoch: u64,
        result: Result<(), String>,
    },
    ExternalRejected {
        epoch: u64,
    },
}
