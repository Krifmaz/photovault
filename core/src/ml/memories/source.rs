//! The inputs of the memories engine, read from the core: the visible
//! files with their capture dates and locations, the albums to leave out
//! of "on this day", the people (named `cgroup`s) and, when ML is ready,
//! CLIP vectors and faces. Mirrors what Ente's `RustMemoriesComputer`
//! (`_loadInputs`), `memory_sources.dart` and the Rust crate's
//! `memories/store.rs` (`load_ml_inputs`) hand to the engine.
//!
//! Runs on a background thread (see the parent module): reading the
//! files' locations means opening every sealed file record, in chunks so
//! memory stays bounded for large libraries.
//!
//! Based on Ente (ente.com).

use super::engine::{
    ClipVectors, Coordinate, LocalDate, MemoriesRequest, MemoryFile, MemoryPerson, MlInputs,
    PeopleInputs,
};
use crate::client::Inner;
use crate::library::FileRec;
use crate::ml::entity::CGROUP;
use crate::ml::store::kv;
use crate::{Result, TimelineItem};
use rusqlite::params;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// Ente: `RustMemoriesComputer._minimumClipIndexedFiles`.
const MINIMUM_CLIP_INDEXED_FILES: i64 = 5;
/// Ente (`memories/store.rs`): weight of a face of a named person.
const NAMED_FACE_WEIGHT: u32 = 10;
/// Ente (`memories/store.rs`): weight of any other face.
const OTHER_FACE_WEIGHT: u32 = 1;
/// Sealed rows read per store lock.
const CHUNK: i64 = 1000;

/// Album names whose photos never make an "on this day" memory (Ente:
/// `getCollectionIDsToExclude`).
const EXCLUDED_ALBUM_WORDS: [&str; 18] = [
    "screenshot",
    "whatsapp",
    "telegram",
    "download",
    "facebook",
    "instagram",
    "messenger",
    "twitter",
    "reddit",
    "discord",
    "signal",
    "viber",
    "wechat",
    "line",
    "meme",
    "internet",
    "saved images",
    "document",
];

/// A named person, as the memories see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersonInfo {
    pub id: String,
    pub name: String,
    pub is_hidden: bool,
    pub birth_date: Option<LocalDate>,
    pub hide_from_memories: bool,
    pub email: Option<String>,
}

