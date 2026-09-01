//! The command palette's own state.

#![deny(unsafe_code)]

use windows_reactor::*;

/// Everything the command palette renders.
pub(crate) struct PaletteDialog {
    pub(crate) open: bool,
    pub(crate) query: String,
    pub(crate) active_index: usize,
    pub(crate) query_reference: ElementRef<TextBox>,
    pub(crate) button_reference: ElementRef<Button>,
    pub(crate) epoch: u64,
    /// The delayed focus handoff a `ContentDialog` needs on open and close.
    pub(crate) focus_task: Option<ComponentTask>,
}

impl Default for PaletteDialog {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            active_index: 0,
            query_reference: ElementRef::new(),
            button_reference: ElementRef::new(),
            epoch: 0,
            focus_task: None,
        }
    }
}
