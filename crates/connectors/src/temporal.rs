//! Calendar conversions for imported date and timestamp text.
//!
//! Databases and spreadsheets hand dates over as text in a small number of ISO forms. These are
//! the conversions that turn them into the engine's day and microsecond counts; the scene's
//! `civil_from_days` is the inverse used when drawing an axis.

/// Days since 1970-01-01 for a civil date in the proleptic Gregorian calendar.
///
/// Howard Hinnant's `days_from_civil`. Kept here rather than pulling in a date library for two
/// formats and one arithmetic operation.
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The civil date for days since 1970-01-01 (Howard Hinnant's `civil_from_days`).
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month + 2) / 5 + 1) as u32;
    let month = if month < 10 { month + 3 } else { month - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Formats days since the epoch as `YYYY-MM-DD`.
pub(crate) fn format_date(days: i32) -> String {
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Formats microseconds since the epoch as `YYYY-MM-DD HH:MM:SS`.
pub(crate) fn format_timestamp(micros: i64) -> String {
    let days = micros.div_euclid(86_400_000_000);
    let rest = micros.rem_euclid(86_400_000_000);
    let (year, month, day) = civil_from_days(days);
    let hour = rest / 3_600_000_000;
    let minute = (rest % 3_600_000_000) / 60_000_000;
    let second = (rest % 60_000_000) / 1_000_000;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// Parses `YYYY-MM-DD` into days since 1970-01-01.
///
/// A trailing time part is ignored, so this also accepts a full timestamp's date portion.
pub(crate) fn parse_date(text: &str) -> Option<i32> {
    let text = text.trim();
    let date = text.split_once(' ').map_or(text, |(date, _)| date);
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day) as i32)
}

/// Parses a date in the forms a spreadsheet cell holds: `YYYY-MM-DD`, `M/D/YY` or `M/D/YYYY`.
///
/// For a slash date, a first component above 12 is read as the day (day-first), otherwise it is
/// read as the month (month-first, the common form in the exports this reads).
pub(crate) fn parse_display_date(text: &str) -> Option<i32> {
    if let Some(days) = parse_date(text) {
        return Some(days);
    }
    let date = text.split_whitespace().next()?;
    let mut parts = date.split('/');
    let first: u32 = parts.next()?.trim().parse().ok()?;
    let second: u32 = parts.next()?.trim().parse().ok()?;
    let third: i64 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let year = if (0..=99).contains(&third) {
        if third >= 70 {
            1900 + third
        } else {
            2000 + third
        }
    } else {
        third
    };
    let (month, day) = if first > 12 && second <= 12 {
        (second, first)
    } else {
        (first, second)
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day) as i32)
}

/// Parses `YYYY-MM-DD[ HH:MM[:SS[.ffffff]]]` into microseconds since 1970-01-01 00:00:00.
pub(crate) fn parse_timestamp(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let normalized;
    let trimmed = if trimmed.contains('T') {
        normalized = trimmed.replace('T', " ");
        normalized.as_str()
    } else {
        trimmed
    };
    let (date, time) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
    let days = parse_date(date)? as i64;
    if time.is_empty() {
        return Some(days * 86_400_000_000);
    }
    // Drop a timezone suffix (`Z`, `+00:00`, `-05:00`) from the time part.
    let time = time.split(['+', 'Z']).next().unwrap_or("");
    let time = time.split('-').next().unwrap_or("");
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut parts = clock.split(':');
    let hour: i64 = parts.next()?.parse().ok()?;
    let minute: i64 = parts.next().unwrap_or("0").parse().ok()?;
    let second: i64 = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let fraction: String = fraction.chars().take(6).collect();
    let micros: i64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<6}").parse().ok()?
    };
    Some(days * 86_400_000_000 + (hour * 3_600 + minute * 60 + second) * 1_000_000 + micros)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip_through_days() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("1970-01-02"), Some(1));
        assert_eq!(parse_date("2025-01-01"), Some(20_089));
        assert_eq!(parse_date("2025-01-01 13:45:00"), Some(20_089));
        assert_eq!(parse_date("not a date"), None);
        assert_eq!(parse_date("2025-13-01"), None);
    }

    #[test]
    fn display_dates_accept_slash_forms() {
        assert_eq!(parse_display_date("2025-01-01"), Some(20_089));
        assert_eq!(parse_display_date("11/23/26"), Some(20_780));
        assert_eq!(parse_display_date("11/23/2026"), Some(20_780));
        assert_eq!(parse_display_date("23/11/26"), Some(20_780));
        assert_eq!(parse_display_date("n/a"), None);
    }

    #[test]
    fn timestamps_keep_their_time_and_fraction() {
        assert_eq!(parse_timestamp("1970-01-01 00:00:00"), Some(0));
        assert_eq!(parse_timestamp("1970-01-02"), Some(86_400_000_000));
        assert_eq!(
            parse_timestamp("2025-01-01 03:04:05"),
            Some(20_089 * 86_400_000_000 + 11_045_000_000)
        );
        assert_eq!(parse_timestamp("1970-01-01 00:00:01.5"), Some(1_500_000));
    }
}
