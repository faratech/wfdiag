//! One module per navigable page.
//!
//! Every screen is the same three files: `state.rs` (its view state and its
//! message enum), `update.rs` (`update` for its own messages, `on_app_event`
//! for the engine facts it owns) and `view.rs` (`view`, taking only the
//! screen's own state plus [`crate::app::screen::ShellEnv`]).
//!
//! **Pages never nest ScrollViewers.** The shell's page host owns the one
//! scroll viewer in the window and shows its bar
//! ([`crate::app::policy::page_host_scrolls`]). A viewer nested inside it is
//! measured with unbounded height, so it never scrolls, and everything past
//! the fold becomes unreachable by pointer, keyboard and UI Automation alike
//! (#193). Pages return plain content; a page that needs an *inner* scroll
//! region for one bounded panel must give that panel an explicit `max_height`.

pub(crate) mod ai;
pub(crate) mod diagnostics;
pub(crate) mod history;
pub(crate) mod issues;
pub(crate) mod monitor;
pub(crate) mod processes;
