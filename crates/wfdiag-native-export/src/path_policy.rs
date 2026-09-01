//! Host-neutral export destination policy.
//!
//! Both shipping shells answer the same three questions before a report is
//! written: what should the suggested filename be, which directories may a
//! user save into, and does the chosen filename belong to this application?
//! The answers are pure path arithmetic, so they live here rather than beside
//! either shell's native file dialog.
//!
//! Resolving the *current user's* Documents/Desktop/Downloads/app-data folders
//! is deliberately left to the host (the `WinUI` shell uses cached
//! `SHGetKnownFolderPath` results; the Tauri shell uses `dirs`). A host hands
//! the resolved folders to [`ExportPathPolicy::for_user_folders`] and every
//! rule below is shared.
//!
//! Nothing here touches a UI framework, and the only filesystem access is the
//! canonicalization required to defend against junction/symlink replacement.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::renderer::ReportFormat;

const REPORT_PREFIX: &str = "wf-diagnostics-";

/// The application's report extensions, lowercase and dot-prefixed.
const REPORT_EXTENSIONS: [&str; 3] = [".txt", ".html", ".json"];

/// UTC calendar date used by Store 2.5.8's ISO-date filename convention.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportUtcDate {
    year: u16,
    month: u16,
    day: u16,
}

/// UTC timestamp used by Store 2.5.8's support-package filename convention
/// (`Date.toISOString().replace(/[:.]/g, '-')`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportUtcTimestamp {
    date: ExportUtcDate,
    hour: u16,
    minute: u16,
    second: u16,
    millisecond: u16,
}

/// The only two ways a caller-supplied instant can be rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportTimeError {
    Date {
        year: u16,
        month: u16,
        day: u16,
    },
    Time {
        hour: u16,
        minute: u16,
        second: u16,
        millisecond: u16,
    },
}

impl fmt::Display for ExportTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Date { year, month, day } => {
                write!(
                    formatter,
                    "invalid UTC export date {year:04}-{month:02}-{day:02}"
                )
            }
            Self::Time {
                hour,
                minute,
                second,
                millisecond,
            } => write!(
                formatter,
                "invalid UTC export time {hour:02}:{minute:02}:{second:02}.{millisecond:03}"
            ),
        }
    }
}

impl Error for ExportTimeError {}

impl ExportUtcTimestamp {
    /// Construct a validated UTC timestamp.
    ///
    /// # Errors
    /// Returns [`ExportTimeError`] when the date is not a real Gregorian date
    /// or a time component is out of range.
    pub fn new(
        year: u16,
        month: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millisecond: u16,
    ) -> Result<Self, ExportTimeError> {
        let date = ExportUtcDate::new(year, month, day)?;
        if hour > 23 || minute > 59 || second > 59 || millisecond > 999 {
            return Err(ExportTimeError::Time {
                hour,
                minute,
                second,
                millisecond,
            });
        }
        Ok(Self {
            date,
            hour,
            minute,
            second,
            millisecond,
        })
    }

    #[must_use]
    pub const fn date(&self) -> ExportUtcDate {
        self.date
    }
}

impl ExportUtcDate {
    /// Construct a validated Gregorian calendar date.
    ///
    /// # Errors
    /// Returns [`ExportTimeError::Date`] for a year of zero, an unknown month,
    /// or a day outside that month (leap years included).
    pub fn new(year: u16, month: u16, day: u16) -> Result<Self, ExportTimeError> {
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
            return Err(ExportTimeError::Date { year, month, day });
        }
        Ok(Self { year, month, day })
    }
}

/// Stable file-dialog filter metadata for one shipping report format.
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

/// Store 2.5.8-compatible support-package JSON filename.
#[must_use]
pub fn suggested_support_package_filename(timestamp: ExportUtcTimestamp) -> String {
    format!(
        "support-package-{:04}-{:02}-{:02}T{:02}-{:02}-{:02}-{:03}Z.json",
        timestamp.date.year,
        timestamp.date.month,
        timestamp.date.day,
        timestamp.hour,
        timestamp.minute,
        timestamp.second,
        timestamp.millisecond,
    )
}

