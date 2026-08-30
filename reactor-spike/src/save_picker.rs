//! Owned Windows Common Item Dialog support for native report export.
//!
//! [`show_export_save_picker`] is deliberately synchronous and must be called
//! directly on Reactor's WinUI dispatcher thread in response to an explicit
//! user action. It obtains only that GUI thread's active window and refuses to
//! continue unless the HWND is a live window owned by both the current process
//! and current thread. It never enumerates windows, matches titles, guesses an
//! HWND, or falls back to an unowned dialog.
//!
//! Keep report rendering and file I/O off the UI thread. The intended order is:
//!
//! 1. call the picker on the UI thread;
//! 2. treat [`SavePickerOutcome::Cancelled`] as a normal no-op;
//! 3. move the selected [`ValidatedExportPath`] and rendered bytes to a worker;
//! 4. write on that worker, reporting completion back to Reactor.
//!
//! Path validation mirrors the Store 2.5.8 export boundary: Documents,
//! Desktop, Downloads, the application's roaming-data directories, and Temp
//! (with an application-owned report filename) are accepted. The returned path
//! is the canonical path that was checked, not the original picker string.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use wfdiag_native_export::ReportFormat;
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::SystemInformation::GetSystemTime;
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_RoamingAppData,
    FOS_FORCEFILESYSTEM, FOS_NOREADONLYRETURN, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST,
    FOS_STRICTFILETYPES, FileSaveDialog, IFileSaveDialog, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};
use windows::core::{HRESULT, PCWSTR};

const REPORT_PREFIX: &str = "wf-diagnostics-";

/// UTC calendar date used by Store 2.5.8's ISO-date filename convention.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportUtcDate {
    year: u16,
    month: u16,
    day: u16,
}

impl ExportUtcDate {
    /// Construct a validated Gregorian calendar date.
    pub fn new(year: u16, month: u16, day: u16) -> Result<Self, SavePickerError> {
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let maximum_day = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => 0,
        };
        if year == 0 || day == 0 || day > maximum_day {
            return Err(SavePickerError::InvalidUtcDate { year, month, day });
        }
        Ok(Self { year, month, day })
    }

    fn current() -> Result<Self, SavePickerError> {
        // SAFETY: GetSystemTime returns a fully initialized value and has no
        // pointer parameters or caller-owned lifetime requirements.
        let value = unsafe { GetSystemTime() };
        Self::new(value.wYear, value.wMonth, value.wDay)
    }
}

/// Stable Common Item Dialog filter metadata for one shipping report format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportFilterSpec {
    pub display_name: &'static str,
    pub pattern: &'static str,
    pub extension: &'static str,
}

/// Return the exact single-format filter used for a report export.
#[must_use]
pub const fn export_filter_spec(format: ReportFormat) -> ExportFilterSpec {
    match format {
        ReportFormat::Json => ExportFilterSpec {
            display_name: "JSON",
            pattern: "*.json",
            extension: "json",
        },
        ReportFormat::Text => ExportFilterSpec {
            display_name: "TXT",
            pattern: "*.txt",
            extension: "txt",
        },
        ReportFormat::Html => ExportFilterSpec {
            display_name: "HTML",
            pattern: "*.html",
            extension: "html",
        },
    }
}

/// Store 2.5.8-compatible suggested filename (`Date.toISOString()` date).
#[must_use]
pub fn suggested_export_filename(format: ReportFormat, date: ExportUtcDate) -> String {
    format!(
        "{REPORT_PREFIX}{:04}-{:02}-{:02}.{}",
        date.year,
        date.month,
        date.day,
        export_filter_spec(format).extension
    )
}

/// Canonical, policy-validated destination selected by the user.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedExportPath {
    path: PathBuf,
    format: ReportFormat,
}

impl ValidatedExportPath {
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn format(&self) -> ReportFormat {
        self.format
    }

    #[must_use]
    pub fn into_path(self) -> PathBuf {
        self.path
    }
}

/// A user cancellation is intentionally not represented as an error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SavePickerOutcome {
    Cancelled,
    Selected(ValidatedExportPath),
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

