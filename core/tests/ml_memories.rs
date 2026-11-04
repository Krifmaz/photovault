//! Memories end to end against the real Photovault server: a synthetic
//! library (photos with fixed EXIF dates and GPS positions, uploaded the
//! way the app does), then on this day across years and time zones, a trip
//! away from the home base, the per-day cache, the seen state across a
//! restart, and nothing readable at rest.
//!
//! People memories need faces (another track); the engine's people rules
//! are covered by the unit tests in `core/src/ml/memories/engine/people.rs`.

mod support;

use chrono::NaiveDate;
use exif::experimental::Writer;
use exif::{Field, In, Rational, Tag, Value};
use photovault_core::ml::{Memory, MemoryKind};
use photovault_core::{Core, LoginStep};
use std::sync::Arc;
use std::time::{Duration, Instant};
use support::{TestCore, jpeg, server, signup};

const EMAIL: &str = "mem@example.org";
const PASSWORD: &str = "memories are made of this";
const HOUR: i64 = 3_600_000_000;

type LatLon = (f64, f64);

/// µs since the epoch of a UTC date-time.
fn us(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    NaiveDate::from_ymd_opt(y, mo, d)
        .unwrap()
        .and_hms_opt(h, mi, 0)
        .unwrap()
        .and_utc()
        .timestamp_micros()
}

/// A JPEG with EXIF DateTimeOriginal (UTC) and, optionally, GPS.
fn photo(seed: u32, at_us: i64, gps: Option<(f64, f64)>) -> Vec<u8> {
    let dt = chrono::DateTime::from_timestamp_micros(at_us).unwrap();
    let date = Field {
        tag: Tag::DateTimeOriginal,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![
            dt.format("%Y:%m:%d %H:%M:%S").to_string().into_bytes(),
        ]),
    };
    let offset = Field {
        tag: Tag::OffsetTimeOriginal,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![b"+00:00".to_vec()]),
    };
    let dms = |v: f64| {
        let v = v.abs();
        let deg = v.trunc();
        let min = ((v - deg) * 60.0).trunc();
        let sec = ((v - deg) * 60.0 - min) * 60.0;
        Value::Rational(vec![
            Rational::from((deg as u32, 1)),
            Rational::from((min as u32, 1)),
            Rational::from(((sec * 1000.0).round() as u32, 1000)),
        ])
    };
    let mut fields = vec![date, offset];
    if let Some((lat, lon)) = gps {
        let r = |s: &str| Value::Ascii(vec![s.as_bytes().to_vec()]);
        for (tag, value) in [
            (Tag::GPSLatitude, dms(lat)),
            (Tag::GPSLatitudeRef, r(if lat < 0.0 { "S" } else { "N" })),
            (Tag::GPSLongitude, dms(lon)),
            (Tag::GPSLongitudeRef, r(if lon < 0.0 { "W" } else { "E" })),
        ] {
            fields.push(Field {
                tag,
                ifd_num: In::PRIMARY,
                value,
            });
        }
    }
    let mut w = Writer::new();
    for f in &fields {
        w.push_field(f);
    }
    let mut tiff = std::io::Cursor::new(Vec::new());
    w.write(&mut tiff, false).unwrap();
    let tiff = tiff.into_inner();
    let image = jpeg(24 + seed % 17, 16 + seed / 17, (seed * 7) as u8);
    // SOI, then an APP1 "Exif" segment, then the rest of the JPEG.
    let mut out = image[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&tiff);
    out.extend_from_slice(&image[2..]);
    out
}

