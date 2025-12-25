use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A simple UTC timestamp that serializes to ISO 8601 format for compatibility with chrono
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    /// Seconds since Unix epoch
    pub secs: i64,
}

impl Timestamp {
    /// Create a timestamp for the current time
    pub fn now() -> Self {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            secs: duration.as_secs() as i64,
        }
    }

    /// Create a timestamp from Unix epoch seconds
    #[allow(dead_code)]
    pub fn from_secs(secs: i64) -> Self {
        Self { secs }
    }

    /// Get the Unix timestamp in seconds
    #[allow(dead_code)]
    pub fn timestamp(&self) -> i64 {
        self.secs
    }

    /// Check if this timestamp is before another by a given duration
    pub fn is_before(&self, other: &Timestamp, duration: Duration) -> bool {
        self.secs + duration.as_secs() as i64 <= other.secs
    }

    /// Format as ISO 8601 string (YYYY-MM-DDTHH:MM:SSZ)
    #[allow(clippy::wrong_self_convention)]
    pub fn to_iso_string(&self) -> String {
        // Convert to components
        let secs = self.secs;

        // Calculate date/time components from Unix timestamp
        // Days since Unix epoch
        let days = secs / 86400;
        let remaining = secs % 86400;

        let hours = (remaining / 3600) % 24;
        let minutes = (remaining % 3600) / 60;
        let seconds = remaining % 60;

        // Calculate year, month, day from days since epoch (1970-01-01)
        let (year, month, day) = days_to_ymd(days);

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            year, month, day, hours, minutes, seconds
        )
    }

    /// Parse from ISO 8601 string
    pub fn from_iso_string(s: &str) -> Result<Self, String> {
        // Parse format: YYYY-MM-DDTHH:MM:SS or YYYY-MM-DDTHH:MM:SSZ or with timezone
        let s = s.trim();

        // Remove trailing Z or timezone
        let s = s.trim_end_matches('Z');
        let s = if let Some(idx) = s.rfind('+') {
            &s[..idx]
        } else if let Some(idx) = s.rfind('-') {
            // Check if it's the date separator or timezone
            if idx > 10 {
                &s[..idx]
            } else {
                s
            }
        } else {
            s
        };

        // Parse components
        if s.len() < 19 {
            return Err(format!("Invalid ISO 8601 format: {}", s));
        }

        let year: i32 = s[0..4].parse().map_err(|_| "Invalid year")?;
        let month: u32 = s[5..7].parse().map_err(|_| "Invalid month")?;
        let day: u32 = s[8..10].parse().map_err(|_| "Invalid day")?;
        let hour: u32 = s[11..13].parse().map_err(|_| "Invalid hour")?;
        let minute: u32 = s[14..16].parse().map_err(|_| "Invalid minute")?;
        let second: u32 = s[17..19].parse().map_err(|_| "Invalid second")?;

        // Convert to Unix timestamp
        let days = ymd_to_days(year, month, day);
        let secs = days * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64;

        Ok(Self { secs })
    }
}

/// Convert year/month/day to days since Unix epoch
fn ymd_to_days(year: i32, month: u32, day: u32) -> i64 {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// Convert days since Unix epoch to year/month/day
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
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

/// Format a SystemTime as a local date-time string (for display)
pub fn format_local_datetime(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs() as i64;

    // Note: This is UTC, not local time. For proper local time we'd need
    // platform-specific code, but for log timestamps UTC is fine.
    let days = secs / 86400;
    let remaining = secs % 86400;

    let hours = (remaining / 3600) % 24;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    let (year, month, day) = days_to_ymd(days);

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    )
}

/// Format just the time portion (HH:MM:SS)
pub fn format_time(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();

    let hours = (secs / 3600) % 24;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
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
