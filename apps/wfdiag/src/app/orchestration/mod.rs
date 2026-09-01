//! `WfdiagShell` method groups, split by the concern they orchestrate.
//!
//! * [`events`] projects the engine's [`wfdiag_app::AppEvent`] stream and
//!   [`wfdiag_app::AppSnapshot`] into view state;
//! * [`commands`] turns user intents into [`wfdiag_app::AppCommand`]s;
//! * [`lifecycle`], [`settings`], [`export`] and [`update`] own the surfaces
//!   that are genuinely the shell's: the window, the dialogs, the save picker,
//!   and the update notice timer.

pub(crate) mod commands;
pub(crate) mod events;
pub(crate) mod export;
pub(crate) mod lifecycle;
pub(crate) mod settings;
pub(crate) mod update;
