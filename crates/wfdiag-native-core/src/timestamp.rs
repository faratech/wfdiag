use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::DiagError;

/// A simple UTC timestamp that serializes to ISO 8601 format for compatibility with chrono
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    /// Seconds since Unix epoch
    pub secs: i64,
}

/// Parse a CIM/WMI datetime — `"20240612153045.500000-300"` — into a
/// Timestamp. The trailing signed value is the LOCAL-to-UTC offset in
/// MINUTES; `+***` (offset unspecified) is treated as UTC. Returns None on
/// anything malformed.
pub fn parse_wmi_datetime(s: &str) -> Option<Timestamp> {
    let s = s.trim();
    if s.len() < 14 || !s.as_bytes()[..14].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let (digits, rest) = s.split_at(14);
    let iso = format!(
        "{}-{}-{}T{}:{}:{}Z",
        &digits[0..4],
        &digits[4..6],
        &digits[6..8],
        &digits[8..10],
        &digits[10..12],
        &digits[12..14]
    );
    let mut ts = Timestamp::from_iso_string(&iso).ok()?;
    // rest = ".ffffff-300" | ".ffffff+***" | "" — the offset sign is the LAST
    // sign per the CIM grammar (the fractional part has none), so rfind avoids
    // mistaking a stray sign in the fraction for the offset delimiter.
    if let Some(sign_pos) = rest.rfind(['+', '-'])
        && let Ok(offset_minutes) = rest[sign_pos..].parse::<i64>()
    {
        // CIM datetime is local time; UTC = local - offset
        ts.secs -= offset_minutes * 60;
    }
    Some(ts)
}

impl Timestamp {
    /// Create a timestamp for the current time
    #[must_use]
    pub fn now() -> Self {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            secs: duration.as_secs().cast_signed(),
        }
    }

    /// Create a timestamp from Unix epoch seconds
    #[must_use]
    pub fn from_secs(secs: i64) -> Self {
        Self { secs }
    }

    /// Check if this timestamp is before another by a given duration
    #[must_use]
    pub fn is_before(&self, other: &Timestamp, duration: Duration) -> bool {
        self.secs + duration.as_secs().cast_signed() <= other.secs
    }

    /// Format timestamp with a format string (simplified implementation)
    /// Supported: %Y (year), %m (month), %d (day), %H (hour), %M (minute), %S (second)
    #[must_use]
    pub fn format(&self, fmt: &str) -> String {
        let secs = self.secs;
        let days = secs / 86_400;
        let remaining = secs % 86_400;

        let hours = (remaining / 3600) % 24;
        let minutes = (remaining % 3600) / 60;
        let seconds = remaining % 60;

        let (year, month, day) = days_to_ymd(days);

        fmt.replace("%Y", &format!("{year:04}"))
            .replace("%m", &format!("{month:02}"))
            .replace("%d", &format!("{day:02}"))
            .replace("%H", &format!("{hours:02}"))
            .replace("%M", &format!("{minutes:02}"))
            .replace("%S", &format!("{seconds:02}"))
    }

    /// Format as ISO 8601 string (YYYY-MM-DDTHH:MM:SSZ)
    #[allow(clippy::wrong_self_convention)]
    #[must_use]
    pub fn to_iso_string(&self) -> String {
        // Convert to components
        let secs = self.secs;

        // Calculate date/time components from Unix timestamp
        // Days since Unix epoch
        let days = secs / 86_400;
        let remaining = secs % 86_400;

        let hours = (remaining / 3600) % 24;
        let minutes = (remaining % 3600) / 60;
        let seconds = remaining % 60;

        // Calculate year, month, day from days since epoch (1970-01-01)
        let (year, month, day) = days_to_ymd(days);

        format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
    }

    /// Parse from ISO 8601 string
    ///
    /// # Errors
    /// Returns a JSON-serialized [`DiagError::Internal`] when the string is
    /// shorter than `YYYY-MM-DDTHH:MM:SS` or any field fails to parse.
    pub fn from_iso_string(s: &str) -> Result<Self, String> {
        // Parse format: YYYY-MM-DDTHH:MM:SS or YYYY-MM-DDTHH:MM:SSZ or with timezone
        let s = s.trim();

        // Remove trailing Z or timezone
        let s = s.trim_end_matches('Z');
        let s = if let Some(idx) = s.rfind('+') {
            &s[..idx]
        } else if let Some(idx) = s.rfind('-') {
            // Check if it's the date separator or timezone
            if idx > 10 { &s[..idx] } else { s }
        } else {
            s
        };

        // Parse components
        if s.len() < 19 {
            return Err(DiagError::internal(format!("Invalid ISO 8601 format: {s}")).into());
        }

        let year: i32 = s[0..4]
            .parse()
            .map_err(|_| DiagError::internal("Invalid year"))?;
        let month: u32 = s[5..7]
            .parse()
            .map_err(|_| DiagError::internal("Invalid month"))?;
        let day: u32 = s[8..10]
            .parse()
            .map_err(|_| DiagError::internal("Invalid day"))?;
        let hour: u32 = s[11..13]
            .parse()
            .map_err(|_| DiagError::internal("Invalid hour"))?;
        let minute: u32 = s[14..16]
            .parse()
            .map_err(|_| DiagError::internal("Invalid minute"))?;
        let second: u32 = s[17..19]
            .parse()
            .map_err(|_| DiagError::internal("Invalid second"))?;

        // Convert to Unix timestamp
        let days = ymd_to_days(year, month, day);
        let secs =
            days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);

        Ok(Self { secs })
    }
}

