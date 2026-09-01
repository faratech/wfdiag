//! Owned Windows Common Item Dialog support for native report export.
//!
//! # The dialog never runs on the UI thread (#140, #196)
//!
//! [`IFileSaveDialog::Show`] is modal: it runs its own message loop until the
//! user answers. Calling it from inside `Component::update` froze the whole
//! shell — no repaint, no wake drain, no scan progress — for as long as the
//! dialog was open (#140). [`SavePickerHost`] therefore runs every picker on a
//! dedicated single-threaded apartment (`COINIT_APARTMENTTHREADED`) thread and
//! posts the answer back as an ordinary `Message`, so the UI thread keeps
//! publishing while the user browses.
//!
//! Moving off the UI thread also fixes the owner (#196): `GetActiveWindow`
//! is per-GUI-thread and returns null on the picker thread, so the owner is
//! now [`crate::platform::instance::registered_main_window_hwnd`] — the exact
//! HWND Reactor registered — validated as a live window of this process. It
//! never enumerates windows, matches titles, guesses an HWND, or falls back to
//! an unowned dialog.
//!
//! Keep report rendering and file I/O off the UI thread too. The intended
//! order is:
//!
//! 1. spawn the picker with [`SavePickerHost::request`];
//! 2. treat [`SavePickerOutcome::Cancelled`] as a normal no-op;
//! 3. move the selected [`ValidatedExportPath`] and rendered bytes to a worker;
//! 4. write on that worker, reporting completion back to Reactor.
//!
//! The destination policy itself — allowed roots, extensions, Temp filenames,
//! suggested names, and the support-package siblings — is shared with the Tauri
//! shell and lives in [`wfdiag_native_export::path_policy`]. This module owns
//! only the Windows half: resolving the current user's known folders, driving
//! `IFileSaveDialog`, and reading the system clock.

use std::collections::HashMap;
use std::sync::Mutex;

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use wfdiag_native_export::ReportFormat;
use wfdiag_native_export::path_policy::{
    ExportPathError, ExportPathPolicy, ExportTimeError, ExportUtcDate, ExportUtcTimestamp,
    export_filter_spec, process_temp_dir, suggested_export_filename,
    suggested_support_package_filename, validate_export_path_with_policy,
    validate_support_package_paths_with_policy,
};
pub use wfdiag_native_export::path_policy::{ValidatedExportPath, ValidatedSupportPackagePaths};
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::SystemInformation::GetSystemTime;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_RoamingAppData,
    FOS_FORCEFILESYSTEM, FOS_NOREADONLYRETURN, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST,
    FOS_STRICTFILETYPES, FileSaveDialog, IFileSaveDialog, IShellItem, KF_FLAG_DEFAULT,
    SHCreateItemFromParsingName, SHGetKnownFolderPath, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};
use windows::core::{HRESULT, PCWSTR};

/// A user cancellation is intentionally not represented as an error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SavePickerOutcome {
    Cancelled,
    Selected(ValidatedExportPath),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupportPackagePickerOutcome {
    Cancelled,
    Selected(ValidatedSupportPackagePaths),
}

#[derive(Debug)]
pub enum SavePickerError {
    NoActiveOwner,
    InvalidOwner {
        reason: &'static str,
    },
    InvalidUtcDate {
        year: u16,
        month: u16,
        day: u16,
    },
    InvalidUtcTime {
        hour: u16,
        minute: u16,
        second: u16,
        millisecond: u16,
    },
    InvalidPath {
        path: PathBuf,
        reason: String,
    },
    Windows {
        operation: &'static str,
        source: windows::core::Error,
    },
    InvalidWindowsPath,
}

impl From<ExportTimeError> for SavePickerError {
    fn from(error: ExportTimeError) -> Self {
        match error {
            ExportTimeError::Date { year, month, day } => Self::InvalidUtcDate { year, month, day },
            ExportTimeError::Time {
                hour,
                minute,
                second,
                millisecond,
            } => Self::InvalidUtcTime {
                hour,
                minute,
                second,
                millisecond,
            },
        }
    }
}

impl From<ExportPathError> for SavePickerError {
    fn from(error: ExportPathError) -> Self {
        Self::InvalidPath {
            path: error.path().to_path_buf(),
            reason: error.to_string(),
        }
    }
}

