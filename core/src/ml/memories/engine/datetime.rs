//! The ISO-8601 / EXIF date-time parser of Ente's `ente-exif` crate
//! (`DateTime::parse` and `unix_micros`), reduced to what the memories
//! need: reading the capture wall clock of a file
//! (`pubMagicMetadata.dateTime`).
//!
//! Based on Ente (ente.com).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DateTime {
    pub(super) year: u16,
    pub(super) month: u8,
    pub(super) day: u8,
    pub(super) hour: u8,
    pub(super) minute: u8,
    pub(super) second: u8,
    pub(super) nanosecond: u32,
    pub(super) offset_minutes: Option<i16>,
}

impl DateTime {
    /// `YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM|±HHMM]` (also `:` date
    /// separators and a space before the time, like EXIF).
    pub(super) fn parse(value: &str) -> Option<Self> {
        parse(value.trim())
    }

    /// µs since the epoch; `None` without an offset or when invalid.
    pub(super) fn unix_micros(self) -> Option<i64> {
        if !self.valid() {
            return None;
        }
        let offset = i64::from(self.offset_minutes?);
        let year = i64::from(self.year) - 1;
        let before_year = 365 * year + year / 4 - year / 100 + year / 400;
        let before_month: i64 = (1..self.month)
            .map(|m| i64::from(days_in_month(self.year, m)))
            .sum();
        let days = before_year + before_month + i64::from(self.day) - 1 - 719_162;
        let seconds = days * 86_400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
            - offset * 60;
        Some(seconds * 1_000_000 + i64::from(self.nanosecond / 1000))
    }

    fn valid(self) -> bool {
        (1..=9999).contains(&self.year)
            && self.day != 0
            && self.day <= days_in_month(self.year, self.month)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
            && self.nanosecond < 1_000_000_000
            && self
                .offset_minutes
                .is_none_or(|v| (-1439..=1439).contains(&v))
    }
}

fn parse(value: &str) -> Option<DateTime> {
    let b = value.as_bytes();
    if b.len() < 19
        || !matches!(b[4], b':' | b'-')
        || b[7] != b[4]
        || !matches!(b[10], b' ' | b'T' | b't')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let mut date = DateTime {
        year: number(&b[..4])? as u16,
        month: number(&b[5..7])? as u8,
        day: number(&b[8..10])? as u8,
        hour: number(&b[11..13])? as u8,
        minute: number(&b[14..16])? as u8,
        second: number(&b[17..19])? as u8,
        nanosecond: 0,
        offset_minutes: None,
    };
    let mut suffix = value.get(19..)?;
    if suffix.starts_with(['.', ':']) {
        let digits = &suffix[1..];
        let end = digits.bytes().take_while(u8::is_ascii_digit).count();
        date.nanosecond = fraction(&digits[..end])?;
        suffix = &digits[end..];
    }
    if !suffix.is_empty() {
        date.offset_minutes = Some(utc_offset(suffix)?);
    }
    date.valid().then_some(date)
}

fn number(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0, |n, b| {
        b.is_ascii_digit().then(|| n * 10 + u32::from(b - b'0'))
    })
}

fn fraction(value: &str) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = value.len().min(9);
    Some(number(&value.as_bytes()[..digits])? * 10u32.pow(9 - digits as u32))
}

fn utc_offset(value: &str) -> Option<i16> {
    if matches!(value, "Z" | "z") {
        return Some(0);
    }
    let b = value.as_bytes();
    let minutes = match b {
        [b'+' | b'-', _, _, b':', _, _] => number(&b[4..])?,
        [b'+' | b'-', _, _, _, _] => number(&b[3..])?,
        _ => return None,
    };
    let hours = number(&b[1..3])?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some((hours * 60 + minutes) as i16 * if b[0] == b'-' { -1 } else { 1 })
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 => {
            if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::DateTime;

    #[test]
    fn parses_wall_clocks_and_offsets() {
        let utc = |s: &str| DateTime::parse(s).and_then(DateTime::unix_micros);
        assert_eq!(utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(utc("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(utc("1970:01:01 00:00:01.5Z"), Some(1_500_000));
        assert_eq!(utc("2024-03-11T08:30:00.000"), None);
        assert!(DateTime::parse("2024-03-11T08:30:00.000").is_some());
        assert!(DateTime::parse("not a date").is_none());
        assert!(utc("2023-02-29T00:00:00Z").is_none());
    }
}