/// Convert year/month/day to days since Unix epoch
// The algorithm's own invariants bound the narrowing casts: `y - era * 400`
// ("year of era") is 0..=399 by construction, so it never loses a sign or
// truncates.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn ymd_to_days(year: i32, month: u32, day: u32) -> i64 {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let y = i64::from(if month <= 2 { year - 1 } else { year });
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + i64::from(doe) - 719_468
}

/// Convert days since Unix epoch to year/month/day
// As above: "day of era" is 0..=146_096, and the reconstructed year fits i32
// for every timestamp an i64 second count can represent.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

impl std::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_iso_string())
    }
}

impl Serialize for Timestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_iso_string())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_iso_string(&s).map_err(serde::de::Error::custom)
    }
}

/// Format a `SystemTime` as a local date-time string (for display)
#[must_use]
pub fn format_local_datetime(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs().cast_signed();

    // Note: This is UTC, not local time. For proper local time we'd need
    // platform-specific code, but for log timestamps UTC is fine.
    let days = secs / 86_400;
    let remaining = secs % 86_400;

    let hours = (remaining / 3600) % 24;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}")
}

/// Format just the time portion (HH:MM:SS)
#[must_use]
pub fn format_time(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();

    let hours = (secs / 3600) % 24;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;

    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timestamp_roundtrip() {
        let ts = Timestamp::now();
        let iso = ts.to_iso_string();
        let parsed = Timestamp::from_iso_string(&iso).unwrap();
        assert_eq!(ts.secs, parsed.secs);
    }

    #[test]
    fn test_known_date() {
        // 2024-01-15T12:30:45Z
        let ts = Timestamp::from_iso_string("2024-01-15T12:30:45Z").unwrap();
        assert_eq!(ts.to_iso_string(), "2024-01-15T12:30:45Z");
    }

    #[test]
    fn test_epoch() {
        let ts = Timestamp::from_secs(0);
        assert_eq!(ts.to_iso_string(), "1970-01-01T00:00:00Z");
    }
}

#[cfg(test)]
mod wmi_datetime_tests {
    use super::*;

    #[test]
    fn parses_cim_datetime_with_minute_offset() {
        // 15:30:45 local at UTC-300min (-5h) => 20:30:45 UTC
        let ts = parse_wmi_datetime("20240612153045.500000-300").unwrap();
        assert_eq!(ts.to_iso_string(), "2024-06-12T20:30:45Z");
        // Positive offset subtracts
        let ts = parse_wmi_datetime("20240612153045.000000+060").unwrap();
        assert_eq!(ts.to_iso_string(), "2024-06-12T14:30:45Z");
    }

    #[test]
    fn unspecified_offset_is_treated_as_utc() {
        // DriverDate commonly arrives as date-only with +***
        let ts = parse_wmi_datetime("20190312000000.000000+***").unwrap();
        assert_eq!(ts.to_iso_string(), "2019-03-12T00:00:00Z");
    }

    #[test]
    fn offset_uses_last_sign_not_first() {
        // The offset delimiter is the LAST sign; a stray sign earlier in the
        // string (e.g. a malformed fraction) must not steal the offset parse.
        let ts = parse_wmi_datetime("20240612153045.5-0-300").unwrap();
        assert_eq!(ts.to_iso_string(), "2024-06-12T20:30:45Z");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_wmi_datetime("").is_none());
        assert!(parse_wmi_datetime("2024-06-12T15:30:45Z").is_none());
        assert!(parse_wmi_datetime("notadate").is_none());
        assert!(parse_wmi_datetime("2024061").is_none());
    }
}
