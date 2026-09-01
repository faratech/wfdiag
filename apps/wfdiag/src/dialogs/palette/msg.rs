//! The command palette's message alphabet.

#![deny(unsafe_code)]

/// Which focus handoff a delayed palette callback performs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaletteFocusAction {
    FocusQuery,
    RestorePrevious,
}

/// Everything the command palette can ask for.
#[derive(Clone)]
pub(crate) enum PaletteMsg {
    Toggle,
    Close,
    FocusReady {
        epoch: u64,
        action: PaletteFocusAction,
    },
    FocusCancelled {
        epoch: u64,
    },
    FocusRejected {
        epoch: u64,
    },
    QueryChanged(String),
    ActiveChanged(usize),
    /// Execute one palette entry by its tag.
    Command(String),
}
