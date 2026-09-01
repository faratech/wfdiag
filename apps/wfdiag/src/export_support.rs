//! Windows-native delivery helpers for Reactor export payloads.
//!
//! Report rendering, the closed set of external targets, and the
//! [`ExportDateStrings`] carrier all live in `wfdiag-native-export`. This
//! module owns only shell concerns that cannot live in that portable crate:
//! asking Windows for the current user's date/time presentation, putting
//! already-rendered text on the clipboard, and handing a resolved target to
//! `ShellExecuteW`.
//!
//! Clipboard calls must be made from Reactor's focused UI dispatcher. WinUI
//! has already initialized WinRT on that thread; this helper deliberately does
//! not initialize or change the caller's apartment.

use std::error::Error;
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wfdiag_native_export::{
    EmailPayload, ExportDateStrings, ExportExternalAction, render_email_compose_uri,
    resolve_export_external_url,
};
use windows::ApplicationModel::DataTransfer::{
    Clipboard, ClipboardContentOptions, DataPackage, DataPackageOperation,
};
use windows::Foundation::DateTime;
use windows::Globalization::DateTimeFormatting::{
    DateTimeFormatter, DayFormat, DayOfWeekFormat, HourFormat, MinuteFormat, MonthFormat,
    SecondFormat, YearFormat,
};
use windows::System::UserProfile::GlobalizationPreferences;
use windows::core::HSTRING;

const WINDOWS_EPOCH_OFFSET_TICKS: u128 = 116_444_736_000_000_000;
const HUNDRED_NANOSECONDS_PER_SECOND: u128 = 10_000_000;
const NANOSECONDS_PER_HUNDRED_NANOSECONDS: u128 = 100;

#[derive(Debug)]
pub enum ExportDeliveryError {
    TimeOutOfRange,
    ClipboardUnavailable,
    Windows {
        operation: &'static str,
        source: windows::core::Error,
    },
    ExternalLaunchFailed {
        code: isize,
    },
    EmailComposeLaunchFailed {
        code: isize,
    },
}

impl fmt::Display for ExportDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimeOutOfRange => {
                formatter.write_str("the export timestamp is outside the Windows DateTime range")
            }
            Self::ClipboardUnavailable => formatter.write_str(
                "Windows could not set the clipboard; keep the app focused and try again",
            ),
            Self::Windows { operation, source } => {
                write!(formatter, "{operation} failed: {source}")
            }
            Self::ExternalLaunchFailed { code } => {
                write!(
                    formatter,
                    "Windows could not open the WindowsForum new-thread page (ShellExecute code {code})"
                )
            }
            Self::EmailComposeLaunchFailed { code } => {
                write!(
                    formatter,
                    "Windows could not open a new email draft (ShellExecute code {code})"
                )
            }
        }
    }
}

impl Error for ExportDeliveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Windows { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn windows_error(
    operation: &'static str,
) -> impl FnOnce(windows::core::Error) -> ExportDeliveryError {
    move |source| ExportDeliveryError::Windows { operation, source }
}

fn winrt_datetime_at(time: SystemTime) -> Result<DateTime, ExportDeliveryError> {
    let since_unix_epoch = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ExportDeliveryError::TimeOutOfRange)?;
    winrt_datetime_from_unix_duration(since_unix_epoch)
}

fn winrt_datetime_from_unix_duration(
    since_unix_epoch: Duration,
) -> Result<DateTime, ExportDeliveryError> {
    let ticks_since_unix_epoch = u128::from(since_unix_epoch.as_secs())
        .checked_mul(HUNDRED_NANOSECONDS_PER_SECOND)
        .and_then(|ticks| {
            ticks.checked_add(
                u128::from(since_unix_epoch.subsec_nanos()) / NANOSECONDS_PER_HUNDRED_NANOSECONDS,
            )
        })
        .ok_or(ExportDeliveryError::TimeOutOfRange)?;
    let universal_time = WINDOWS_EPOCH_OFFSET_TICKS
        .checked_add(ticks_since_unix_epoch)
        .and_then(|ticks| i64::try_from(ticks).ok())
        .ok_or(ExportDeliveryError::TimeOutOfRange)?;

    Ok(DateTime {
        UniversalTime: universal_time,
    })
}

