//! Transient outcome notices: the native counterpart of the Store shell's
//! toast stack. One info bar at a time, auto-dismissed, closable, raised for
//! the outcomes a user would otherwise only find in the status line.

#![deny(unsafe_code)]

pub(crate) mod route;
pub(crate) mod state;
pub(crate) mod update;
pub(crate) mod view;
