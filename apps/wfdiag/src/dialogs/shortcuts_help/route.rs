//! Routing the shortcut-help overlay's two messages.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::dialogs::shortcuts_help::state::ShortcutHelpMsg;

impl WfdiagShell {
    /// One shortcut-help message.
    pub(crate) fn route_shortcuts(&mut self, message: ShortcutHelpMsg) {
        match message {
            ShortcutHelpMsg::Show => self.shortcuts.open = true,
            ShortcutHelpMsg::Close => self.shortcuts.open = false,
        }
    }
}