fn user_formatters() -> Result<(DateTimeFormatter, DateTimeFormatter), ExportDeliveryError> {
    let languages = GlobalizationPreferences::Languages()
        .map_err(windows_error("reading the user's language preferences"))?;
    let geographic_region = GlobalizationPreferences::HomeGeographicRegion().map_err(
        windows_error("reading the user's geographic-region preference"),
    )?;
    let calendars = GlobalizationPreferences::Calendars()
        .map_err(windows_error("reading the user's calendar preferences"))?;
    let calendar = calendars.GetAt(0).map_err(windows_error(
        "reading the user's primary calendar preference",
    ))?;
    let clocks = GlobalizationPreferences::Clocks()
        .map_err(windows_error("reading the user's clock preferences"))?;
    let clock = clocks
        .GetAt(0)
        .map_err(windows_error("reading the user's primary clock preference"))?;

    let date_time = DateTimeFormatter::CreateDateTimeFormatterDateTimeContext(
        YearFormat::Full,
        MonthFormat::Numeric,
        DayFormat::Default,
        DayOfWeekFormat::None,
        HourFormat::Default,
        MinuteFormat::Default,
        SecondFormat::Default,
        &languages,
        &geographic_region,
        &calendar,
        &clock,
    )
    .map_err(windows_error("creating the user's date-and-time formatter"))?;
    let date = DateTimeFormatter::CreateDateTimeFormatterDateTimeContext(
        YearFormat::Full,
        MonthFormat::Numeric,
        DayFormat::Default,
        DayOfWeekFormat::None,
        HourFormat::None,
        MinuteFormat::None,
        SecondFormat::None,
        &languages,
        &geographic_region,
        &calendar,
        &clock,
    )
    .map_err(windows_error("creating the user's date formatter"))?;

    Ok((date_time, date))
}

/// Format the current instant like Store 2.5.8's parameterless JavaScript
/// `toLocaleString()` and `toLocaleDateString()` calls.
///
/// Windows supplies component ordering, separators, numeral system, calendar,
/// clock, user language preferences, and the local time-zone conversion.
pub fn current_export_date_strings() -> Result<ExportDateStrings, ExportDeliveryError> {
    export_date_strings_at(SystemTime::now())
}

/// Format a caller-supplied instant using the same policy as
/// [`current_export_date_strings`]. Supplying the clock makes integration
/// tests and request construction deterministic without weakening locale
/// correctness in production.
pub fn export_date_strings_at(time: SystemTime) -> Result<ExportDateStrings, ExportDeliveryError> {
    let value = winrt_datetime_at(time)?;
    let (date_time_formatter, date_formatter) = user_formatters()?;
    let generated = date_time_formatter
        .Format(value)
        .map_err(windows_error("formatting the local date and time"))?
        .to_string();
    let local_date = date_formatter
        .Format(value)
        .map_err(windows_error("formatting the local date"))?
        .to_string();

    Ok(ExportDateStrings {
        generated,
        local_date,
    })
}

