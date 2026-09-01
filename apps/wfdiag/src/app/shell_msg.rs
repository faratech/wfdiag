//! The chrome's own message alphabet: navigation, the pane, the theme, and
//! the two shell-level commands that are not any one screen's.

#![deny(unsafe_code)]

/// One chrome message.
#[derive(Clone)]
pub(crate) enum ShellMsg {
    /// A nav-rail or command-bar tag. `None` is a deselection the rail emits
    /// while it is being rebuilt and is ignored.
    Navigate(Option<String>),
    TogglePane,
    ToggleTheme,
    /// Refresh whatever the open page shows.
    Refresh,
    RestartAsAdmin,
}
