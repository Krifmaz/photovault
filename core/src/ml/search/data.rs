//! What search runs on, kept in memory and rebuilt lazily:
//!
//! - [`SearchData`]: the files the user can see (the library index minus
//!   files that are only in hidden collections, like Ente's
//!   `getAllFilesForSearch`), with lowercase names; cheap, rebuilt from the
//!   index whenever it changes.
//! - [`Extras`]: captions and locations, which only the sealed file records
//!   hold; built off the caller's thread by reading the store in chunks.
//! - [`ClipMatrix`]: the current CLIP image embeddings from the stored
//!   `mldata`, also read in chunks.
//!
//! Based on Ente (ente.com).

use super::clip::{ClipMatrix, Rescorer};
use super::places::file_location;
use crate::client::Inner;
use crate::library::Index;
use crate::{Result, TimelineItem};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Rows read per store round trip (the store lock is held only for that).
const CHUNK: i64 = 256;

/// The visible files of one library index.
#[derive(Debug)]
pub(crate) struct SearchData {
    /// The index this was built from.
    pub index: Arc<Index>,
    /// Positions in `index.all_items()` of visible files, ascending
    /// (newest first).
    pub visible: Vec<u32>,
    /// Lowercase file names, parallel to `visible`.
    pub names: Vec<Box<str>>,
    /// Visible file id → position in `index.all_items()`.
    pub by_id: HashMap<i64, u32>,
}

impl SearchData {
    /// Build from an index: files in any visible (not hidden) collection.
    pub(crate) fn build(index: Arc<Index>) -> Self {
        let all = index.all_items();
        let mut pos_of: HashMap<i64, u32> = HashMap::with_capacity(all.len());
        for (p, it) in all.iter().enumerate() {
            pos_of.insert(it.id, p as u32);
        }
        let mut seen = vec![false; all.len()];
        for c in index.visible_collections() {
            for it in index.collection_items(c.id) {
                if let Some(&p) = pos_of.get(&it.id) {
                    seen[p as usize] = true;
                }
            }
        }
        // Timeline files are always in a visible collection; kept for safety.
        for it in index.timeline_items() {
            if let Some(&p) = pos_of.get(&it.id) {
                seen[p as usize] = true;
            }
        }
        let visible: Vec<u32> = (0..all.len() as u32)
            .filter(|p| seen[*p as usize])
            .collect();
        pos_of.retain(|_, p| seen[*p as usize]);
        let names = visible
            .iter()
            .map(|&p| all[p as usize].title.to_lowercase().into_boxed_str())
            .collect();
        Self {
            index,
            visible,
            names,
            by_id: pos_of,
        }
    }

    /// The item at a position.
    pub(crate) fn item(&self, pos: u32) -> &TimelineItem {
        &self.index.all_items()[pos as usize]
    }

    /// Visible items with the given ids, newest first.
    pub(crate) fn items_of(&self, ids: impl IntoIterator<Item = i64>) -> Vec<TimelineItem> {
        let mut pos: Vec<u32> = ids
            .into_iter()
            .filter_map(|id| self.by_id.get(&id).copied())
            .collect();
        pos.sort_unstable();
        pos.dedup();
        pos.into_iter().map(|p| self.item(p).clone()).collect()
    }

    /// Indexes into `visible` of files taken within `[start, end)` µs,
    /// newest first (`visible` is sorted by capture time, newest first).
    pub(crate) fn taken_within(&self, start: i64, end: i64) -> std::ops::Range<usize> {
        let t = |i: &u32| self.item(*i).taken_at_us;
        let a = self.visible.partition_point(|i| t(i) >= end);
        let b = self.visible.partition_point(|i| t(i) >= start);
        a..b.max(a)
    }
}

/// Captions and locations of the library's files (visible or not; callers
/// filter with [`SearchData::by_id`]).
#[derive(Debug, Default)]
pub(crate) struct Extras {
    /// `(file id, lowercase caption)`.
    pub captions: Vec<(i64, Box<str>)>,
    /// `(file id, latitude, longitude)`.
    pub locations: Vec<(i64, f64, f64)>,
}

/// The parts of a sealed file record search reads.
#[derive(Deserialize)]
struct FilePart {
    id: i64,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    pub_magic: Map<String, Value>,
}

