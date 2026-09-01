#![windows_subsystem = "windows"]

//! Native WinUI 3 shell entry point. Everything the process does lives in
//! the modules below; this file only starts it.

mod ai;
mod app;
mod dialogs;
mod fixtures;
mod platform;
mod screens;
mod widgets;

use crate::app::WfdiagShell;
use crate::app::consts::{APP_VERSION, ELEVATED_RELAUNCH_FLAG};
use crate::app::policy::write_version_probe_if_requested;
use crate::platform::instance;
use std::time::Duration;
use windows_reactor::App;

fn main() {
    // Installed before anything else: a WinUI or Windows App Runtime bootstrap
    // panic has no window to report itself in, and the release profile aborts
    // on panic, so without this hook the process would exit silently.
    crate::platform::crash::install_panic_hook(APP_VERSION);

    // This probe must remain ahead of App::run_component so version validation
    // never initializes WinUI or creates a visible window.
    if write_version_probe_if_requested() {
        return;
    }
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
        instance::SingleInstanceDecision::Secondary => return,
    };
    if let Err(error) = App::run_component::<WfdiagShell>(()) {
        crate::platform::crash::report_runtime_start_failure(&error);
    }
}