/// Why one destination was refused.
///
/// The variants are deliberately fine-grained: each shell renders its own
/// user-facing wording, and the Tauri command surface maps a missing parent
/// directory onto a distinct wire error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExportPathErrorKind {
    NotAbsolute,
    NoParent,
    ParentMissing,
    NoFilename,
    Canonicalize(String),
    /// The filename must end in the one extension for the requested format.
    ExtensionMismatch {
        expected: &'static str,
    },
    /// The filename must end in one of the application's report extensions.
    NotAReportExtension,
    OutsideAllowedRoots,
    TempFilenameNotAllowed,
    FilenameContainsPath,
    FilenameMissing,
    NotUnicode,
    NotJsonSupportPackageBase,
}

impl fmt::Display for ExportPathErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAbsolute => formatter.write_str("the selected destination is not absolute"),
            Self::NoParent => {
                formatter.write_str("the selected destination has no parent directory")
            }
            Self::ParentMissing => {
                formatter.write_str("the selected parent directory does not exist")
            }
            Self::NoFilename => formatter.write_str("the selected destination has no filename"),
            Self::Canonicalize(reason) => formatter.write_str(reason),
            Self::ExtensionMismatch { expected } => {
                write!(formatter, "the filename must end in .{expected}")
            }
            Self::NotAReportExtension => {
                formatter.write_str("filename must end in .txt, .html, or .json")
            }
            Self::OutsideAllowedRoots => formatter
                .write_str("choose Documents, Desktop, Downloads, application data, or Temp"),
            Self::TempFilenameNotAllowed => formatter
                .write_str("Temp exports must use a WindowsForum Diagnostics report filename"),
            Self::FilenameContainsPath => formatter.write_str("filename must not contain a path"),
            Self::FilenameMissing => formatter.write_str("filename is required"),
            Self::NotUnicode => formatter.write_str("the destination is not valid Unicode"),
            Self::NotJsonSupportPackageBase => {
                formatter.write_str("a support package must be based on a JSON destination")
            }
        }
    }
}

/// A refused destination together with the path that was checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportPathError {
    path: PathBuf,
    kind: ExportPathErrorKind,
}

impl ExportPathError {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, kind: ExportPathErrorKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }

    /// The path that was actually checked, which may be the canonical form of
    /// the caller's original string.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn kind(&self) -> &ExportPathErrorKind {
        &self.kind
    }
}

impl fmt::Display for ExportPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.kind, formatter)
    }
}

impl Error for ExportPathError {}

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

    /// Re-check the destination against a freshly resolved policy immediately
    /// before a write.
    ///
    /// The save picker performs the first check before report generation.
    /// Revalidation narrows the same directory-junction/symlink replacement
    /// window as Tauri's `save_results_to_file`, which validates again inside
    /// every write command.
    ///
    /// # Errors
    /// Returns [`ExportPathError`] when the destination no longer satisfies
    /// the policy.
    pub fn revalidate_with(&self, policy: &ExportPathPolicy) -> Result<Self, ExportPathError> {
        validate_export_path_with_policy(&self.path, self.format, policy)
    }
}

/// Three independently validated destinations produced from the selected
/// support-package JSON base path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedSupportPackagePaths {
    pub json: PathBuf,
    pub text: PathBuf,
    pub html: PathBuf,
}

impl ValidatedSupportPackagePaths {
    /// Re-check all three sibling destinations immediately before delivery.
    /// Each file is subsequently checked again just before its own write.
    ///
    /// # Errors
    /// Returns [`ExportPathError`] for the first sibling that no longer
    /// satisfies the policy.
    pub fn revalidate_with(&self, policy: &ExportPathPolicy) -> Result<Self, ExportPathError> {
        Ok(Self {
            json: validate_export_path_with_policy(&self.json, ReportFormat::Json, policy)?
                .into_path(),
            text: validate_export_path_with_policy(&self.text, ReportFormat::Text, policy)?
                .into_path(),
            html: validate_export_path_with_policy(&self.html, ReportFormat::Html, policy)?
                .into_path(),
        })
    }
}

/// The directories a user may save a report into, plus the Temp root that
/// carries the extra application-owned-filename rule.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExportPathPolicy {
    allowed_roots: Vec<PathBuf>,
    temp_root: Option<PathBuf>,
}

impl ExportPathPolicy {
    /// Build a policy from already-resolved roots. `temp_root` must also
    /// appear in `allowed_roots` to be reachable at all; it additionally
    /// enables the report-filename restriction inside Temp.
    #[must_use]
    pub fn from_roots(allowed_roots: Vec<PathBuf>, temp_root: Option<PathBuf>) -> Self {
        Self {
            allowed_roots,
            temp_root,
        }
    }