fn resolve_current_thread_owner() -> Result<HWND, SavePickerError> {
    // SAFETY: These functions take no pointers. HWND validity and ownership
    // are explicitly checked before the handle is used.
    let owner = unsafe { GetActiveWindow() };
    if owner.0.is_null() {
        return Err(SavePickerError::NoActiveOwner);
    }
    // SAFETY: IsWindow only observes the handle table.
    if !unsafe { IsWindow(Some(owner)) }.as_bool() {
        return Err(SavePickerError::InvalidOwner {
            reason: "GetActiveWindow returned a stale HWND",
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
    // SAFETY: These functions take no arguments and return numeric IDs.
    let current_thread = unsafe { GetCurrentThreadId() };
    let current_process = unsafe { GetCurrentProcessId() };
    if owner_thread != current_thread {
        return Err(SavePickerError::InvalidOwner {
            reason: "the HWND does not belong to the current UI thread",
        });
    }
    if owner_process != current_process {
        return Err(SavePickerError::InvalidOwner {
            reason: "the HWND does not belong to this process",
        });
    }
    Ok(owner)
}

fn known_folder(folder: &windows::core::GUID) -> Result<PathBuf, SavePickerError> {
    // SAFETY: SHGetKnownFolderPath allocates the returned NUL-terminated path
    // with CoTaskMemAlloc. The allocation is copied and freed below.
    let raw = unsafe { SHGetKnownFolderPath(folder, KF_FLAG_DEFAULT, None) }
        .map_err(windows_error("resolving an allowed export folder"))?;
    let value = unsafe { raw.to_string() }.map_err(|_| SavePickerError::InvalidWindowsPath);
    // SAFETY: raw came from SHGetKnownFolderPath and is freed exactly once.
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    value.map(PathBuf::from)
}

#[derive(Debug)]
struct ExportPathPolicy {
    allowed_roots: Vec<PathBuf>,
    temp_root: Option<PathBuf>,
}

impl ExportPathPolicy {
    fn current_user() -> Result<Self, SavePickerError> {
        let roaming = known_folder(&FOLDERID_RoamingAppData)?;
        let mut allowed_roots = vec![
            known_folder(&FOLDERID_Documents)?,
            known_folder(&FOLDERID_Desktop)?,
            known_folder(&FOLDERID_Downloads)?,
            roaming.join("wfdiag-tauri"),
            roaming.join("com.windowsforum.diagnostics"),
        ];
        let temp_root = std::env::var_os("TEMP")
            .or_else(|| std::env::var_os("TMP"))
            .map(PathBuf::from);
        if let Some(temp) = &temp_root {
            allowed_roots.push(temp.clone());
        }
        Ok(Self {
            allowed_roots,
            temp_root,
        })
    }
}

fn filename_is_temp_safe(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    let report_extension =
        lower.ends_with(".txt") || lower.ends_with(".html") || lower.ends_with(".json");
    (lower.starts_with("wfdiag_") && (lower.ends_with(".txt") || lower.ends_with(".html")))
        || (lower.starts_with(REPORT_PREFIX) && report_extension)
        || (lower.starts_with("support-package-") && report_extension)
}

fn canonical_candidate(path: &Path) -> Result<PathBuf, SavePickerError> {
    if !path.is_absolute() {
        return Err(SavePickerError::InvalidPath {
            path: path.to_path_buf(),
            reason: "the selected destination is not absolute".to_string(),
        });
    }
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|error| SavePickerError::InvalidPath {
                path: path.to_path_buf(),
                reason: error.to_string(),
            });
    }
    let parent = path.parent().ok_or_else(|| SavePickerError::InvalidPath {
        path: path.to_path_buf(),
        reason: "the selected destination has no parent directory".to_string(),
    })?;
    if !parent.is_dir() {
        return Err(SavePickerError::InvalidPath {
            path: path.to_path_buf(),
            reason: "the selected parent directory does not exist".to_string(),
        });
    }
    let filename = path
        .file_name()
        .ok_or_else(|| SavePickerError::InvalidPath {
            path: path.to_path_buf(),
            reason: "the selected destination has no filename".to_string(),
        })?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|error| SavePickerError::InvalidPath {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
    Ok(canonical_parent.join(filename))
}

fn canonical_existing_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .collect()
}

fn validate_export_path_with_policy(
    path: &Path,
    format: ReportFormat,
    policy: &ExportPathPolicy,
) -> Result<ValidatedExportPath, SavePickerError> {
    let filter = export_filter_spec(format);
    let actual_extension = path.extension().and_then(|extension| extension.to_str());
    if !actual_extension.is_some_and(|extension| extension.eq_ignore_ascii_case(filter.extension)) {
        return Err(SavePickerError::InvalidPath {
            path: path.to_path_buf(),
            reason: format!("the filename must end in .{}", filter.extension),
        });
    }

    let candidate = canonical_candidate(path)?;
    let allowed_roots = canonical_existing_roots(&policy.allowed_roots);
    if !allowed_roots.iter().any(|root| candidate.starts_with(root)) {
        return Err(SavePickerError::InvalidPath {
            path: candidate,
            reason: "choose Documents, Desktop, Downloads, application data, or Temp".to_string(),
        });
    }

    if let Some(temp) = policy
        .temp_root
        .as_ref()
        .and_then(|root| root.canonicalize().ok())
        && candidate.starts_with(&temp)
    {
        let safe = candidate
            .file_name()
            .and_then(|filename| filename.to_str())
            .is_some_and(filename_is_temp_safe);
        if !safe {
            return Err(SavePickerError::InvalidPath {
                path: candidate,
                reason: "Temp exports must use a WindowsForum Diagnostics report filename"
                    .to_string(),
            });
        }
    }

    Ok(ValidatedExportPath {
        path: candidate,
        format,
    })
}

/// Show the native save dialog with a validated Reactor UI-thread owner.
///
/// The dialog is configured for one settings-selected Store 2.5.8 format.
/// The suggested date is UTC, matching the original
/// `new Date().toISOString().split('T')[0]` behavior.
pub fn show_export_save_picker(format: ReportFormat) -> Result<SavePickerOutcome, SavePickerError> {
    show_export_save_picker_for_date(format, ExportUtcDate::current()?)
}

/// Deterministic-date form of [`show_export_save_picker`].
pub fn show_export_save_picker_for_date(
    format: ReportFormat,
    date: ExportUtcDate,
) -> Result<SavePickerOutcome, SavePickerError> {
    let owner = resolve_current_thread_owner()?;
    let filter = export_filter_spec(format);
    let display_name = wide(filter.display_name);
    let pattern = wide(filter.pattern);
    let extension = wide(filter.extension);
    let filename = wide(&suggested_export_filename(format, date));
    let title = wide("Export Diagnostic Report");
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
        validate_export_path_with_policy(&selected, format, &ExportPathPolicy::current_user()?)?;
    Ok(SavePickerOutcome::Selected(validated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("wfdiag-save-picker-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn filenames_match_store_2_5_8_for_every_format() {
        let date = ExportUtcDate::new(2026, 8, 30).unwrap();
        assert_eq!(
            suggested_export_filename(ReportFormat::Json, date),
            "wf-diagnostics-2026-08-30.json"
        );
        assert_eq!(
            suggested_export_filename(ReportFormat::Text, date),
            "wf-diagnostics-2026-08-30.txt"
        );
        assert_eq!(
            suggested_export_filename(ReportFormat::Html, date),
            "wf-diagnostics-2026-08-30.html"
        );
    }

    #[test]
    fn dates_reject_invalid_days_and_accept_leap_days() {
        assert!(ExportUtcDate::new(2024, 2, 29).is_ok());
        assert!(ExportUtcDate::new(2100, 2, 29).is_err());
        assert!(ExportUtcDate::new(2026, 4, 31).is_err());
        assert!(ExportUtcDate::new(2026, 13, 1).is_err());
    }

    #[test]
    fn filters_are_closed_and_exact() {
        assert_eq!(
            export_filter_spec(ReportFormat::Json),
            ExportFilterSpec {
                display_name: "JSON",
                pattern: "*.json",
                extension: "json"
            }
        );
        assert_eq!(export_filter_spec(ReportFormat::Text).pattern, "*.txt");
        assert_eq!(export_filter_spec(ReportFormat::Html).pattern, "*.html");
    }

    #[test]
    fn path_policy_accepts_allowed_root_and_exact_extension() {
        let root = scratch("allowed");
        let policy = ExportPathPolicy {
            allowed_roots: vec![root.clone()],
            temp_root: None,
        };
        let path = root.join("wf-diagnostics-2026-08-30.txt");
        let validated =
            validate_export_path_with_policy(&path, ReportFormat::Text, &policy).unwrap();
        assert_eq!(validated.format(), ReportFormat::Text);
        assert_eq!(
            validated.as_path(),
            path.canonicalize().unwrap_or(path.clone())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn path_policy_rejects_escape_and_mismatched_extension() {
        let root = scratch("root");
        let outside = scratch("outside");
        let policy = ExportPathPolicy {
            allowed_roots: vec![root.clone()],
            temp_root: None,
        };
        assert!(
            validate_export_path_with_policy(
                &outside.join("wf-diagnostics-2026-08-30.txt"),
                ReportFormat::Text,
                &policy
            )
            .is_err()
        );
        assert!(
            validate_export_path_with_policy(
                &root.join("wf-diagnostics-2026-08-30.json"),
                ReportFormat::Text,
                &policy
            )
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn temp_requires_an_application_owned_report_name() {
        let root = scratch("temp");
        let policy = ExportPathPolicy {
            allowed_roots: vec![root.clone()],
            temp_root: Some(root.clone()),
        };
        assert!(
            validate_export_path_with_policy(&root.join("random.txt"), ReportFormat::Text, &policy)
                .is_err()
        );
        assert!(
            validate_export_path_with_policy(
                &root.join("wf-diagnostics-2026-08-30.txt"),
                ReportFormat::Text,
                &policy
            )
            .is_ok()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
