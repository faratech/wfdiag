//! The cross-cutting `WfdiagShell` method groups.
//!
//! Anything that belongs to exactly one screen or dialog now lives with that
//! screen or dialog. What is left here is genuinely the shell's:
//!
//! * [`route`] is the dispatcher itself — it hands one message to the screen
//!   that owns it and then performs the [`crate::app::screen::Effect`]s that
//!   screen asked for;
//! * [`events`] projects [`wfdiag_app::AppSnapshot`] into every screen's view
//!   state and keeps the few engine facts that outlive any one screen (scan
//!   finalization, worker failures);
//! * [`commands`] holds the intents more than one surface starts: a scan, an
//!   elevation relaunch, the completion toast;
//! * [`lifecycle`] owns the window: the coalesced wake drain, the tray, the
//!   navigation-rail transitions and the palette's command catalogue.

pub(crate) mod commands;
pub(crate) mod events;
pub(crate) mod lifecycle;
pub(crate) mod route;
