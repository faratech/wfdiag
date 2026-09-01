//! The keyboard-shortcut help dialog's state.

#![deny(unsafe_code)]

/// The shortcut list overlay. It has no state beyond being open, and no
/// message of its own: the shell opens and closes it.
#[derive(Default)]
pub(crate) struct ShortcutHelpDialog {
    pub(crate) open: bool,
}

/// The rows the dialog prints, in display order.
pub(crate) const SHORTCUT_ROWS: &[(&str, &str)] = &[
    ("Ctrl+K", "Open the command palette"),
    ("Ctrl+1 … Ctrl+6", "Switch between screens"),
    ("Ctrl+Shift+Q", "Run a Quick Scan"),
    ("Ctrl+Shift+F", "Run a Full Scan"),
    ("Ctrl+R", "Refresh"),
    ("Ctrl+/", "Show this shortcut list"),
    ("Esc", "Close dialogs and overlays"),
];

/// Everything the shortcut help overlay can ask for.
#[derive(Clone, Copy)]
pub(crate) enum ShortcutHelpMsg {
    Show,
    Close,
}