    /// Build the shipping policy from the current user's resolved folders.
    ///
    /// Unavailable folders are skipped independently, so one missing
    /// redirected folder never disables every other valid destination.
    #[must_use]
    pub fn for_user_folders(
        documents: Option<PathBuf>,
        desktop: Option<PathBuf>,
        downloads: Option<PathBuf>,
        roaming_app_data: Option<PathBuf>,
        temp: Option<PathBuf>,
    ) -> Self {
        let mut allowed_roots = [documents, desktop, downloads]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if let Some(roaming) = roaming_app_data {
            allowed_roots.push(roaming.join("wfdiag-tauri"));
            allowed_roots.push(roaming.join("com.windowsforum.diagnostics"));
        }
        if let Some(temp) = &temp {
            allowed_roots.push(temp.clone());
        }
        Self {
            allowed_roots,
            temp_root: temp,
        }
    }

    #[must_use]
    pub fn allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }

    #[must_use]
    pub fn temp_root(&self) -> Option<&Path> {
        self.temp_root.as_deref()
    }
}

/// The process temp directory, using the same `TEMP` then `TMP` order both
/// shells already relied on.
#[must_use]
pub fn process_temp_dir() -> Option<PathBuf> {
    std::env::var_os("TEMP")
        .or_else(|| std::env::var_os("TMP"))
        .map(PathBuf::from)
}

/// Which extensions a destination may carry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportExtensionRule {
    /// Exactly the one extension belonging to this report format.
    Format(ReportFormat),
    /// Any of the application's report extensions, used where the caller has
    /// not selected a format (the Tauri `save_results_to_file` boundary).
    AnyReport,
}

/// Does this filename belong to a report this application generates?
///
/// Temp is shared with every other program on the machine, so an export there
/// must additionally look like one of our own files.
// `lower` is already ASCII-lowercased, so these suffix checks are the
// case-insensitive comparison the lint asks for.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
#[must_use]
pub fn filename_is_temp_safe(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    let text_or_html = lower.ends_with(".txt") || lower.ends_with(".html");
    let report_extension = text_or_html || lower.ends_with(".json");
    (lower.starts_with("wfdiag_") && text_or_html)
        || (lower.starts_with(REPORT_PREFIX) && report_extension)
        || (lower.starts_with("support-package-") && report_extension)
}

/// Every allowed save directory — not just Temp — is restricted to the
/// application's own report extensions. Without this, Documents/Desktop/
/// Downloads/app data would accept any filename and extension a caller
/// supplied.
#[must_use]
pub fn has_report_extension(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|lower| {
            REPORT_EXTENSIONS
                .iter()
                .any(|extension| lower.ends_with(extension))
        })
}

/// Accept a bare report filename with no directory component.
///
/// Used where a caller supplies only the name half of a suggested path, so a
/// traversal segment must never survive into the joined result.
///
/// # Errors
/// Returns [`ExportPathError`] when the value carries a path, is empty, or
/// does not use a report extension.
pub fn safe_report_filename(filename: &str) -> Result<&str, ExportPathError> {
    let name = Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ExportPathError::new(filename, ExportPathErrorKind::FilenameMissing))?;

    if name != filename || name.contains(['/', '\\']) {
        return Err(ExportPathError::new(
            filename,
            ExportPathErrorKind::FilenameContainsPath,
        ));
    }
    if !has_report_extension(Path::new(name)) {
        return Err(ExportPathError::new(
            filename,
            ExportPathErrorKind::NotAReportExtension,
        ));
    }
    Ok(name)
}

/// Resolve the path that will actually be written.
///
/// An existing destination is canonicalized outright. A new one canonicalizes
/// its parent and rejoins the filename, which is what closes the
/// junction/symlink swap window without requiring the file to exist first.
///
/// # Errors
/// Returns [`ExportPathError`] for a relative path, a missing or
/// non-canonicalizable parent, or a path with no filename.
pub fn canonical_candidate(path: &Path) -> Result<PathBuf, ExportPathError> {
    if !path.is_absolute() {
        return Err(ExportPathError::new(path, ExportPathErrorKind::NotAbsolute));
    }
    if path.exists() {
        return path.canonicalize().map_err(|error| {
            ExportPathError::new(path, ExportPathErrorKind::Canonicalize(error.to_string()))
        });
    }
    let parent = path
        .parent()
        .ok_or_else(|| ExportPathError::new(path, ExportPathErrorKind::NoParent))?;
    if !parent.is_dir() {
        return Err(ExportPathError::new(
            path,
            ExportPathErrorKind::ParentMissing,
        ));
    }
    let filename = path
        .file_name()
        .ok_or_else(|| ExportPathError::new(path, ExportPathErrorKind::NoFilename))?;
    let canonical_parent = parent.canonicalize().map_err(|error| {
        ExportPathError::new(path, ExportPathErrorKind::Canonicalize(error.to_string()))
    })?;
    Ok(canonical_parent.join(filename))
}

