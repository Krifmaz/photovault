//! Places and dates for the map and the home-screen widgets.
//!
//! * [`location`]: a file's location as Ente's apps read it: the user's edit
//!   in public magic metadata (`lat`, `long`) wins over the uploaded
//!   metadata (`latitude`, `longitude`). `0, 0` and out-of-range values mean
//!   "no location", as in Ente's `Location.isValidLocation`.
//! * [`on_this_day`]: timeline items taken on today's calendar date in
//!   earlier years ("on this day"), in the device's UTC offset.

use crate::TimelineItem;
use chrono::{DateTime, Datelike, FixedOffset};
use serde_json::{Map, Value};

fn f64_of(m: &Map<String, Value>, k: &str) -> Option<f64> {
    m.get(k).and_then(Value::as_f64)
}

fn valid(lat: f64, lon: f64) -> Option<(f64, f64)> {
    let ok = lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0);
    ok.then_some((lat, lon))
}

/// The file's location, if it has a usable one.
pub(crate) fn location(pub_magic: &Map<String, Value>, metadata: &Value) -> Option<(f64, f64)> {
    if let (Some(lat), Some(lon)) = (f64_of(pub_magic, "lat"), f64_of(pub_magic, "long")) {
        return valid(lat, lon);
    }
    let md = metadata.as_object()?;
    valid(f64_of(md, "latitude")?, f64_of(md, "longitude")?)
}

fn local(us: i64, offset: FixedOffset) -> Option<DateTime<FixedOffset>> {
    DateTime::from_timestamp_micros(us).map(|t| t.with_timezone(&offset))
}

/// Items (newest first) taken on `now_us`'s month and day in an earlier year,
/// at most `limit`, in the order given.
pub(crate) fn on_this_day<'a>(
    items: impl Iterator<Item = &'a TimelineItem>,
    now_us: i64,
    utc_offset_secs: i32,
    limit: u32,
) -> Vec<TimelineItem> {
    let Some(offset) = FixedOffset::east_opt(utc_offset_secs) else {
        return Vec::new();
    };
    let Some(today) = local(now_us, offset) else {
        return Vec::new();
    };
    items
        .filter(|it| it.taken_at_us > 0)
        .filter(|it| {
            local(it.taken_at_us, offset).is_some_and(|t| {
                t.year() < today.year() && t.month() == today.month() && t.day() == today.day()
            })
        })
        .take(limit as usize)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileKind;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn edited_location_wins_over_metadata() {
        let md = json!({"latitude": 48.85, "longitude": 2.35});
        assert_eq!(location(&Map::new(), &md), Some((48.85, 2.35)));
        let pm = obj(json!({"lat": 52.52, "long": 13.4}));
        assert_eq!(location(&pm, &md), Some((52.52, 13.4)));
    }

    #[test]
    fn missing_zero_and_out_of_range_are_no_location() {
        assert_eq!(location(&Map::new(), &json!({"title": "a.jpg"})), None);
        assert_eq!(
            location(&Map::new(), &json!({"latitude": 0.0, "longitude": 0.0})),
            None
        );
        assert_eq!(
            location(&Map::new(), &json!({"latitude": 91.0, "longitude": 2.0})),
            None
        );
        assert_eq!(
            location(&Map::new(), &json!({"latitude": 1.0, "longitude": null})),
            None
        );
        // A user who cleared the location (0, 0 in magic metadata) hides the original one.
        let pm = obj(json!({"lat": 0.0, "long": 0.0}));
        assert_eq!(
            location(&pm, &json!({"latitude": 48.85, "longitude": 2.35})),
            None
        );
    }

    fn item(id: i64, rfc3339: &str) -> TimelineItem {
        TimelineItem {
            id,
            collection_id: 1,
            kind: FileKind::Image,
            taken_at_us: DateTime::parse_from_rfc3339(rfc3339)
                .unwrap()
                .timestamp_micros(),
            width: 0,
            height: 0,
            duration_ms: None,
            has_stream: false,
            title: String::new(),
        }
    }

    #[test]
    fn on_this_day_matches_the_local_date_in_earlier_years() {
        let items = [
            item(1, "2026-11-04T09:00:00Z"), // today: not a memory
            item(2, "2025-11-04T23:30:00Z"), // 2025-11-05 00:30 at +01:00: no
            item(3, "2024-11-04T12:00:00Z"),
            item(4, "2024-11-03T23:30:00Z"), // 2024-11-04 00:30 at +01:00: yes
            item(5, "2023-11-05T12:00:00Z"),
            item(6, "2020-11-04T08:00:00Z"),
        ];
        let now = DateTime::parse_from_rfc3339("2026-11-04T10:00:00Z")
            .unwrap()
            .timestamp_micros();
        let ids: Vec<i64> = on_this_day(items.iter(), now, 3600, 10)
            .iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(ids, vec![3, 4, 6]);
        assert_eq!(on_this_day(items.iter(), now, 3600, 2).len(), 2);
        assert!(on_this_day(items.iter(), now, 999_999, 10).is_empty());
    }
}