impl fmt::Display for SavePickerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoActiveOwner => formatter.write_str(
                "no active Reactor window belongs to the current UI thread; export was not opened",
            ),
            Self::InvalidOwner { reason } => {
                write!(
                    formatter,
                    "the active export-dialog owner is invalid: {reason}"
                )
            }
            Self::InvalidUtcDate { year, month, day } => {
                write!(
                    formatter,
                    "invalid UTC export date {year:04}-{month:02}-{day:02}"
                )
            }
            Self::InvalidUtcTime {
                hour,
                minute,
                second,
                millisecond,
            } => write!(
                formatter,
                "invalid UTC export time {hour:02}:{minute:02}:{second:02}.{millisecond:03}"
            ),
            Self::InvalidPath { path, reason } => {
                write!(
                    formatter,
                    "export path {} is not allowed: {reason}",
                    path.display()
                )
            }
            Self::Windows { operation, source } => {
                write!(formatter, "{operation} failed: {source}")
            }
            Self::InvalidWindowsPath => {
                formatter.write_str("Windows returned an invalid Unicode export path")
            }
        }
    }
}

impl Error for SavePickerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Windows { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn windows_error(operation: &'static str) -> impl FnOnce(windows::core::Error) -> SavePickerError {
    move |source| SavePickerError::Windows { operation, source }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Read the current UTC date from the system clock.
fn current_export_utc_date() -> Result<ExportUtcDate, SavePickerError> {
    // SAFETY: GetSystemTime returns a fully initialized value and has no
    // pointer parameters or caller-owned lifetime requirements.
    let value = unsafe { GetSystemTime() };
    Ok(ExportUtcDate::new(value.wYear, value.wMonth, value.wDay)?)
}

/// Read the current UTC timestamp from the system clock.
fn current_export_utc_timestamp() -> Result<ExportUtcTimestamp, SavePickerError> {
    // SAFETY: GetSystemTime returns a fully initialized value.
    let value = unsafe { GetSystemTime() };
    Ok(ExportUtcTimestamp::new(
        value.wYear,
        value.wMonth,
        value.wDay,
        value.wHour,
        value.wMinute,
        value.wSecond,
        value.wMilliseconds,
    )?)
}

/// Validate the owner HWND the UI thread handed to the picker thread.
///
/// #196: the old resolver called `GetActiveWindow`, which is per-GUI-thread,
/// so it produced the right answer only while the dialog ran on the UI thread
/// and produced `NoActiveOwner` whenever the shell window was not the active
/// one. The owner is now the exact registered Reactor HWND, checked here for
/// liveness and process ownership. It deliberately does NOT require the
/// current thread: the picker runs on its own STA thread by design.
fn validate_owner(owner: HWND) -> Result<HWND, SavePickerError> {
    if owner.0.is_null() {
        return Err(SavePickerError::NoActiveOwner);
    }
    // SAFETY: IsWindow only observes the handle table.
    if !unsafe { IsWindow(Some(owner)) }.as_bool() {
        return Err(SavePickerError::InvalidOwner {
            reason: "the registered shell window is no longer a live HWND",
        });
    }

    let mut owner_process = 0;
    // SAFETY: owner is live, and owner_process points to initialized storage.
    let owner_thread = unsafe { GetWindowThreadProcessId(owner, Some(&mut owner_process)) };
    if owner_thread == 0 {
        return Err(SavePickerError::InvalidOwner {
            reason: "Windows could not resolve the HWND owner",
        });
    }
    // SAFETY: This function takes no arguments and returns a numeric ID.
    let current_process = unsafe { GetCurrentProcessId() };
    if owner_process != current_process {
        return Err(SavePickerError::InvalidOwner {
            reason: "the HWND does not belong to this process",
        });
    }
    Ok(owner)
}

fn known_folder(folder: &windows::core::GUID) -> Result<PathBuf, SavePickerError> {
    // Known folders are stable for the process lifetime, and one support-package
    // export resolves them several times across policy construction and path
    // validation. Cache successes (failures stay uncached so a transient
    // shell hiccup can retry).
    static CACHE: std::sync::OnceLock<Mutex<HashMap<windows::core::GUID, PathBuf>>> =
        std::sync::OnceLock::new();
    if let Some(cached) = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|cache| cache.get(folder).cloned())
    {
        return Ok(cached);
    }

    // SAFETY: SHGetKnownFolderPath allocates the returned NUL-terminated path
    // with CoTaskMemAlloc. The allocation is copied and freed below.
    let raw = unsafe { SHGetKnownFolderPath(folder, KF_FLAG_DEFAULT, None) }
        .map_err(windows_error("resolving an allowed export folder"))?;
    let value = unsafe { raw.to_string() }.map_err(|_| SavePickerError::InvalidWindowsPath);
    // SAFETY: raw came from SHGetKnownFolderPath and is freed exactly once.
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    let resolved = value.map(PathBuf::from);
    if let Ok(resolved) = &resolved
        && let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock()
    {
        cache.insert(*folder, resolved.clone());
    }
    resolved
}

/// Resolve the shared export path policy for the signed-in user.
///
/// `dirs` treats unavailable known folders independently. Mirror that
/// behavior instead of making one missing redirected folder disable every
/// otherwise valid destination.
fn current_user_export_policy() -> ExportPathPolicy {
    ExportPathPolicy::for_user_folders(
        known_folder(&FOLDERID_Documents).ok(),
        known_folder(&FOLDERID_Desktop).ok(),
        known_folder(&FOLDERID_Downloads).ok(),
        known_folder(&FOLDERID_RoamingAppData).ok(),
        process_temp_dir(),
    )
}

/// Re-check one destination against a freshly resolved policy immediately
/// before a write.
pub fn revalidate_export_path(
    path: &ValidatedExportPath,
) -> Result<ValidatedExportPath, SavePickerError> {
    Ok(path.revalidate_with(&current_user_export_policy())?)
}

/// Re-check all three support-package destinations immediately before
/// delivery. Each file is subsequently checked again just before its own write.
pub fn revalidate_support_package_paths(
    paths: &ValidatedSupportPackagePaths,
) -> Result<ValidatedSupportPackagePaths, SavePickerError> {
    Ok(paths.revalidate_with(&current_user_export_policy())?)
}

fn default_export_directory() -> Option<PathBuf> {
    // Exact shipping fallback order from `default_export_dir()`.
    [
        known_folder(&FOLDERID_Downloads).ok(),
        known_folder(&FOLDERID_Documents).ok(),
        known_folder(&FOLDERID_Desktop).ok(),
        known_folder(&FOLDERID_RoamingAppData)
            .ok()
            .map(|path| path.join("com.windowsforum.diagnostics")),
    ]
    .into_iter()
    .flatten()
    .find(|path| path.is_dir())
}

/// Show the native save dialog owned by `owner`.
///
/// The dialog is configured for one settings-selected Store 2.5.8 format.
/// The suggested date is UTC, matching the original
/// `new Date().toISOString().split('T')[0]` behavior.
fn show_export_save_picker(
    owner: HWND,
    format: ReportFormat,
) -> Result<SavePickerOutcome, SavePickerError> {
    show_export_save_picker_for_date(owner, format, current_export_utc_date()?)
}

/// Deterministic-date form of [`show_export_save_picker`].
fn show_export_save_picker_for_date(
    owner: HWND,
    format: ReportFormat,
    date: ExportUtcDate,
) -> Result<SavePickerOutcome, SavePickerError> {
    show_save_picker(
        owner,
        format,
        &suggested_export_filename(format, date),
        "Export Diagnostic Report",
    )
}

/// Show the Store-compatible support-package picker and validate the JSON,
/// text, and HTML destinations before returning any of them.
fn show_support_package_save_picker(
    owner: HWND,
) -> Result<SupportPackagePickerOutcome, SavePickerError> {
    show_support_package_save_picker_for_timestamp(owner, current_export_utc_timestamp()?)
}

/// Deterministic-time form of [`show_support_package_save_picker`].
fn show_support_package_save_picker_for_timestamp(
    owner: HWND,
    timestamp: ExportUtcTimestamp,
) -> Result<SupportPackagePickerOutcome, SavePickerError> {
    let filename = suggested_support_package_filename(timestamp);
    match show_save_picker(
        owner,
        ReportFormat::Json,
        &filename,
        "Generate Support Package",
    )? {
        SavePickerOutcome::Cancelled => Ok(SupportPackagePickerOutcome::Cancelled),
        SavePickerOutcome::Selected(selected) => Ok(SupportPackagePickerOutcome::Selected(
            validate_support_package_paths_with_policy(&selected, &current_user_export_policy())?,
        )),
    }
}

fn show_save_picker(
    owner: HWND,
    format: ReportFormat,
    suggested_filename: &str,
    dialog_title: &str,
) -> Result<SavePickerOutcome, SavePickerError> {
    let owner = validate_owner(owner)?;
    let filter = export_filter_spec(format);
    let display_name = wide(filter.display_name);
    let pattern = wide(filter.pattern);
    let extension = wide(filter.extension);
    let filename = wide(suggested_filename);
    let title = wide(dialog_title);
    let default_directory = default_export_directory();
    let filter_spec = [COMDLG_FILTERSPEC {
        pszName: PCWSTR(display_name.as_ptr()),
        pszSpec: PCWSTR(pattern.as_ptr()),
    }];

    // SAFETY: WinUI has initialized COM on Reactor's UI thread. All strings
    // remain alive through configuration; the dialog copies them. The owner
    // was validated above as a live current-process/current-thread HWND.
    let dialog: IFileSaveDialog = unsafe {
        CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(windows_error("creating the Windows save dialog"))?
    };
    unsafe {
        dialog
            .SetFileTypes(&filter_spec)
            .map_err(windows_error("setting the export file filter"))?;
        dialog
            .SetFileTypeIndex(1)
            .map_err(windows_error("selecting the export file filter"))?;
        dialog
            .SetDefaultExtension(PCWSTR(extension.as_ptr()))
            .map_err(windows_error("setting the export extension"))?;
        // The shipping frontend passes a full suggested path rooted at
        // Downloads, then Documents, Desktop, or app data. `SetFileName`
        // alone leaves Reactor in Windows' last-used directory; force the
        // same directory when one is available. As in the frontend's
        // `suggestedExportPath` fallback, inability to resolve that optional
        // suggestion must not prevent the dialog from opening.
        if let Some(default_directory) = default_directory
            && let Some(default_directory) = default_directory.to_str()
        {
            let default_directory = wide(default_directory);
            let item: windows::core::Result<IShellItem> =
                SHCreateItemFromParsingName(PCWSTR(default_directory.as_ptr()), None);
            if let Ok(item) = item {
                let _ = dialog.SetFolder(&item);
            }
        }
        dialog
            .SetFileName(PCWSTR(filename.as_ptr()))
            .map_err(windows_error("setting the suggested export filename"))?;
        dialog
            .SetTitle(PCWSTR(title.as_ptr()))
            .map_err(windows_error("setting the export dialog title"))?;
        let existing = dialog
            .GetOptions()
            .map_err(windows_error("reading the save dialog options"))?;
        dialog
            .SetOptions(
                existing
                    | FOS_FORCEFILESYSTEM
                    | FOS_PATHMUSTEXIST
                    | FOS_OVERWRITEPROMPT
                    | FOS_NOREADONLYRETURN
                    | FOS_STRICTFILETYPES,
            )
            .map_err(windows_error("configuring the save dialog"))?;

        match dialog.Show(Some(owner)) {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
                return Ok(SavePickerOutcome::Cancelled);
            }
            Err(source) => {
                return Err(SavePickerError::Windows {
                    operation: "showing the Windows save dialog",
                    source,
                });
            }
        }
    }