fn canonical_existing_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    // Roots are stable for an `ExportPathPolicy`'s lifetime, and one
    // support-package export validates several paths against the same policy.
    // Cache canonicalization per raw root; failures stay uncached so a
    // transient shell hiccup can retry.
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    roots
        .iter()
        .filter_map(|root| {
            if let Some(cached) = cache.lock().ok().and_then(|cache| cache.get(root).cloned()) {
                return Some(cached);
            }
            let canonical = root.canonicalize().ok()?;
            if let Ok(mut cache) = cache.lock() {
                cache.insert(root.clone(), canonical.clone());
            }
            Some(canonical)
        })
        .collect()
}

/// Validate one destination against the policy, returning the canonical path
/// that was checked.
///
/// # Errors
/// Returns [`ExportPathError`] for a wrong extension, an unresolvable path, a
/// destination outside every allowed root, or a Temp destination that does not
/// use an application-owned report filename.
pub fn validate_report_path_with_policy(
    path: &Path,
    rule: ExportExtensionRule,
    policy: &ExportPathPolicy,
) -> Result<PathBuf, ExportPathError> {
    match rule {
        ExportExtensionRule::Format(format) => {
            let expected = export_filter_spec(format).extension;
            let matches = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case(expected));
            if !matches {
                return Err(ExportPathError::new(
                    path,
                    ExportPathErrorKind::ExtensionMismatch { expected },
                ));
            }
        }
        ExportExtensionRule::AnyReport => {
            if !has_report_extension(path) {
                return Err(ExportPathError::new(
                    path,
                    ExportPathErrorKind::NotAReportExtension,
                ));
            }
        }
    }

    let candidate = canonical_candidate(path)?;
    let allowed_roots = canonical_existing_roots(policy.allowed_roots());
    if !allowed_roots.iter().any(|root| candidate.starts_with(root)) {
        return Err(ExportPathError::new(
            candidate,
            ExportPathErrorKind::OutsideAllowedRoots,
        ));
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
            return Err(ExportPathError::new(
                candidate,
                ExportPathErrorKind::TempFilenameNotAllowed,
            ));
        }
    }

    Ok(candidate)
}

/// Format-typed form of [`validate_report_path_with_policy`].
///
/// # Errors
/// Returns [`ExportPathError`] for the same reasons as
/// [`validate_report_path_with_policy`].
pub fn validate_export_path_with_policy(
    path: &Path,
    format: ReportFormat,
    policy: &ExportPathPolicy,
) -> Result<ValidatedExportPath, ExportPathError> {
    let path = validate_report_path_with_policy(path, ExportExtensionRule::Format(format), policy)?;
    Ok(ValidatedExportPath { path, format })
}

/// Derive and validate the three support-package siblings of a selected JSON
/// destination.
///
/// # Errors
/// Returns [`ExportPathError`] when the base is not a JSON destination or when
/// any sibling fails validation.
pub fn validate_support_package_paths_with_policy(
    selected_json: &ValidatedExportPath,
    policy: &ExportPathPolicy,
) -> Result<ValidatedSupportPackagePaths, ExportPathError> {
    if selected_json.format() != ReportFormat::Json {
        return Err(ExportPathError::new(
            selected_json.as_path(),
            ExportPathErrorKind::NotJsonSupportPackageBase,
        ));
    }
    let (json_candidate, text_candidate, html_candidate) =
        shipping_support_package_sibling_paths(selected_json.as_path())?;
    Ok(ValidatedSupportPackagePaths {
        json: validate_report_path_with_policy(
            &json_candidate,
            ExportExtensionRule::Format(ReportFormat::Json),
            policy,
        )?,
        text: validate_report_path_with_policy(
            &text_candidate,
            ExportExtensionRule::Format(ReportFormat::Text),
            policy,
        )?,
        html: validate_report_path_with_policy(
            &html_candidate,
            ExportExtensionRule::Format(ReportFormat::Html),
            policy,
        )?,
    })
}