/// Visible files as the engine's [`MemoryFile`]s (capture time, wall
/// clock, location, album), the files without a sealed record (if any)
/// from their [`TimelineItem`] alone.
pub(crate) fn memory_files(inner: &Inner, utc_offset_secs: i32) -> Result<Vec<MemoryFile>> {
    let index = inner.index();
    let mut visible: HashMap<i64, &TimelineItem> =
        index.timeline_items().map(|item| (item.id, item)).collect();
    let mut files = Vec::with_capacity(visible.len());
    let mut after = i64::MIN;
    loop {
        let (rows, opener) = {
            let st = inner.store();
            let mut q = st
                .conn()
                .prepare("SELECT id, sealed FROM files WHERE id > ?1 ORDER BY id LIMIT ?2")?;
            let rows: Vec<(i64, Vec<u8>)> = q
                .query_map(params![after, CHUNK], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            (rows, st.opener())
        };
        let Some(&(last, _)) = rows.last() else {
            break;
        };
        after = last;
        for (id, blob) in rows {
            let Some(item) = visible.remove(&id) else {
                continue;
            };
            let Ok(rec) = opener.open_sealed::<FileRec>(&blob) else {
                files.push(file_from_item(item, utc_offset_secs, None, None));
                continue;
            };
            files.push(file_from_item(
                item,
                utc_offset_secs,
                local_date_time(&rec.pub_magic),
                location(&rec.metadata, &rec.pub_magic),
            ));
        }
    }
    files.extend(
        visible
            .into_values()
            .map(|item| file_from_item(item, utc_offset_secs, None, None)),
    );
    Ok(files)
}

/// Visible files from the index alone (no wall clock, no location): the
/// quick "simple memories" while the full computation runs.
pub(crate) fn quick_files(inner: &Inner, utc_offset_secs: i32) -> Vec<MemoryFile> {
    inner
        .index()
        .timeline_items()
        .map(|item| file_from_item(item, utc_offset_secs, None, None))
        .collect()
}

fn file_from_item(
    item: &TimelineItem,
    utc_offset_secs: i32,
    local_date_time: Option<String>,
    location: Option<Coordinate>,
) -> MemoryFile {
    MemoryFile {
        id: item.id,
        creation_time: item.taken_at_us,
        local_date_time,
        // Ente uses the device's offset at the capture time; the core only
        // knows the caller's current offset.
        utc_offset_seconds: utc_offset_secs,
        collection_id: Some(item.collection_id),
        location,
    }
}

/// `pubMagicMetadata.dateTime`: the capture wall clock.
fn local_date_time(pub_magic: &Map<String, Value>) -> Option<String> {
    pub_magic
        .get("dateTime")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The file's location: `pubMagicMetadata.lat/long` (edited) over
/// `metadata.latitude/longitude`, like Ente's files DB.
pub(crate) fn location(metadata: &Value, pub_magic: &Map<String, Value>) -> Option<Coordinate> {
    let number = |v: Option<&Value>| -> Option<f64> {
        match v? {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    };
    let edited = number(pub_magic.get("lat")).zip(number(pub_magic.get("long")));
    let original = number(metadata.get("latitude")).zip(number(metadata.get("longitude")));
    edited
        .or(original)
        .filter(|(lat, lon)| lat.is_finite() && lon.is_finite())
        .map(|(lat, lon)| Coordinate::new(lat, lon))
}

/// Visible albums whose names mark them as screenshots, downloads,
/// messengers and the like.
pub(crate) fn excluded_collection_ids(inner: &Inner) -> Vec<i64> {
    inner
        .index()
        .visible_collections()
        .iter()
        .filter(|c| {
            let name = c.name.to_lowercase();
            EXCLUDED_ALBUM_WORDS.iter().any(|w| name.contains(w))
        })
        .map(|c| c.id)
        .collect()
}

/// Named people with the `cgroup` fields the memories read.
pub(crate) fn people(inner: &Inner) -> Vec<PersonInfo> {
    let entities: HashMap<String, Value> = inner
        .store()
        .entities(CGROUP)
        .unwrap_or_default()
        .into_iter()
        .map(|(id, _, data)| (id, data))
        .collect();
    crate::ml::faces::people(inner, true, 0)
        .into_iter()
        .filter_map(|p| {
            let name = p.name.filter(|n| !n.is_empty())?;
            let data = entities.get(&p.id);
            let field = |k: &str| data.and_then(|d| d.get(k));
            Some(PersonInfo {
                birth_date: p.birth_date.as_deref().and_then(parse_birth_date),
                hide_from_memories: field("hideFromMemories")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                email: field("email").and_then(Value::as_str).map(str::to_owned),
                id: p.id,
                name,
                is_hidden: p.is_hidden,
            })
        })
        .collect()
}

/// `YYYY-MM-DD` (Dart's `DateTime.tryParse` accepts a time after it too).
pub(crate) fn parse_birth_date(s: &str) -> Option<LocalDate> {
    let b = s.trim().as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| std::str::from_utf8(&b[r]).ok()?.parse::<i32>().ok();
    let (year, month, day) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let valid = (1..=12).contains(&month) && (1..=31).contains(&day);
    valid.then_some(LocalDate { year, month, day })
}

/// Fill the people part of a request (Ente: persons not hidden from
/// memories; me = the person whose email is the account's).
pub(crate) fn add_people(request: &mut MemoriesRequest, people: &[PersonInfo], email: &str) {
    request.people = people
        .iter()
        .filter(|p| !p.hide_from_memories)
        .map(|p| MemoryPerson {
            id: p.id.clone(),
            is_ignored: p.is_hidden,
            birth_date: p.birth_date,
        })
        .collect();
    request.people_hidden_from_memories = people
        .iter()
        .filter(|p| p.hide_from_memories)
        .map(|p| p.id.clone())
        .collect();
    request.me_person_id = people
        .iter()
        .filter(|p| !p.hide_from_memories)
        .find(|p| {
            p.email
                .as_deref()
                .is_some_and(|e| e.eq_ignore_ascii_case(email))
        })
        .map(|p| p.id.clone());
}

/// Ente's `isMlReady`: ML on, and at least a few files with CLIP indexes.
pub(crate) fn ml_ready(inner: &Inner) -> bool {
    let st = inner.store();
    let enabled = st
        .get_plain(kv::ENABLED)
        .ok()
        .flatten()
        .is_some_and(|v| v == "1");
    enabled
        && st
            .conn()
            .query_row("SELECT COUNT(*) FROM ml_data WHERE clip_ok = 1", [], |r| {
                r.get::<_, i64>(0)
            })
            .is_ok_and(|n| n >= MINIMUM_CLIP_INDEXED_FILES)
}

/// CLIP through the search module's matrix; single vectors from the
/// file's `mldata`.
pub(crate) struct InnerClip<'a> {
    pub inner: &'a Inner,
}

impl ClipVectors for InnerClip<'_> {
    fn matches(&self, query: &[f32], min_similarity: f32) -> Result<Vec<(i64, f32)>> {
        // Ente's theme embeddings are quantised to i8: normalise them so
        // the scores are cosine similarities.
        Ok(crate::ml::search::clip_matches(
            self.inner,
            &unit(query),
            min_similarity,
        ))
    }

    fn vector(&self, file_id: i64) -> Result<Option<Vec<f32>>> {
        let d = self.inner.store().ml_data(file_id)?;
        Ok(d.and_then(|d| {
            d.current_clip()
                .map(|c| c.embedding.iter().map(|&v| v as f32).collect())
        }))
    }
}

pub(crate) fn unit(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter().map(|x| x / norm).collect()
    } else {
        v.to_vec()
    }
}

