//! Immich date semantics, turned into UTC microseconds for Ente.
//!
//! Immich keeps three capture times per asset:
//!
//! - `exifInfo.dateTimeOriginal`: the capture **instant** (UTC). Immich
//!   converts the EXIF time to an instant using the file's offset; a time
//!   without an offset is taken in the Immich server's zone (normally UTC).
//! - `fileCreatedAt`: also an instant; equal to `dateTimeOriginal` when the
//!   file has EXIF, otherwise the file's creation time.
//! - `localDateTime`: the **wall-clock** time where the photo was taken,
//!   written with a `Z` suffix although it is not UTC. It must never be used
//!   as an instant directly; with `exifInfo.timeZone` it can be turned back
//!   into one.
//!
//! Ente stores `creationTime` as microseconds since the epoch (UTC), so the
//! importer prefers the instants and only falls back to `localDateTime`.

use chrono::{DateTime, FixedOffset, Local, NaiveDateTime, TimeZone, Utc};

use super::model::Asset;

/// How to read a capture time that has no time zone in Immich.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NaiveTimes {
    /// Trust Immich's instant (the server interpreted the naive EXIF time in
    /// its own zone, normally UTC). The default.
    #[default]
    AsImmich,
    /// Interpret the wall-clock time in this machine's local zone, the way
    /// the Ente apps read an EXIF time without an offset. Only applied to
    /// images whose `exifInfo.timeZone` is empty.
    Local,
}

/// Parse an ISO 8601 / RFC 3339 timestamp to microseconds since the epoch.
///
/// Accepts `Z` or numeric offsets (`+02:00`, `+0200`, `+02`) and a space
/// instead of `T`. A timestamp without any offset is read as UTC.
pub fn parse_instant_us(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_micros());
    }
    let t = s.replacen(' ', "T", 1);
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f%:z", "%Y-%m-%dT%H:%M:%S%.f%z"] {
        if let Ok(dt) = DateTime::parse_from_str(&t, fmt) {
            return Some(dt.timestamp_micros());
        }
    }
    // Short offset such as `+02` (Postgres style).
    if let Some((head, off)) = split_short_offset(&t)
        && let Ok(dt) =
            DateTime::parse_from_str(&format!("{head}{off}:00"), "%Y-%m-%dT%H:%M:%S%.f%:z")
    {
        return Some(dt.timestamp_micros());
    }
    parse_wall_clock(&t).map(|n| n.and_utc().timestamp_micros())
}

/// Split `...+HH` / `...-HH` at the end of a timestamp.
fn split_short_offset(s: &str) -> Option<(&str, &str)> {
    let n = s.len();
    if n < 4 || !s.is_char_boundary(n - 3) {
        return None;
    }
    let (head, off) = s.split_at(n - 3);
    let b = off.as_bytes();
    let sign_ok = b[0] == b'+' || b[0] == b'-';
    (sign_ok && b[1].is_ascii_digit() && b[2].is_ascii_digit() && head.contains('T'))
        .then_some((head, off))
}

/// Parse the wall-clock part of a timestamp, ignoring a trailing `Z` (as in
/// Immich's `localDateTime`) and any numeric offset.
pub fn parse_wall_clock(s: &str) -> Option<NaiveDateTime> {
    let t = s.trim().replacen(' ', "T", 1);
    let t = t
        .strip_suffix('Z')
        .or_else(|| t.strip_suffix('z'))
        .unwrap_or(&t);
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(t, fmt) {
            return Some(n);
        }
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s.trim()) {
        return Some(dt.naive_local());
    }
    None
}

