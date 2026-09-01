//! Windows platform edges for the native shell.
//!
//! Everything in this module talks to Win32, WinRT, or the WinUI object model
//! directly. The rest of the shell reaches Windows only through these files so
//! a future Reactor-native equivalent is a localized swap.

pub(crate) mod crash;
pub(crate) mod external;
pub(crate) mod focus;
pub(crate) mod instance;
pub(crate) mod notifications;
pub(crate) mod save_picker;
pub(crate) mod ui_wake;
pub(crate) mod window;
#[allow(dead_code, non_snake_case, non_upper_case_globals)]
pub(crate) mod winui_focus_bindings;