/// Only the face ids of a file's `mldata` (the rest is skipped unparsed).
#[derive(Deserialize)]
struct FacesOnly {
    face: Option<FaceIndexIds>,
}

#[derive(Deserialize)]
struct FaceIndexIds {
    #[serde(default)]
    faces: Vec<FaceId>,
}

#[derive(Deserialize)]
struct FaceId {
    #[serde(rename = "faceID")]
    _face_id: IgnoredAny,
}

/// Faces per file, from every stored `mldata` with a current face index.
/// A file indexed without faces counts one (Ente stores an empty face for
/// it).
fn face_counts(inner: &Inner) -> Result<HashMap<i64, u32>> {
    let mut counts = HashMap::new();
    let mut after = i64::MIN;
    loop {
        let (rows, opener) = {
            let st = inner.store();
            let mut q = st.conn().prepare(
                "SELECT file_id, sealed FROM ml_data WHERE face_ok = 1 AND file_id > ?1 \
                 ORDER BY file_id LIMIT ?2",
            )?;
            let rows: Vec<(i64, Vec<u8>)> = q
                .query_map(params![after, CHUNK], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            (rows, st.opener())
        };
        let Some(&(last, _)) = rows.last() else {
            break;
        };
        after = last;
        for (file_id, blob) in rows {
            if let Ok(FacesOnly { face: Some(f) }) = opener.open_sealed::<FacesOnly>(&blob) {
                counts.insert(file_id, f.faces.len().max(1) as u32);
            }
        }
    }
    Ok(counts)
}

/// The ML inputs (Ente's `load_ml_inputs`): faces per file, their
/// ranking weights (named people count more) and each person's files.
pub(crate) fn ml_inputs<'a>(
    inner: &'a Inner,
    clip: &'a InnerClip<'a>,
    request: &MemoriesRequest,
) -> Result<MlInputs<'a>> {
    let person_file_ids: HashMap<String, HashSet<i64>> = request
        .people
        .iter()
        .map(|p| {
            let files = crate::ml::faces::person_file_ids(inner, &p.id)
                .into_iter()
                .collect();
            (p.id.clone(), files)
        })
        .collect();
    let face_counts = face_counts(inner)?;
    Ok(MlInputs {
        clip,
        face_weights: face_weights(&face_counts, &person_file_ids),
        people: PeopleInputs {
            face_counts,
            person_file_ids,
            // Ente shows unnamed clusters only with a developer option.
            unnamed_clusters: Vec::new(),
        },
    })
}

/// Named faces weigh [`NAMED_FACE_WEIGHT`], others [`OTHER_FACE_WEIGHT`].
/// A named person is on a file at most once, so the named faces of a file
/// are the people whose files hold it.
pub(crate) fn face_weights(
    face_counts: &HashMap<i64, u32>,
    person_file_ids: &HashMap<String, HashSet<i64>>,
) -> HashMap<i64, u32> {
    face_counts
        .iter()
        .map(|(&file_id, &count)| {
            let named = person_file_ids
                .values()
                .filter(|files| files.contains(&file_id))
                .count() as u32;
            let named = named.min(count);
            (
                file_id,
                named * NAMED_FACE_WEIGHT + (count - named) * OTHER_FACE_WEIGHT,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edited_locations_win_and_strings_parse() {
        let md = json!({"latitude": "48.85", "longitude": 2.35});
        let mut pm = Map::new();
        assert_eq!(location(&md, &pm), Some(Coordinate::new(48.85, 2.35)));
        pm.insert("lat".into(), json!(1.5));
        pm.insert("long".into(), json!(-2.0));
        assert_eq!(location(&md, &pm), Some(Coordinate::new(1.5, -2.0)));
        assert_eq!(location(&json!({}), &Map::new()), None);
    }

    #[test]
    fn birth_dates_and_people() {
        assert_eq!(
            parse_birth_date("1990-02-03"),
            Some(LocalDate {
                year: 1990,
                month: 2,
                day: 3
            })
        );
        assert_eq!(parse_birth_date("1990-13-03"), None);
        assert_eq!(parse_birth_date(""), None);
        let p = |id: &str, email: Option<&str>, hide| PersonInfo {
            id: id.into(),
            name: id.into(),
            is_hidden: false,
            birth_date: None,
            hide_from_memories: hide,
            email: email.map(str::to_owned),
        };
        let mut r = MemoriesRequest::default();
        add_people(
            &mut r,
            &[
                p("a", Some("Me@x.org"), false),
                p("b", None, true),
                p("c", None, false),
            ],
            "me@x.org",
        );
        assert_eq!(r.people.len(), 2);
        assert_eq!(r.people_hidden_from_memories, ["b"]);
        assert_eq!(r.me_person_id.as_deref(), Some("a"));
    }

    #[test]
    fn named_faces_weigh_ten() {
        let counts = HashMap::from([(1, 2), (2, 1), (3, 1)]);
        let people = HashMap::from([("p".to_string(), HashSet::from([1, 2]))]);
        assert_eq!(
            face_weights(&counts, &people),
            HashMap::from([(1, 11), (2, 10), (3, 1)])
        );
    }
}
