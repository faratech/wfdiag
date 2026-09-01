//! The export surface's own state.

#![deny(unsafe_code)]

use crate::app::state::PendingExportAction;
use windows_reactor::*;

/// The save picker and the file write behind it.
#[derive(Default)]
pub(crate) struct ExportState {
    /// Guards a save-picker answer against a superseded request (#140).
    pub(crate) picker_epoch: u64,
    pub(crate) picker_busy: bool,
    pub(crate) pending: Option<PendingExportAction>,
    pub(crate) write_task: Option<ComponentTask>,
    pub(crate) error: Option<String>,
}
