//! Tests of the memories pipeline around Ente's engine (inputs → engine →
//! cache → titles).

use super::cache::{self, StoredCache, StoredKind};
use super::engine::{self, ClipVectors, MemoriesRequest, MemoryFile, Period};
use super::{source, titles};
use crate::Result;
use std::collections::{HashMap, HashSet};

const DAY: i64 = cache::MICROS_PER_DAY;

/// No CLIP index: themes and nostalgia stay empty.
struct NoClip;

impl ClipVectors for NoClip {
    fn matches(&self, _: &[f32], _: f32) -> Result<Vec<(i64, f32)>> {
        Ok(Vec::new())
    }
    fn vector(&self, _: i64) -> Result<Option<Vec<f32>>> {
        Ok(None)
    }
}

/// Noon UTC of a date.
fn at(year: i32, month: i32, day: i32) -> i64 {
    engine::LocalDate { year, month, day }.days_since_epoch_pub() * DAY + 12 * 3_600_000_000
}

/// A person with fake face data: a birthday two days away gives Ente's
/// lead-up and birthday spotlights, titled with the person's name.
#[test]
fn people_memories_from_fake_faces() {
    let ana: Vec<i64> = (1..=8).collect();
    let files: Vec<MemoryFile> = ana
        .iter()
        .map(|&id| MemoryFile {
            id,
            creation_time: at(2017 + id as i32, 3, 10),
            local_date_time: None,
            utc_offset_seconds: 0,
            collection_id: Some(1),
            location: None,
        })
        .collect();
    let people = [source::PersonInfo {
        id: "ana".into(),
        name: "Ana".into(),
        is_hidden: false,
        birth_date: source::parse_birth_date("1996-11-06"),
        hide_from_memories: false,
        email: None,
    }];
    let mut request = MemoriesRequest {
        files,
        seed: 1,
        ..MemoriesRequest::default()
    };
    source::add_people(&mut request, &people, "me@example.org");
    let face_counts: HashMap<i64, u32> = ana.iter().map(|&id| (id, 1)).collect();
    let person_files = HashMap::from([(
        "ana".to_string(),
        ana.iter().copied().collect::<HashSet<_>>(),
    )]);
    let ml = engine::MlInputs {
        clip: &NoClip,
        face_weights: source::face_weights(&face_counts, &person_files),
        people: engine::PeopleInputs {
            face_counts,
            person_file_ids: person_files,
            unnamed_clusters: Vec::new(),
        },
    };
    let now = at(2026, 11, 4);
    let next = now + cache::UPDATE_FREQUENCY;
    let periods = [now, next].map(|now| Period {
        now,
        utc_offset_seconds: 0,
    });
    let mut out = engine::compute_smart_memories(&request, &periods, Some(&ml), None).unwrap();
    let next_memories = out.periods.pop().unwrap();
    let now_memories = out.periods.pop().unwrap();
    let c = cache::merge(
        StoredCache::empty(1),
        now,
        next,
        now_memories,
        next_memories,
    );
    let shown = |t: i64| -> Vec<String> {
        c.showing(t)
            .filter(|m| matches!(m.kind, StoredKind::People { .. }))
            .map(|m| titles::title(&m.kind, Some("Ana"), false, 2026))
            .collect()
    };
    assert_eq!(shown(now), ["Ana turning 30 soon"]);
    assert_eq!(shown(at(2026, 11, 6)), ["Ana is 30!"]);
    let birthday = c
        .memories
        .iter()
        .find(|m| matches!(m.kind, StoredKind::People { .. }))
        .unwrap();
    // Ente's person selection: newest first.
    assert_eq!(birthday.file_ids, [8, 7, 6, 5, 4, 3, 2, 1]);
}
