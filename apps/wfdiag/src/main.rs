#![windows_subsystem = "windows"]

//! Native WinUI 3 shell entry point. Everything the process does lives in
//! the modules below; this file only starts it.

mod app;
mod dialogs;
mod fixtures;
mod platform;
mod screens;
mod widgets;

use crate::app::WfdiagShell;
use crate::app::consts::{APP_VERSION, ELEVATED_RELAUNCH_FLAG};
use crate::fixtures::knobs::write_version_probe_if_requested;
use crate::platform::instance;
use std::time::Duration;
use windows_reactor::App;

fn main() {
    // Installed before anything else: a WinUI or Windows App Runtime bootstrap
    // panic has no window to report itself in, and the release profile aborts
    // on panic, so without this hook the process would exit silently.
    crate::platform::crash::install_panic_hook(APP_VERSION);

    // This probe must remain ahead of App::run_component so version validation
    // never initializes WinUI or creates a visible window. Without the
    // `validation` feature it is a compile-time `false` (#212).
    if write_version_probe_if_requested() {
        return;
    }
    // The one deliberate command-line read outside `fixtures::knobs`: this is
    // production behaviour, not a validation knob (#186).
    let elevated_handoff = std::env::args_os()
        .skip(1)
        .any(|argument| argument == std::ffi::OsStr::new(ELEVATED_RELAUNCH_FLAG));
    let instance = if elevated_handoff {
        instance::acquire_for_relaunch("com.windowsforum.diagnostics", Duration::from_secs(30))
    } else {
        instance::acquire("com.windowsforum.diagnostics")
    };
    let _instance_watch = match instance {
        instance::SingleInstanceDecision::Primary(watch) => watch,
        // #188: the single-instance lock could not be created at all (an SDDL
        // or CreateMutexW failure, not "another copy owns it"). Exiting here
        // used to be silent and total: the user double-clicks the app and
        // nothing whatsoever happens. Starting anyway is the strictly better
        // failure mode — the worst case is a second window the user can
        // close, whereas the old behaviour made the app unlaunchable. The
        // elevated-relaunch hand-off takes the same branch on purpose: by the
        // time it runs the unelevated original is already exiting, so there is
        // no second interactive instance to protect against, and refusing to
        // start would strand the user with no window after a UAC prompt they
        // just approved.
        instance::SingleInstanceDecision::PrimaryWithoutLock { watch, reason } => {
            crate::platform::crash::show_startup_warning(&format!(
                "WindowsForum Diagnostics could not create its single-instance lock.\n\n\
                 Details: {reason}\n\n\
                 The app will start anyway, because not starting would leave you with no \
                 window at all. If another copy of WindowsForum Diagnostics is already \
                 running you may now see two windows; close the extra one."
            ));
            watch
        }
        instance::SingleInstanceDecision::Secondary => return,
    };
    if let Err(error) = App::run_component::<WfdiagShell>(()) {
        crate::platform::crash::report_runtime_start_failure(APP_VERSION, &error);
    }
}
