//! RFC 3339 / ISO 8601 UTC timestamp formatting and parsing for `SystemTime`.
//!
//! Provides deterministic, zero-dependency serialization and deserialization of `SystemTime`
//! for SQLite persistence.

use crate::error::StorageError;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Formats a `SystemTime` as an RFC 3339 UTC string with millisecond precision:
/// `YYYY-MM-DDTHH:MM:SS.sssZ`.
pub fn format_rfc3339(st: SystemTime) -> String {
    let dur = match st.duration_since(UNIX_EPOCH) {
        Ok(d) => d,
        Err(_) => return "1970-01-01T00:00:00.000Z".to_string(),
    };

    let total_secs = dur.as_secs();
    let millis = dur.subsec_millis();

    let secs_of_day = total_secs % 86400;
    let hours = secs_of_day / 3600;
    let minutes = (secs_of_day % 3600) / 60;
    let seconds = secs_of_day % 60;

    let days = (total_secs / 86400) as i64;

    // Civil date from days since 1970-01-01 (Howard Hinnant algorithm)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

/// Parses an RFC 3339 / ISO 8601 UTC timestamp string into a `SystemTime`.
///
/// Supports timestamps formatted as:
/// - `YYYY-MM-DDTHH:MM:SSZ`
/// - `YYYY-MM-DDTHH:MM:SS.sssZ`
pub fn parse_rfc3339(s: &str) -> Result<SystemTime, StorageError> {
    let s = s.trim();
    if s.len() < 20 {
        return Err(StorageError::CorruptRecord(format!(
            "Timestamp string '{s}' too short for RFC 3339"
        )));
    }

    let (date_part, time_part) = s
        .split_once('T')
        .ok_or_else(|| StorageError::CorruptRecord(format!("Missing 'T' in timestamp '{s}'")))?;

    // Parse date: YYYY-MM-DD
    let mut date_fields = date_part.split('-');
    let y_str = date_fields
        .next()
        .ok_or_else(|| StorageError::CorruptRecord(format!("Invalid date format in '{s}'")))?;
    let m_str = date_fields
        .next()
        .ok_or_else(|| StorageError::CorruptRecord(format!("Invalid date format in '{s}'")))?;
    let d_str = date_fields
        .next()
        .ok_or_else(|| StorageError::CorruptRecord(format!("Invalid date format in '{s}'")))?;

    let y: i64 = y_str
        .parse()
        .map_err(|_| StorageError::CorruptRecord(format!("Invalid year in '{s}'")))?;
    let m: u64 = m_str
        .parse()
        .map_err(|_| StorageError::CorruptRecord(format!("Invalid month in '{s}'")))?;
    let d: u64 = d_str
        .parse()
        .map_err(|_| StorageError::CorruptRecord(format!("Invalid day in '{s}'")))?;

    // Strip trailing 'Z' if present
    let time_clean = time_part.strip_suffix('Z').unwrap_or(time_part);

    // Split time into HH:MM:SS and optional fractional seconds
    let (hms_part, frac_part) = match time_clean.split_once('.') {
        Some((hms, frac)) => (hms, Some(frac)),
        None => (time_clean, None),
    };

    let mut time_fields = hms_part.split(':');
    let h_str = time_fields
        .next()
        .ok_or_else(|| StorageError::CorruptRecord(format!("Invalid time format in '{s}'")))?;
    let min_str = time_fields
        .next()
        .ok_or_else(|| StorageError::CorruptRecord(format!("Invalid time format in '{s}'")))?;
    let sec_str = time_fields
        .next()
        .ok_or_else(|| StorageError::CorruptRecord(format!("Invalid time format in '{s}'")))?;

    let h: u64 = h_str
        .parse()
        .map_err(|_| StorageError::CorruptRecord(format!("Invalid hours in '{s}'")))?;
    let min: u64 = min_str
        .parse()
        .map_err(|_| StorageError::CorruptRecord(format!("Invalid minutes in '{s}'")))?;
    let sec: u64 = sec_str
        .parse()
        .map_err(|_| StorageError::CorruptRecord(format!("Invalid seconds in '{s}'")))?;

    let millis: u64 = match frac_part {
        Some(f) => {
            let padded = format!("{:<03}", f);
            padded[..3].parse().unwrap_or(0)
        }
        None => 0,
    };

    // Days since 1970-01-01 (inverse Howard Hinnant algorithm)
    let y_calc = if m <= 2 { y - 1 } else { y };
    let era = if y_calc >= 0 { y_calc } else { y_calc - 399 } / 400;
    let yoe = (y_calc - era * 400) as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + (doe as i64) - 719468;

    if days < 0 {
        return Err(StorageError::CorruptRecord(format!(
            "Timestamps prior to Unix epoch (1970) not supported: '{s}'"
        )));
    }

    let total_secs = (days as u64) * 86400 + h * 3600 + min * 60 + sec;
    Ok(UNIX_EPOCH + Duration::from_secs(total_secs) + Duration::from_millis(millis))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_and_parse_roundtrip() {
        let now = SystemTime::now();
        let formatted = format_rfc3339(now);
        assert!(formatted.ends_with('Z'));
        assert!(formatted.contains('T'));

        let parsed = parse_rfc3339(&formatted).expect("parsing failed");
        let diff = if parsed > now {
            parsed.duration_since(now).unwrap()
        } else {
            now.duration_since(parsed).unwrap()
        };
        // Accurate within 1 millisecond
        assert!(diff.as_millis() <= 1);
    }

    #[test]
    fn test_unix_epoch() {
        let epoch = UNIX_EPOCH;
        let formatted = format_rfc3339(epoch);
        assert_eq!(formatted, "1970-01-01T00:00:00.000Z");

        let parsed = parse_rfc3339(&formatted).expect("parsing epoch failed");
        assert_eq!(parsed, epoch);
    }

    #[test]
    fn test_known_timestamp() {
        // 2026-11-02T12:00:00.500Z
        let parsed = parse_rfc3339("2026-11-02T12:00:00.500Z").expect("parse known failed");
        let formatted = format_rfc3339(parsed);
        assert_eq!(formatted, "2026-11-02T12:00:00.500Z");
    }
}