impl Extras {
    /// Read every file record, a chunk at a time.
    pub(crate) fn build(inner: &Inner) -> Result<Self> {
        let mut out = Self::default();
        let mut after = i64::MIN;
        loop {
            let (opener, rows): (_, Vec<(i64, Vec<u8>)>) = {
                let st = inner.store();
                let mut q = st
                    .conn()
                    .prepare("SELECT id, sealed FROM files WHERE id > ?1 ORDER BY id LIMIT ?2")?;
                let rows = q
                    .query_map(params![after, CHUNK], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                (st.opener(), rows)
            };
            let Some(last) = rows.last().map(|r| r.0) else {
                break;
            };
            after = last;
            for (_, blob) in &rows {
                let Ok(f) = opener.open_sealed::<FilePart>(blob) else {
                    continue;
                };
                if let Some(c) = f
                    .pub_magic
                    .get("caption")
                    .and_then(Value::as_str)
                    .filter(|c| !c.is_empty())
                {
                    out.captions.push((f.id, c.to_lowercase().into_boxed_str()));
                }
                if let Some((lat, long)) = file_location(&f.metadata, &f.pub_magic) {
                    out.locations.push((f.id, lat, long));
                }
            }
            if (rows.len() as i64) < CHUNK {
                break;
            }
        }
        out.captions.shrink_to_fit();
        out.locations.shrink_to_fit();
        Ok(out)
    }
}

/// The `clip` part of a sealed `mldata` object.
#[derive(Deserialize)]
struct MlPart {
    #[serde(default)]
    clip: Option<ClipPart>,
}

#[derive(Deserialize)]
struct ClipPart {
    #[serde(default)]
    embedding: Vec<f64>,
}

/// Build the CLIP matrix from every stored `mldata` whose CLIP index is
/// current (`ml_data.clip_ok`), a chunk at a time.
pub(crate) fn build_clip(inner: &Inner) -> Result<ClipMatrix> {
    let rows: i64 = inner.store().conn().query_row(
        "SELECT COUNT(*) FROM ml_data WHERE clip_ok = 1",
        [],
        |r| r.get(0),
    )?;
    let mut m: Option<ClipMatrix> = None;
    let mut after = i64::MIN;
    loop {
        let (opener, chunk): (_, Vec<(i64, Vec<u8>)>) = {
            let st = inner.store();
            let mut q = st.conn().prepare(
                "SELECT file_id, sealed FROM ml_data WHERE clip_ok = 1 AND file_id > ?1 \
                 ORDER BY file_id LIMIT ?2",
            )?;
            let chunk = q
                .query_map(params![after, CHUNK], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            (st.opener(), chunk)
        };
        let Some(last) = chunk.last().map(|r| r.0) else {
            break;
        };
        after = last;
        for (id, blob) in &chunk {
            let Some(e) = opener
                .open_sealed::<MlPart>(blob)
                .ok()
                .and_then(|p| p.clip)
                .map(|c| c.embedding)
                .filter(|e| !e.is_empty())
            else {
                continue;
            };
            m.get_or_insert_with(|| ClipMatrix::with_capacity(rows.max(0) as usize, e.len()))
                .push(*id, &e);
        }
        if (chunk.len() as i64) < CHUNK {
            break;
        }
    }
    Ok(m.unwrap_or_default())
}

/// Exact scores from the stored `mldata`.
pub(crate) struct StoreRescorer<'a>(pub &'a Inner);

impl Rescorer for StoreRescorer<'_> {
    fn rescore(&mut self, file_ids: &[i64], t: &[f32]) -> HashMap<i64, f32> {
        let mut out = HashMap::with_capacity(file_ids.len());
        for ids in file_ids.chunks(CHUNK as usize) {
            let (opener, blobs) = {
                let st = self.0.store();
                let Ok(mut q) = st
                    .conn()
                    .prepare_cached("SELECT sealed FROM ml_data WHERE file_id = ?1")
                else {
                    return out;
                };
                let blobs: Vec<(i64, Vec<u8>)> = ids
                    .iter()
                    .filter_map(|id| {
                        q.query_row(params![id], |r| r.get::<_, Vec<u8>>(0))
                            .ok()
                            .map(|b| (*id, b))
                    })
                    .collect();
                (st.opener(), blobs)
            };
            for (id, blob) in blobs {
                if let Some(c) = opener
                    .open_sealed::<MlPart>(&blob)
                    .ok()
                    .and_then(|p| p.clip)
                    && c.embedding.len() == t.len()
                {
                    out.insert(id, super::clip::dot(&c.embedding, t));
                }
            }
        }
        out
    }
}