/// Put one already-rendered plain-text report on the Windows clipboard.
///
/// The payload is length-delimited `HSTRING` text, so embedded NULs or Unicode
/// cannot become a raw clipboard buffer bug. Reports are deliberately excluded
/// from clipboard history and cross-device roaming because diagnostic output
/// can contain machine-specific data. `Flush` keeps the current item available
/// if WFDiag closes before the user pastes it.
pub fn write_text_to_clipboard(text: &str) -> Result<(), ExportDeliveryError> {
    let package = DataPackage::new().map_err(windows_error("creating the clipboard package"))?;
    package
        .SetRequestedOperation(DataPackageOperation::Copy)
        .map_err(windows_error("marking the clipboard package as a copy"))?;
    package
        .SetText(&HSTRING::from(text))
        .map_err(windows_error("putting text in the clipboard package"))?;

    let options = ClipboardContentOptions::new()
        .map_err(windows_error("creating clipboard privacy options"))?;
    options
        .SetIsAllowedInHistory(false)
        .map_err(windows_error("excluding the report from clipboard history"))?;
    options
        .SetIsRoamable(false)
        .map_err(windows_error("excluding the report from clipboard roaming"))?;

    let accepted = Clipboard::SetContentWithOptions(&package, &options)
        .map_err(windows_error("setting the Windows clipboard"))?;
    if !accepted {
        return Err(ExportDeliveryError::ClipboardUnavailable);
    }
    Clipboard::Flush().map_err(windows_error("flushing the Windows clipboard"))
}

/// Open one typed export action through the Windows shell.
///
/// No caller-controlled URL reaches `ShellExecuteW`; the action must already
/// have crossed Reactor's explicit user-activation boundary.
pub fn launch_export_external_action(
    action: ExportExternalAction,
) -> Result<(), ExportDeliveryError> {
    let target = resolve_export_external_url(action);
    let code = shell_execute_open(target);
    if code <= 32 {
        Err(ExportDeliveryError::ExternalLaunchFailed { code })
    } else {
        Ok(())
    }
}

fn shell_execute_open(target: &str) -> isize {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    let verb: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    let target: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    result.0 as isize
}

/// Open the user's registered mail app with a new, unsent email draft.
///
/// The target is constructed from the shared [`EmailPayload`] by
/// [`render_email_compose_uri`]: it has no recipient, carries only the short
/// paste instruction, and percent-encodes every query component. The full
/// report is never placed on the command line or in the URI. This function
/// does not write the clipboard and cannot send mail; the caller must invoke
/// it only after an explicit user action and after successfully copying
/// [`EmailPayload::clipboard_body`] with [`write_text_to_clipboard`].
pub fn launch_email_compose_draft(payload: &EmailPayload) -> Result<(), ExportDeliveryError> {
    let target = render_email_compose_uri(payload);
    let code = shell_execute_open(&target);
    if code <= 32 {
        Err(ExportDeliveryError::EmailComposeLaunchFailed { code })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_converts_to_the_documented_windows_epoch_offset() {
        assert_eq!(
            winrt_datetime_from_unix_duration(Duration::ZERO)
                .unwrap()
                .UniversalTime,
            116_444_736_000_000_000
        );
    }

    #[test]
    fn conversion_keeps_whole_seconds_and_truncates_sub_tick_precision() {
        let value = winrt_datetime_from_unix_duration(Duration::new(1, 999)).unwrap();
        assert_eq!(value.UniversalTime, 116_444_736_010_000_009);
    }

    #[test]
    fn conversion_rejects_values_outside_winrt_datetime() {
        let too_large = Duration::from_secs((i64::MAX as u64 / 10_000_000) + 1);
        assert!(matches!(
            winrt_datetime_from_unix_duration(too_large),
            Err(ExportDeliveryError::TimeOutOfRange)
        ));
    }

    #[test]
    fn email_compose_failure_never_echoes_payload_text() {
        let error = ExportDeliveryError::EmailComposeLaunchFailed { code: 31 }.to_string();
        assert_eq!(
            error,
            "Windows could not open a new email draft (ShellExecute code 31)"
        );
        assert!(!error.contains("subject"));
        assert!(!error.contains("diagnostic"));
    }

    #[test]
    fn delivery_errors_never_echo_clipboard_text() {
        let error = ExportDeliveryError::ClipboardUnavailable.to_string();
        assert!(!error.contains("diagnostic payload"));
        assert!(error.contains("clipboard"));
    }
}