/// Match the frontend's case-sensitive `endsWith('.json')` base derivation.
/// In particular, selecting `report.JSON` produces `report.JSON.json`,
/// `report.JSON.txt`, and `report.JSON.html`; `Path::with_extension` would
/// incorrectly collapse that distinct shipping behavior to `report.*`.
///
/// # Errors
/// Returns [`ExportPathError`] when the destination is not valid Unicode.
pub fn shipping_support_package_sibling_paths(
    selected_json: &Path,
) -> Result<(PathBuf, PathBuf, PathBuf), ExportPathError> {
    let selected = selected_json
        .to_str()
        .ok_or_else(|| ExportPathError::new(selected_json, ExportPathErrorKind::NotUnicode))?;
    let base = selected.strip_suffix(".json").unwrap_or(selected);
    Ok((
        PathBuf::from(format!("{base}.json")),
        PathBuf::from(format!("{base}.txt")),
        PathBuf::from(format!("{base}.html")),
    ))
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
        let path = std::env::temp_dir().join(format!("wfdiag-export-policy-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn policy(root: &Path, temp: Option<&Path>) -> ExportPathPolicy {
        ExportPathPolicy::from_roots(
            vec![root.to_path_buf()],
            temp.map(std::path::Path::to_path_buf),
        )
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
    fn support_package_filename_matches_the_store_iso_timestamp_convention() {
        let timestamp = ExportUtcTimestamp::new(2026, 8, 31, 9, 7, 5, 42).expect("valid timestamp");
        assert_eq!(
            suggested_support_package_filename(timestamp),
            "support-package-2026-08-31T09-07-05-042Z.json"
        );
        assert!(ExportUtcTimestamp::new(2026, 8, 31, 24, 0, 0, 0).is_err());
        assert!(ExportUtcTimestamp::new(2026, 8, 31, 0, 60, 0, 0).is_err());
        assert!(ExportUtcTimestamp::new(2026, 8, 31, 0, 0, 0, 1000).is_err());
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
        let policy = policy(&root, None);
        let path = root.join("wf-diagnostics-2026-08-30.txt");
        let validated =
            validate_export_path_with_policy(&path, ReportFormat::Text, &policy).unwrap();
        assert_eq!(validated.format(), ReportFormat::Text);
        // The destination does not exist yet, so validation canonicalizes
        // the parent and rejoins the filename. On current Windows Rust that
        // canonical parent may use the equivalent `\\?\` extended prefix.
        assert_eq!(
            validated.as_path(),
            root.canonicalize()
                .unwrap()
                .join("wf-diagnostics-2026-08-30.txt")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn path_policy_rejects_escape_and_mismatched_extension() {
        let root = scratch("root");
        let outside = scratch("outside");
        let policy = policy(&root, None);
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
        let policy = policy(&root, Some(&root));
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

    #[test]
    fn any_report_rule_accepts_every_report_extension_but_nothing_else() {
        let root = scratch("any-report");
        let policy = policy(&root, None);
        for filename in ["report.txt", "report.html", "report.JSON"] {
            assert!(
                validate_report_path_with_policy(
                    &root.join(filename),
                    ExportExtensionRule::AnyReport,
                    &policy
                )
                .is_ok(),
                "{filename}"
            );
        }
        let error = validate_report_path_with_policy(
            &root.join("run.bat"),
            ExportExtensionRule::AnyReport,
            &policy,
        )
        .unwrap_err();
        assert_eq!(error.kind(), &ExportPathErrorKind::NotAReportExtension);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_parent_directory_is_reported_distinctly() {
        let root = scratch("missing-parent");
        let policy = policy(&root, None);
        let error = validate_report_path_with_policy(
            &root.join("nope").join("report.txt"),
            ExportExtensionRule::AnyReport,
            &policy,
        )
        .unwrap_err();
        assert_eq!(error.kind(), &ExportPathErrorKind::ParentMissing);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn temp_filename_rules_allow_app_reports_only() {
        for filename in [
            "wfdiag_report.txt",
            "wfdiag_report.html",
            "wf-diagnostics-2026-06-22.txt",
            "wf-diagnostics-2026-06-22.html",
            "wf-diagnostics-2026-06-22.json",
            "support-package-2026-06-22T12-00-00.json",
            "support-package-2026-06-22T12-00-00.txt",
            "support-package-2026-06-22T12-00-00.html",
        ] {
            assert!(filename_is_temp_safe(filename), "{filename}");
        }

        for filename in [
            "wfdiag_report.json",
            "support-package-2026-06-22.exe",
            "other-report.txt",
            "wf-diagnostics-2026-06-22.exe",
        ] {
            assert!(!filename_is_temp_safe(filename), "{filename}");
        }
    }

    #[test]
    fn has_report_extension_applies_outside_temp_too() {
        assert!(has_report_extension(Path::new(
            r"C:\Users\me\Documents\wf-diagnostics.txt"
        )));
        assert!(has_report_extension(Path::new(
            r"C:\Users\me\Desktop\report.html"
        )));
        assert!(has_report_extension(Path::new(
            r"C:\Users\me\Downloads\report.JSON"
        )));
        assert!(!has_report_extension(Path::new(
            r"C:\Users\me\Desktop\run.bat"
        )));
        assert!(!has_report_extension(Path::new(
            r"C:\Users\me\Documents\startup.vbs"
        )));
        assert!(!has_report_extension(Path::new(
            r"C:\Users\me\Documents\noext"
        )));
    }

    #[test]
    fn safe_report_filename_rejects_paths_and_non_report_extensions() {
        assert_eq!(
            safe_report_filename("wf-diagnostics-2026-06-22.txt").unwrap(),
            "wf-diagnostics-2026-06-22.txt"
        );
        assert!(safe_report_filename("../report.txt").is_err());
        assert!(safe_report_filename("nested/report.txt").is_err());
        assert!(safe_report_filename("report.exe").is_err());
    }

    #[test]
    fn support_package_validates_every_sibling_destination() {
        let root = scratch("support-package");
        let policy = policy(&root, Some(&root));
        let selected = validate_export_path_with_policy(
            &root.join("support-package-2026-08-31T09-07-05-042Z.json"),
            ReportFormat::Json,
            &policy,
        )
        .unwrap();

        let paths = validate_support_package_paths_with_policy(&selected, &policy).unwrap();
        assert!(
            paths
                .json
                .ends_with("support-package-2026-08-31T09-07-05-042Z.json")
        );
        assert!(
            paths
                .text
                .ends_with("support-package-2026-08-31T09-07-05-042Z.txt")
        );
        assert!(
            paths
                .html
                .ends_with("support-package-2026-08-31T09-07-05-042Z.html")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn support_package_siblings_match_the_frontend_case_sensitive_json_suffix() {
        let lowercase = PathBuf::from(r"C:\Reports\case.json");
        let (json, text, html) = shipping_support_package_sibling_paths(&lowercase).unwrap();
        assert_eq!(json, PathBuf::from(r"C:\Reports\case.json"));
        assert_eq!(text, PathBuf::from(r"C:\Reports\case.txt"));
        assert_eq!(html, PathBuf::from(r"C:\Reports\case.html"));

        let uppercase = PathBuf::from(r"C:\Reports\case.JSON");
        let (json, text, html) = shipping_support_package_sibling_paths(&uppercase).unwrap();
        assert_eq!(json, PathBuf::from(r"C:\Reports\case.JSON.json"));
        assert_eq!(text, PathBuf::from(r"C:\Reports\case.JSON.txt"));
        assert_eq!(html, PathBuf::from(r"C:\Reports\case.JSON.html"));
    }

    #[test]
    fn support_package_write_time_revalidation_rejects_a_changed_sibling() {
        let allowed = scratch("support-revalidate-allowed");
        let outside = scratch("support-revalidate-outside");
        let policy = policy(&allowed, None);
        let paths = ValidatedSupportPackagePaths {
            json: allowed.join("support-package-case.json"),
            text: outside.join("support-package-case.txt"),
            html: allowed.join("support-package-case.html"),
        };

        assert!(paths.revalidate_with(&policy).is_err());

        fs::remove_dir_all(allowed).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn user_folder_policy_adds_both_app_data_roots_and_temp() {
        let policy = ExportPathPolicy::for_user_folders(
            Some(PathBuf::from("/docs")),
            None,
            Some(PathBuf::from("/downloads")),
            Some(PathBuf::from("/roaming")),
            Some(PathBuf::from("/temp")),
        );
        assert_eq!(
            policy.allowed_roots(),
            [
                PathBuf::from("/docs"),
                PathBuf::from("/downloads"),
                PathBuf::from("/roaming/wfdiag-tauri"),
                PathBuf::from("/roaming/com.windowsforum.diagnostics"),
                PathBuf::from("/temp"),
            ]
        );
        assert_eq!(policy.temp_root(), Some(Path::new("/temp")));
    }
}