/// Parse a fixed-offset time zone as Immich reports it: `UTC`, `UTC+2`,
/// `UTC-05:30`, `GMT+1`, `+02:00`, `Etc/GMT-2` (POSIX sign: UTC+2), `Z`.
/// IANA region names (`Europe/Berlin`) return `None`: resolving them needs a
/// time zone database, and Immich's instants already account for them.
pub fn parse_fixed_offset(tz: &str) -> Option<FixedOffset> {
    let tz = tz.trim();
    if tz.eq_ignore_ascii_case("utc")
        || tz.eq_ignore_ascii_case("gmt")
        || tz == "Z"
        || tz.eq_ignore_ascii_case("etc/utc")
    {
        return FixedOffset::east_opt(0);
    }
    let (rest, invert) = if let Some(r) = tz.strip_prefix("Etc/GMT") {
        (r, true)
    } else if let Some(r) = tz.strip_prefix("UTC").or_else(|| tz.strip_prefix("GMT")) {
        (r, false)
    } else {
        (tz, false)
    };
    let (sign, digits) = match rest.as_bytes().first()? {
        b'+' => (1, &rest[1..]),
        b'-' => (-1, &rest[1..]),
        _ => return None,
    };
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) => (h, m),
        None if digits.len() == 4 => digits.split_at(2),
        None => (digits, "0"),
    };
    let h: i32 = h.parse().ok()?;
    let m: i32 = m.parse().ok()?;
    if h > 14 || m > 59 {
        return None;
    }
    let secs = sign * (h * 3600 + m * 60) * if invert { -1 } else { 1 };
    FixedOffset::east_opt(secs)
}

/// The capture time of an asset in UTC microseconds.
///
/// Order: `exifInfo.dateTimeOriginal`, then `fileCreatedAt` (both instants),
/// then `localDateTime` converted with a fixed-offset `exifInfo.timeZone`
/// (or read as UTC if there is none). With [`NaiveTimes::Local`], an image
/// without a time zone uses its wall-clock time in the local zone instead.
pub fn creation_time_us(asset: &Asset, naive: NaiveTimes) -> Option<i64> {
    let exif = asset.exif_info.as_ref();
    let tz = exif
        .and_then(|e| e.time_zone.as_deref())
        .filter(|t| !t.trim().is_empty());
    if naive == NaiveTimes::Local
        && tz.is_none()
        && asset.kind == "IMAGE"
        && let Some(n) = asset.local_date_time.as_deref().and_then(parse_wall_clock)
        && let Some(dt) = Local.from_local_datetime(&n).earliest()
    {
        return Some(dt.with_timezone(&Utc).timestamp_micros());
    }
    exif.and_then(|e| e.date_time_original.as_deref())
        .and_then(parse_instant_us)
        .or_else(|| asset.file_created_at.as_deref().and_then(parse_instant_us))
        .or_else(|| {
            let n = asset
                .local_date_time
                .as_deref()
                .and_then(parse_wall_clock)?;
            let off = tz
                .and_then(parse_fixed_offset)
                .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"));
            Some(off.from_local_datetime(&n).single()?.timestamp_micros())
        })
}

