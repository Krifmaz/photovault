//! Time helpers for Ente's microsecond timestamps.
//!
//! [`crate::abi::FileInfo::creation_time`] and [`crate::abi::HookInput::now`]
//! are microseconds since the Unix epoch. These helpers turn them into UTC
//! calendar dates without pulling a date library into the Wasm module.

/// Microseconds in one day.
pub const MICROS_PER_DAY: i64 = 86_400 * 1_000_000;

/// A UTC calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    /// Year, e.g. 2024.
    pub year: i32,
    /// Month, 1-12.
    pub month: u32,
    /// Day of the month, 1-31.
    pub day: u32,
}

/// The UTC date of a microsecond timestamp.
///
/// ```
/// use photovault_plugin_pdk::time::{date_from_micros, Date};
/// assert_eq!(date_from_micros(0), Date { year: 1970, month: 1, day: 1 });
/// assert_eq!(
///     date_from_micros(1_709_210_096_000_000),
///     Date { year: 2024, month: 2, day: 29 }
/// );
/// ```
pub fn date_from_micros(micros: i64) -> Date {
    // Howard Hinnant's days-to-civil algorithm.
    let days = micros.div_euclid(MICROS_PER_DAY);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    Date { year, month, day }
}

/// Whole days between two microsecond timestamps (`later - earlier`, rounded
/// down; negative if `later` is before `earlier`).
pub fn days_between(earlier: i64, later: i64) -> i64 {
    (later - earlier).div_euclid(MICROS_PER_DAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(
            date_from_micros(-1),
            Date {
                year: 1969,
                month: 12,
                day: 31
            }
        );
        // 2000-03-01T00:00:00Z
        assert_eq!(
            date_from_micros(951_868_800_000_000),
            Date {
                year: 2000,
                month: 3,
                day: 1
            }
        );
        // 2023-12-31T23:59:59Z
        assert_eq!(
            date_from_micros(1_704_067_199_000_000),
            Date {
                year: 2023,
                month: 12,
                day: 31
            }
        );
        assert_eq!(days_between(0, MICROS_PER_DAY * 3 - 1), 2);
        assert_eq!(days_between(MICROS_PER_DAY, 0), -1);
    }
}