/// Wait (bounded) until the background update has produced what `done`
/// expects.
fn eventually(core: &Core, now: i64, offset: i32, done: impl Fn(&[Memory]) -> bool) -> Vec<Memory> {
    let start = Instant::now();
    loop {
        let m = core.memories(now, offset);
        if done(&m) {
            return m;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "memories never got there: {m:#?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn titles(m: &Memory) -> Vec<String> {
    let mut t: Vec<String> = m.items.iter().map(|i| i.title.clone()).collect();
    t.sort();
    t
}

fn of_kind(memories: &[Memory], kind: MemoryKind) -> Vec<&Memory> {
    memories.iter().filter(|m| m.kind == kind).collect()
}

async fn login(c: &TestCore) {
    assert!(matches!(
        c.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap(),
        LoginStep::Done { .. }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memories_end_to_end() {
    let srv = server(&[EMAIL]).await;
    signup(&srv.url, EMAIL, PASSWORD).await;
    let a = TestCore::connected(&srv.url).await;
    login(&a).await;

    // The library.
    let mut files: Vec<(String, i64, Option<LatLon>)> = Vec::new();
    // On this day (Nov 4) in 2023 and 2021.
    for (y, n) in [(2023, 3), (2021, 3)] {
        for i in 0..n {
            files.push((format!("otd_{y}_{i}.jpg"), us(y, 11, 4, 9, 20 * i), None));
        }
    }
    // Late on Nov 5 UTC (Nov 6 east of UTC+1) in 2020 and 2019.
    for (y, n) in [(2020, 3), (2019, 2)] {
        for i in 0..n {
            files.push((format!("tz_{y}_{i}.jpg"), us(y, 11, 5, 23, 15 * i), None));
        }
    }
    // Home in Amsterdam: every five days for two months.
    let home = (52.37, 4.90);
    for i in 0..14 {
        let t = us(2024, 1, 1, 12, 0) + i * 5 * 24 * HOUR;
        files.push((format!("home_{i}.jpg"), t, Some(home)));
    }
    // A trip to Lisbon: 20 photos over two and a half days.
    let lisbon = (38.7223, -9.1393);
    for i in 0..20 {
        let t = us(2024, 6, 10, 8, 0) + i * 3 * HOUR;
        files.push((format!("trip_{i}.jpg"), t, Some(lisbon)));
    }
    for (seed, (name, at, gps)) in files.iter().enumerate() {
        let path = a.dir.path().join(name);
        std::fs::write(&path, photo(seed as u32, *at, *gps)).unwrap();
        a.core
            .enqueue_upload(path.display().to_string(), None)
            .unwrap();
    }
    let st = a.core.run_uploads().await.unwrap();
    assert_eq!((st.pending, st.failed), (0, 0), "{st:?}");
    assert_eq!(a.core.timeline_count(), files.len() as u64);

    // First call: no cache yet. Ente's quick simple memories (on this day)
    // come back at once while the full update runs in the background.
    let now = us(2026, 11, 4, 10, 0);
    let first = a.core.memories(now, 0);
    let otd = of_kind(&first, MemoryKind::OnThisDay);
    assert_eq!(otd.len(), 1, "{first:#?}");
    assert_eq!(otd[0].title, "On this day");

    // The full update: on this day across 2021 and 2023, and the trip.
    let full = eventually(&a.core, now, 0, |m| {
        !of_kind(m, MemoryKind::Trip).is_empty()
    });
    let otd = of_kind(&full, MemoryKind::OnThisDay);
    assert_eq!(otd.len(), 1, "{full:#?}");
    assert_eq!(
        titles(otd[0]),
        [
            "otd_2021_0.jpg",
            "otd_2021_1.jpg",
            "otd_2021_2.jpg",
            "otd_2023_0.jpg",
            "otd_2023_1.jpg",
            "otd_2023_2.jpg"
        ]
    );
    // Story order is chronological.
    let taken: Vec<i64> = otd[0].items.iter().map(|i| i.taken_at_us).collect();
    assert!(taken.is_sorted(), "{taken:?}");
    assert_eq!(otd[0].cover_file_id, otd[0].items[0].id);
    let trips = of_kind(&full, MemoryKind::Trip);
    assert_eq!(trips.len(), 1, "{full:#?}");
    let trip = trips[0].clone();
    assert_eq!(trip.title, "Trip in 2024");
    assert_eq!(trip.items.len(), 10);
    assert!(trip.items.iter().all(|i| i.title.starts_with("trip_")));
    assert!(full.iter().all(|m| !m.seen && !m.items.is_empty()));
    let ids: std::collections::HashSet<&str> = full.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids.len(), full.len());

    // Per-day cache: the same list, fast, all day.
    let t = Instant::now();
    assert_eq!(a.core.memories(now, 0), full);
    assert_eq!(a.core.memories(now + 5 * HOUR, 0), full);
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "{:?}",
        t.elapsed()
    );

    // The next day (UTC): tomorrow's on this day was computed with today's:
    // the photos from late Nov 5 UTC.
    let next_day = a.core.memories(us(2026, 11, 5, 12, 0), 0);
    let otd = of_kind(&next_day, MemoryKind::OnThisDay);
    assert_eq!(otd.len(), 1, "{next_day:#?}");
    assert!(otd[0].items.iter().all(|i| i.title.starts_with("tz_")));
    assert_eq!(otd[0].items.len(), 5);

    // Seen: persisted and sorted last.
    a.core.mark_memory_seen(trip.id.clone()).unwrap();
    let after = a.core.memories(now, 0);
    assert_eq!(after.len(), full.len());
    let last = after.last().unwrap();
    assert_eq!((last.id.as_str(), last.seen), (trip.id.as_str(), true));
    assert!(after[..after.len() - 1].iter().all(|m| !m.seen));

    // Nothing about the memories is readable at rest.
    for path in walk(&a.data_dir()) {
        let bytes = std::fs::read(&path).unwrap();
        for needle in [
            &b"trip_"[..],
            b"file_ids",
            b"Trip in",
            &trip.id.as_bytes()[..12],
        ] {
            assert!(
                !bytes.windows(needle.len()).any(|w| w == needle),
                "{} holds {:?}",
                path.display(),
                String::from_utf8_lossy(needle)
            );
        }
    }

    // Restart: the stored cache answers at once, seen state intact.
    let (data, cache) = (a.data_dir(), a.cache_dir());
    let a2 = Core::new(data.display().to_string(), cache.display().to_string()).unwrap();
    drop(a);
    let again = a2.memories(now + HOUR, 0);
    assert_eq!(again.len(), full.len(), "{again:#?}");
    let trip_again = again.iter().find(|m| m.id == trip.id).unwrap();
    assert!(trip_again.seen);
    assert_eq!(trip_again.items, trip.items);
    drop(a2);

    // Another device east of UTC (+03:00) late on Nov 5 UTC: it is Nov 6
    // there, and on this day holds the photos taken late on Nov 5 UTC.
    let b = TestCore::connected(&srv.url).await;
    login(&b).await;
    b.core.sync(None).await.unwrap();
    let local_nov6 = us(2026, 11, 5, 22, 0);
    let east = 3 * 3600;
    let quick = b.core.memories(local_nov6, east);
    let otd = of_kind(&quick, MemoryKind::OnThisDay);
    assert_eq!(otd.len(), 1, "{quick:#?}");
    assert!(otd[0].items.iter().all(|i| i.title.starts_with("tz_")));
    let full_b = eventually(&b.core, local_nov6, east, |m| {
        !of_kind(m, MemoryKind::Trip).is_empty()
    });
    let otd = of_kind(&full_b, MemoryKind::OnThisDay);
    assert_eq!(otd.len(), 1, "{full_b:#?}");
    assert_eq!(otd[0].items.len(), 5);
    assert!(otd[0].items.iter().all(|i| i.title.starts_with("tz_")));
    // The seen state is per device.
    assert!(full_b.iter().all(|m| !m.seen));
    let _: &Arc<Core> = &b.core;
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