/// The modification time of an asset (`fileModifiedAt`) in UTC microseconds.
pub fn modification_time_us(asset: &Asset) -> Option<i64> {
    asset.file_modified_at.as_deref().and_then(parse_instant_us)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(json: serde_json::Value) -> Asset {
        serde_json::from_value(json).expect("asset json")
    }

    #[test]
    fn instants() {
        assert_eq!(parse_instant_us("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_instant_us("1970-01-01T00:00:01.5Z"), Some(1_500_000));
        assert_eq!(
            parse_instant_us("2024-06-01T14:00:00.000Z"),
            Some(1_717_250_400_000_000)
        );
        assert_eq!(
            parse_instant_us("2024-06-01T16:00:00+02:00"),
            Some(1_717_250_400_000_000)
        );
        assert_eq!(
            parse_instant_us("2024-06-01T16:00:00+0200"),
            Some(1_717_250_400_000_000)
        );
        assert_eq!(
            parse_instant_us("2024-06-01 16:00:00+02"),
            Some(1_717_250_400_000_000)
        );
        assert_eq!(
            parse_instant_us("2024-06-01T14:00:00"),
            Some(1_717_250_400_000_000)
        );
        assert_eq!(
            parse_instant_us("2024-06-01T14:00:00.123456Z"),
            Some(1_717_250_400_123_456)
        );
        assert_eq!(
            parse_instant_us("1960-01-01T00:00:00Z"),
            Some(-315_619_200_000_000)
        );
        assert_eq!(parse_instant_us(""), None);
        assert_eq!(parse_instant_us("yesterday"), None);
    }

    #[test]
    fn wall_clock_ignores_z() {
        let n = parse_wall_clock("2024-06-01T14:00:00.000Z").expect("parse");
        assert_eq!(n.to_string(), "2024-06-01 14:00:00");
    }

    #[test]
    fn offsets() {
        let h = |x: i32| FixedOffset::east_opt(x);
        assert_eq!(parse_fixed_offset("UTC"), h(0));
        assert_eq!(parse_fixed_offset("UTC+2"), h(7200));
        assert_eq!(parse_fixed_offset("UTC-05:30"), h(-19800));
        assert_eq!(parse_fixed_offset("UTC+0530"), h(19800));
        assert_eq!(parse_fixed_offset("+09:00"), h(32400));
        assert_eq!(parse_fixed_offset("GMT-3"), h(-10800));
        assert_eq!(parse_fixed_offset("Etc/GMT-2"), h(7200));
        assert_eq!(parse_fixed_offset("Europe/Berlin"), None);
        assert_eq!(parse_fixed_offset("UTC+99"), None);
    }

    #[test]
    fn prefers_exif_instant_over_local_wall_clock() {
        // Taken 16:00 in Berlin (CEST, UTC+2) = 14:00Z.
        let a = asset(serde_json::json!({
            "id": "a", "type": "IMAGE",
            "fileCreatedAt": "2024-06-01T14:00:00.000Z",
            "localDateTime": "2024-06-01T16:00:00.000Z",
            "exifInfo": {"dateTimeOriginal": "2024-06-01T14:00:00.000Z", "timeZone": "Europe/Berlin"}
        }));
        assert_eq!(
            creation_time_us(&a, NaiveTimes::AsImmich),
            Some(1_717_250_400_000_000)
        );
        // A known zone is never overridden by the local-zone option.
        assert_eq!(
            creation_time_us(&a, NaiveTimes::Local),
            Some(1_717_250_400_000_000)
        );
    }

    #[test]
    fn falls_back_to_file_created_at() {
        let a = asset(serde_json::json!({
            "id": "a", "type": "VIDEO",
            "fileCreatedAt": "2020-01-01T00:00:00.000Z",
            "localDateTime": "2020-01-01T09:00:00.000Z",
            "exifInfo": {"dateTimeOriginal": null}
        }));
        assert_eq!(
            creation_time_us(&a, NaiveTimes::AsImmich),
            Some(1_577_836_800_000_000)
        );
    }

    #[test]
    fn local_date_time_needs_the_offset() {
        // Only localDateTime is usable: 09:00 at UTC+9 = 00:00Z.
        let a = asset(serde_json::json!({
            "id": "a", "type": "IMAGE",
            "fileCreatedAt": "garbage",
            "localDateTime": "2020-01-01T09:00:00.000Z",
            "exifInfo": {"timeZone": "UTC+9"}
        }));
        assert_eq!(
            creation_time_us(&a, NaiveTimes::AsImmich),
            Some(1_577_836_800_000_000)
        );
    }

    #[test]
    fn naive_local_mode_uses_wall_clock() {
        let a = asset(serde_json::json!({
            "id": "a", "type": "IMAGE",
            "fileCreatedAt": "2020-01-01T09:00:00.000Z",
            "localDateTime": "2020-01-01T09:00:00.000Z",
            "exifInfo": {"dateTimeOriginal": "2020-01-01T09:00:00.000Z", "timeZone": null}
        }));
        let n = parse_wall_clock("2020-01-01T09:00:00").expect("parse");
        let want = Local
            .from_local_datetime(&n)
            .earliest()
            .expect("local")
            .timestamp_micros();
        assert_eq!(creation_time_us(&a, NaiveTimes::Local), Some(want));
        assert_eq!(
            creation_time_us(&a, NaiveTimes::AsImmich),
            Some(1_577_869_200_000_000)
        );
    }

    #[test]
    fn modification_time() {
        let a = asset(serde_json::json!({"id": "a", "fileModifiedAt": "1970-01-01T00:00:02Z"}));
        assert_eq!(modification_time_us(&a), Some(2_000_000));
    }
}