    // SAFETY: GetDisplayName returns a CoTaskMem-allocated NUL-terminated
    // filesystem path for SIGDN_FILESYSPATH. It is copied and freed once.
    let raw_path = unsafe {
        dialog
            .GetResult()
            .and_then(|item| item.GetDisplayName(SIGDN_FILESYSPATH))
            .map_err(windows_error("reading the selected export path"))?
    };
    let selected = unsafe { raw_path.to_string() }.map_err(|_| SavePickerError::InvalidWindowsPath);
    unsafe { CoTaskMemFree(Some(raw_path.0.cast())) };
    let selected = PathBuf::from(selected?);
    let validated =
        validate_export_path_with_policy(&selected, format, &current_user_export_policy())?;
    Ok(SavePickerOutcome::Selected(validated))
}

// ------------------------------------------------------------- host -------

/// Which picker to show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SavePickerRequest {
    /// The single-file report export, in the settings-selected format.
    Export(ReportFormat),
    /// The three-file support package.
    SupportPackage,
}

/// What one picker answered.
///
/// Cancellation is a normal, silent outcome; only a typed failure carries
/// status text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SavePickerReply {
    /// The user dismissed the dialog. Nothing happens.
    Cancelled,
    /// A validated single-file destination.
    Export(ValidatedExportPath),
    /// Three validated support-package destinations.
    SupportPackage(ValidatedSupportPackagePaths),
    /// The dialog could not run, or the chosen path is not allowed.
    Failed(String),
}

