//! Startup and panic crash reporting for the native shell.
//!
//! The release profile builds with `panic = "abort"` and WinUI bootstrap
//! failures surface as an `Err` from `App::run_component`, so without this
//! module a missing `Microsoft.WindowsAppRuntime.2` framework package makes the
//! executable exit silently with no window and no log. Both paths now leave a
//! record on disk and put one modal `MessageBoxW` in front of the user.
//!
//! Everything here runs on a process that is already failing: no allocation is
//! required for the message box path to work, every filesystem step is
//! best-effort, and nothing in this module may panic on its own.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_ICONWARNING, MB_OK, MESSAGEBOX_STYLE, MessageBoxW,
};
use windows::core::HSTRING;

/// Modal title shared by every crash surface.
const CRASH_DIALOG_TITLE: &str = "WindowsForum Diagnostics";

/// Framework package the shell depends on for its WinUI 3 runtime.
const APP_RUNTIME_PACKAGE: &str = "Microsoft.WindowsAppRuntime.2";

/// Upper bound on the panic payload copied into a crash record.
///
/// Crash logs are read by hand and mailed in support packages, so a runaway
/// `Display` implementation must not be able to fill the user's disk.
const MAX_PANIC_MESSAGE_CHARS: usize = 2_000;

/// Number of filename suffixes tried when a crash file already exists.
const MAX_CRASH_FILE_ATTEMPTS: u32 = 16;

/// Install the process-wide panic hook.
///
/// Call this as the first statement in `main`, before any Windows App SDK or
/// Reactor initialization, so a bootstrap panic is still reported. The hook
/// writes a crash record, shows a modal, and then returns — the profile's own
/// panic strategy (abort in release) takes over exactly as it did before.
pub(crate) fn install_panic_hook(app_version: &'static str) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let record = crash_record(app_version, info);
        let written = write_crash_record(&record);
        show_panic_dialog(written.as_deref());
        previous(info);
    }));
}

/// Report a WinUI 3 / Windows App Runtime startup failure and exit.
///
/// `App::run_component` returns `Err` when the framework package the shell
/// depends on cannot be resolved or activated. That happens before any window
/// exists, so a modal is the only surface the user can see.
pub(crate) fn report_runtime_start_failure(error: &dyn std::fmt::Display) -> ! {
    let detail = truncate_chars(&error.to_string(), MAX_PANIC_MESSAGE_CHARS);
    let body = format!(
        "WindowsForum Diagnostics could not start its Windows App Runtime host.\n\n\
         The {APP_RUNTIME_PACKAGE} framework package is required. Install or repair it, \
         then start the app again.\n\n\
         Details: {detail}"
    );
    show_message_box(&body);
    std::process::exit(2);
}

/// Assemble the on-disk crash record for one panic.
fn crash_record(app_version: &str, info: &PanicHookInfo<'_>) -> String {
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>").to_string();
    let location = info.location().map_or_else(
        || "<unknown location>".to_string(),
        |location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        },
    );
    let message = truncate_chars(&panic_message(info), MAX_PANIC_MESSAGE_CHARS);

    let mut record = String::new();
    let _ = writeln!(record, "timestamp: {}", timestamp_seconds());
    let _ = writeln!(record, "version: {app_version}");
    let _ = writeln!(record, "thread: {thread_name}");
    let _ = writeln!(record, "location: {location}");
    let _ = writeln!(record, "panic: {message}");
    record
}

/// Extract the panic payload as text without ever panicking itself.
fn panic_message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Seconds since the Unix epoch, or `0` when the clock is before it.
fn timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Truncate on a character boundary so a huge payload cannot bloat the record.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if out.chars().count() < text.chars().count() {
        out.push_str(" […truncated]");
    }
    out
}

/// `%LOCALAPPDATA%\WFDiag\logs`, when the environment exposes it.
///
/// This is the one production environment read outside `fixtures::knobs` (see
/// that module's invariant, #186). It stays here deliberately: the caller is a
/// panic hook on an already-failing process, where a plain environment lookup
/// is markedly safer than calling into shell32 for a known-folder path.
fn crash_log_directory() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    if local.is_empty() {
        return None;
    }
    Some(Path::new(&local).join("WFDiag").join("logs"))
}

/// Write one crash record, returning the path when the write succeeded.
///
/// `create_new` is deliberate: it fails rather than opening an existing path,
/// which is what keeps this from ever writing through a planted symlink.
fn write_crash_record(record: &str) -> Option<PathBuf> {
    let directory = crash_log_directory()?;
    std::fs::create_dir_all(&directory).ok()?;
    let stamp = timestamp_seconds();
    for attempt in 0..MAX_CRASH_FILE_ATTEMPTS {
        let name = if attempt == 0 {
            format!("crash-{stamp}.log")
        } else {
            format!("crash-{stamp}-{attempt}.log")
        };
        let path = directory.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                return file.write_all(record.as_bytes()).ok().map(|()| path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return None,
        }
    }
    None
}

/// Show the panic modal, naming the crash log when one was written.
fn show_panic_dialog(log_path: Option<&Path>) {
    let body = match log_path {
        Some(path) => format!(
            "WindowsForum Diagnostics stopped unexpectedly.\n\n\
             A crash log was saved to:\n{}",
            path.display()
        ),
        None => "WindowsForum Diagnostics stopped unexpectedly.\n\n\
             A crash log could not be written."
            .to_string(),
    };
    show_message_box(&body);
}

/// One modal warning box for a degraded — but survivable — startup.
///
/// Shared with the single-instance path (#188, #189), which needs to tell the
/// user something before any window exists and must not be mistaken for the
/// fatal crash modal above.
pub(crate) fn show_startup_warning(body: &str) {
    show_message_box_with_style(body, MB_ICONWARNING | MB_OK);
}

/// One modal error box on the calling thread.
fn show_message_box(body: &str) {
    show_message_box_with_style(body, MB_ICONERROR | MB_OK);
}

fn show_message_box_with_style(body: &str, style: MESSAGEBOX_STYLE) {
    let text = HSTRING::from(body);
    let caption = HSTRING::from(CRASH_DIALOG_TITLE);
    // SAFETY: both strings outlive the synchronous call, and a null owner
    // window is documented as "no owner" rather than an invalid handle.
    unsafe {
        MessageBoxW(None, &text, &caption, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_marks_truncated_payloads() {
        let long = "x".repeat(MAX_PANIC_MESSAGE_CHARS + 10);
        let truncated = truncate_chars(&long, MAX_PANIC_MESSAGE_CHARS);
        assert!(truncated.ends_with("[…truncated]"));
        assert!(truncated.chars().count() < long.chars().count() + 16);
    }

    #[test]
    fn truncate_chars_leaves_short_payloads_alone() {
        assert_eq!(truncate_chars("boom", MAX_PANIC_MESSAGE_CHARS), "boom");
    }

    #[test]
    fn crash_log_directory_is_under_local_app_data() {
        let Some(directory) = crash_log_directory() else {
            return;
        };
        assert!(directory.ends_with(Path::new("WFDiag").join("logs")));
    }
}