/// One finished picker, waiting for the UI thread to collect it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavePickerCompletion {
    /// The request generation this answers, so a superseded picker is dropped.
    pub epoch: u64,
    /// Which picker ran.
    pub request: SavePickerRequest,
    /// What it answered.
    pub reply: SavePickerReply,
}

/// The completed answer, published by the picker thread and taken by the UI
/// thread on the next coalesced wake.
///
/// Reactor's component sender is `!Send`, so the picker thread cannot enqueue
/// a `Message` directly. It parks the answer here and posts the same
/// process-wide wake every other producer uses; the shell drains it beside the
/// engine's own event queue.
static COMPLETED: Mutex<Option<SavePickerCompletion>> = Mutex::new(None);

/// Take the finished picker answer, if one is waiting.
#[must_use]
pub fn take_completed_picker() -> Option<SavePickerCompletion> {
    COMPLETED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

/// Runs Common Item Dialogs off the UI thread (#140).
///
/// Each request gets its own single-threaded-apartment thread: WinUI owns the
/// UI thread's apartment, and a modal `IFileSaveDialog::Show` there blocks the
/// whole shell. The answer is parked in [`take_completed_picker`] and the host
/// is woken; the epoch lets the shell drop an answer it has already
/// superseded.
#[derive(Debug, Clone, Copy)]
pub struct SavePickerHost;

impl SavePickerHost {
    /// Start one picker. `wake` runs on the picker thread once the dialog
    /// closes and must only signal, never touch UI state.
    ///
    /// # Errors
    ///
    /// Returns a message when the thread could not be spawned, or when no
    /// registered shell window is available to own the dialog (#196).
    pub fn request(request: SavePickerRequest, epoch: u64, wake: fn()) -> Result<(), String> {
        let owner = crate::platform::instance::registered_main_window_hwnd()
            .ok_or_else(|| SavePickerError::NoActiveOwner.to_string())?;
        // HWND is not Send. The numeric handle is, and it is revalidated on
        // the picker thread against this process's live window list.
        let owner = owner.0 as usize;
        std::thread::Builder::new()
            .name("wfdiag-save-picker".to_string())
            .spawn(move || {
                let owner = HWND(owner as *mut std::ffi::c_void);
                let reply = {
                    let _apartment = SingleThreadedApartment::enter();
                    run_picker(owner, request)
                };
                *COMPLETED
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(SavePickerCompletion {
                        epoch,
                        request,
                        reply,
                    });
                wake();
            })
            .map(|_| ())
            .map_err(|error| format!("the export dialog thread could not start: {error}"))
    }
}

fn run_picker(owner: HWND, request: SavePickerRequest) -> SavePickerReply {
    match request {
        SavePickerRequest::Export(format) => match show_export_save_picker(owner, format) {
            Ok(SavePickerOutcome::Cancelled) => SavePickerReply::Cancelled,
            Ok(SavePickerOutcome::Selected(path)) => SavePickerReply::Export(path),
            Err(error) => SavePickerReply::Failed(error.to_string()),
        },
        SavePickerRequest::SupportPackage => match show_support_package_save_picker(owner) {
            Ok(SupportPackagePickerOutcome::Cancelled) => SavePickerReply::Cancelled,
            Ok(SupportPackagePickerOutcome::Selected(paths)) => {
                SavePickerReply::SupportPackage(paths)
            }
            Err(error) => SavePickerReply::Failed(error.to_string()),
        },
    }
}

/// COM apartment lifetime for one picker thread.
///
/// The Common Item Dialog requires an STA. `CoUninitialize` is paired in
/// `Drop` so the apartment is torn down even if the dialog path returns early.
struct SingleThreadedApartment {
    initialized: bool,
}

impl SingleThreadedApartment {
    fn enter() -> Self {
        // SAFETY: this thread is freshly spawned and has no apartment yet.
        // RPC_E_CHANGED_MODE cannot occur here; any other failure simply means
        // the dialog will report its own COM error instead.
        let result = unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            )
        };
        Self {
            initialized: result.is_ok(),
        }
    }
}

impl Drop for SingleThreadedApartment {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: paired with the successful CoInitializeEx above, on the
            // same thread.
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}
