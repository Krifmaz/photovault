//! Search: magic search (MobileCLIP text embedding vs the files' image
//! embeddings, Ente's thresholds) and the metadata search of Ente's apps
//! (file names, captions, albums, dates, people, file types, places).
//!
//! Behaviour follows Ente's mobile app (`services/search_service.dart`,
//! `ui/viewer/search/search_widget.dart` and `search_suggestions.dart` for
//! what matches and how sections are ordered,
//! `services/machine_learning/semantic_search/semantic_search_service.dart`
//! and `services/magic_cache_service.dart` for magic search and the
//! discovery prompts) and the web app's search worker
//! (`web/packages/new/photos/services/search/worker.ts`). Holidays and the
//! world-cities list are left out (no data source but Ente's).
//!
//! Everything is computed in memory from the library index, the sealed
//! store and the stored `mldata`; nothing is written. Results only contain
//! files the user can see: files that are only in hidden collections are
//! left out, like the apps do.
//!
//! Internal API: [`invalidate`] (after `mldata` changes) and
//! [`clip_matches`] (memories, which use Ente's precomputed theme
//! embeddings, so no text model is needed there).
//!
//! Based on Ente (ente.com).

mod clip;
mod data;
mod dates;
mod places;
#[cfg(test)]
mod tests;

use super::{Person, SearchKind, SearchSuggestion};
use crate::client::Inner;
use crate::library::Index;
use crate::{Core, CoreError, FileKind, TimelineItem};
use chrono::{Datelike, Local, NaiveDate};
use clip::{ClipMatrix, MIN_SIMILARITY, PROMPTS};
use data::{Extras, SearchData, StoreRescorer};
use dates::DateKey;
use places::LocationTag;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

/// Items per discovery section (`kSearchSectionLimit`).
const SECTION_LIMIT: usize = 9;
/// Smallest unnamed face cluster shown in search
/// (`kMinimumClusterSizeSearchResult`).
const MIN_CLUSTER_SIZE: u32 = 10;
/// Albums suggested for a query at most (`_maximumResultsLimit`).
const MAX_ALBUM_RESULTS: usize = 20;
/// Libraries up to this many files get their visible-file list rebuilt on
/// the caller's thread when the index changed (a millisecond or two per
/// 10k files); larger ones in the background.
const SYNC_REBUILD_MAX: usize = 20_000;
/// Query embeddings kept (`_queryEmbeddingCache = LRUMap(20)`).
#[cfg_attr(not(feature = "ml"), allow(dead_code))]
const TEXT_CACHE: usize = 20;

/// Search caches (visible files, captions and places, CLIP matrix, query
/// embeddings, discovery prompts), rebuilt lazily after [`invalidate`] or
/// when the library index changes.
#[derive(Default)]
pub(crate) struct SearchCache {
    state: Mutex<CacheState>,
    /// Bumped by [`invalidate`]; the CLIP caches are current while equal.
    generation: AtomicU64,
    /// Serialises the expensive builds.
    extras_build: Mutex<()>,
    clip_build: Mutex<()>,
}

impl std::fmt::Debug for SearchCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchCache").finish_non_exhaustive()
    }
}

/// Prompt index → matching file ids, best first.
type PromptMatches = Vec<(usize, Vec<i64>)>;

#[derive(Default)]
struct CacheState {
    data: Option<Arc<SearchData>>,
    data_running: bool,
    extras: Option<Arc<Extras>>,
    /// The index `extras` was built for.
    extras_for: Weak<Index>,
    extras_running: bool,
    clip: Option<(u64, Arc<ClipMatrix>)>,
    prompts: Option<(u64, Arc<PromptMatches>)>,
    prompts_running: bool,
    /// Recent query embeddings, oldest first (only filled with feature `ml`).
    #[cfg_attr(not(feature = "ml"), allow(dead_code))]
    texts: VecDeque<(String, Arc<Vec<f32>>)>,
}

fn cache(inner: &Inner) -> &SearchCache {
    &inner.ml.search
}

fn state(inner: &Inner) -> MutexGuard<'_, CacheState> {
    cache(inner).state.lock().unwrap_or_else(|e| e.into_inner())
}

fn lock(m: &Mutex<()>) -> MutexGuard<'_, ()> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Drop the caches (library or `mldata` changed).
pub(crate) fn invalidate(inner: &Inner) {
    cache(inner).generation.fetch_add(1, Ordering::SeqCst);
    let mut s = state(inner);
    s.clip = None;
    s.prompts = None;
    // Captions and places are rebuilt on next use; the old ones serve
    // until then.
    s.extras_for = Weak::new();
}

/// Files whose CLIP image embedding scores at least `threshold` against a
/// text embedding, best first: `(file_id, score)`. Includes files the user
/// cannot see (callers filter). Blocking: builds the CLIP matrix on first
/// use after [`invalidate`]; call it off the UI thread.
pub(crate) fn clip_matches(
    inner: &Inner,
    text_embedding: &[f32],
    threshold: f32,
) -> Vec<(i64, f32)> {
    clip_matrix(inner).matches(text_embedding, threshold, &mut StoreRescorer(inner))
}

/// The CLIP text embedding of `text` (normalised, like the image ones), or
/// `None` when the text model is not downloaded or cannot run (or this
/// build has no cargo feature `ml`). Blocking (tens of ms on first use).
#[cfg(feature = "ml")]
fn text_embedding_arc(inner: &Inner, text: &str) -> Option<Arc<Vec<f32>>> {
    let (model, vocab) = clip::text_model_paths(&inner.ml.models_dir)?;
    {
        let mut s = state(inner);
        if let Some(i) = s.texts.iter().position(|(t, _)| t == text) {
            let hit = s.texts.remove(i)?;
            let e = hit.1.clone();
            s.texts.push_back(hit);
            return Some(e);
        }
    }
    let r = ente_ml::indexing::run_clip_text(ente_ml::indexing::RunClipTextRequest {
        text: text.to_string(),
        model_path: model.display().to_string(),
        vocab_path: vocab.display().to_string(),
    })
    .ok()?;
    if r.embedding.is_empty() {
        return None;
    }
    let e = Arc::new(r.embedding);
    let mut s = state(inner);
    s.texts.push_back((text.to_string(), e.clone()));
    while s.texts.len() > TEXT_CACHE {
        s.texts.pop_front();
    }
    Some(e)
}

#[cfg(not(feature = "ml"))]
fn text_embedding_arc(inner: &Inner, text: &str) -> Option<Arc<Vec<f32>>> {
    let _ = (inner, text);
    None
}

/// Whether magic search can return anything: the text model is here and
/// some file has a current CLIP index. Fast.
fn magic_available(inner: &Inner) -> bool {
    if !cfg!(feature = "ml") || clip::text_model_paths(&inner.ml.models_dir).is_none() {
        return false;
    }
    let generation = cache(inner).generation.load(Ordering::SeqCst);
    if let Some((g, m)) = &state(inner).clip
        && *g == generation
    {
        return !m.is_empty();
    }
    inner
        .store()
        .conn()
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM ml_data WHERE clip_ok = 1)",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap_or(false)
}

// ---------------------------------------------------------------- caches

/// The CLIP matrix, built now if missing or stale.
fn clip_matrix(inner: &Inner) -> Arc<ClipMatrix> {
    let c = cache(inner);
    let current = |s: &CacheState, g: u64| {
        s.clip
            .as_ref()
            .filter(|(sg, _)| *sg == g)
            .map(|(_, m)| m.clone())
    };
    let g = c.generation.load(Ordering::SeqCst);
    if let Some(m) = current(&state(inner), g) {
        return m;
    }
    let _b = lock(&c.clip_build);
    let g = c.generation.load(Ordering::SeqCst);
    if let Some(m) = current(&state(inner), g) {
        return m;
    }
    let m = Arc::new(data::build_clip(inner).unwrap_or_default());
    state(inner).clip = Some((g, m.clone()));
    m
}

/// The visible files. Rebuilt now when the index changed, except that with
/// `bg` a large library gets the stale copy while a fresh one is built in
/// the background.
fn search_data(inner: &Inner, bg: Option<&Arc<Inner>>) -> Arc<SearchData> {
    let idx = inner.index();
    {
        let mut s = state(inner);
        if let Some(d) = &s.data {
            if Arc::ptr_eq(&d.index, &idx) {
                return d.clone();
            }
            if let Some(bg) = bg.filter(|_| idx.all_items().len() > SYNC_REBUILD_MAX) {
                let d = d.clone();
                if !s.data_running {
                    s.data_running = true;
                    drop(s);
                    spawn_refresh(bg);
                }
                return d;
            }
        }
    }
    let d = Arc::new(SearchData::build(idx));
    let mut s = state(inner);
    let newer = s
        .data
        .as_ref()
        .is_none_or(|old| !Arc::ptr_eq(&old.index, &inner.index()));
    if newer {
        s.data = Some(d.clone());
    }
    d
}

/// Captions and locations. With `bg`, whatever is there (possibly stale or
/// none) while a fresh copy is built in the background; without, built
/// now if needed.
fn extras(inner: &Inner, bg: Option<&Arc<Inner>>) -> Option<Arc<Extras>> {
    let fresh = |s: &CacheState, idx: &Arc<Index>| {
        s.extras
            .as_ref()
            .filter(|_| Weak::ptr_eq(&s.extras_for, &Arc::downgrade(idx)))
            .cloned()
    };
    let idx = inner.index();
    {
        let mut s = state(inner);
        if let Some(e) = fresh(&s, &idx) {
            return Some(e);
        }
        if let Some(bg) = bg {
            let e = s.extras.clone();
            if !s.extras_running {
                s.extras_running = true;
                drop(s);
                spawn_refresh(bg);
            }
            return e;
        }
    }
    let _b = lock(&cache(inner).extras_build);
    let idx = inner.index();
    if let Some(e) = fresh(&state(inner), &idx) {
        return Some(e);
    }
    match Extras::build(inner) {
        Ok(e) => {
            let e = Arc::new(e);
            let mut s = state(inner);
            s.extras = Some(e.clone());
            s.extras_for = Arc::downgrade(&idx);
            Some(e)
        }
        Err(_) => state(inner).extras.clone(),
    }
}

/// Rebuild the visible files and the extras in the background.
fn spawn_refresh(inner: &Arc<Inner>) {
    let inner = inner.clone();
    crate::runtime().spawn_blocking(move || {
        let _ = search_data(&inner, None);
        state(&inner).data_running = false;
        let _ = extras(&inner, None);
        state(&inner).extras_running = false;
    });
}

/// The discovery prompts' matches, if computed for the current `mldata`;
/// otherwise starts computing them in the background.
fn prompt_matches(inner: &Inner, bg: Option<&Arc<Inner>>) -> Option<Arc<PromptMatches>> {
    let g = cache(inner).generation.load(Ordering::SeqCst);
    let mut s = state(inner);
    if let Some((pg, p)) = &s.prompts
        && *pg == g
    {
        return Some(p.clone());
    }
    if let Some(bg) = bg
        && !s.prompts_running
    {
        s.prompts_running = true;
        drop(s);
        if !magic_available(inner) {
            state(inner).prompts_running = false;
            return None;
        }
        let inner = bg.clone();
        crate::runtime().spawn_blocking(move || {
            let p = compute_prompts(&inner);
            let mut s = state(&inner);
            s.prompts_running = false;
            if let Some(p) = p {
                s.prompts = Some((g, Arc::new(p)));
            }
        });
    }
    None
}

fn compute_prompts(inner: &Inner) -> Option<PromptMatches> {
    let mut out = Vec::new();
    for (i, p) in PROMPTS.iter().enumerate() {
        let t = text_embedding_arc(inner, p.query)?;
        let ids = clip_matches(inner, &t, p.min_score)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        out.push((i, ids));
    }
    Some(out)
}

fn location_tags(inner: &Inner) -> Vec<LocationTag> {
    let rows = inner
        .store()
        .entities(super::entity::LOCATION)
        .unwrap_or_default();
    let mut tags: Vec<LocationTag> = rows
        .iter()
        .filter_map(|(id, _, data)| LocationTag::from_entity(id, data))
        .collect();
    tags.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    tags
}

// ---------------------------------------------------------------- matching

/// The file types search knows (`SearchKind::FileType` keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeKey {
    Image,
    Video,
    Live,
    Screenshot,
    Raw,
}

/// `(key, kind, label, words the query may start)`. The first three follow
/// the mobile app's `getHumanReadableString`.
const TYPES: &[(&str, TypeKey, &str, &[&str])] = &[
    ("image", TypeKey::Image, "Photos", &["photos", "images"]),
    ("video", TypeKey::Video, "Videos", &["videos"]),
    ("live", TypeKey::Live, "Live Photos", &["live photos"]),
    (
        "screenshot",
        TypeKey::Screenshot,
        "Screenshots",
        &["screenshots"],
    ),
    ("raw", TypeKey::Raw, "RAW", &["raw"]),
];

/// Ente's RAW extensions (mobile `utils/image_util.dart`).
const RAW_EXTENSIONS: &[&str] = &[
    "arw", "cr2", "cr3", "nef", "nrw", "dng", "orf", "raf", "rw2", "pef", "srw", "3fr", "fff",
    "rwl", "x3f", "iiq", "kdc", "dcr", "mrw", "erf", "mef", "raw",
];

/// The extension of a (lowercase) file name, without the dot.
fn extension(name: &str) -> Option<&str> {
    let (_, e) = name.rsplit_once('.')?;
    (!e.is_empty()).then_some(e)
}

fn is_type(t: TypeKey, item: &TimelineItem, lower_name: &str) -> bool {
    match t {
        TypeKey::Image => item.kind == FileKind::Image,
        TypeKey::Video => item.kind == FileKind::Video,
        TypeKey::Live => item.kind == FileKind::LivePhoto,
        TypeKey::Screenshot => {
            item.kind != FileKind::Video
                && ["screenshot", "screen shot", "screen_shot"]
                    .iter()
                    .any(|w| lower_name.contains(w))
        }
        TypeKey::Raw => extension(lower_name).is_some_and(|e| RAW_EXTENSIONS.contains(&e)),
    }
}

/// Whether a file (item, lowercase name) is of a type.
type TypeMatcher = Box<dyn Fn(&TimelineItem, &str) -> bool>;

/// A file-type key: one of [`TYPES`] or `ext:<EXT>`.
fn type_matcher(key: &str) -> Option<TypeMatcher> {
    if let Some(ext) = key.strip_prefix("ext:") {
        let ext = ext.to_lowercase();
        return (!ext.is_empty()).then(|| {
            Box::new(move |_: &TimelineItem, n: &str| extension(n) == Some(ext.as_str()))
                as TypeMatcher
        });
    }
    let t = TYPES.iter().find(|t| t.0 == key)?.1;
    Some(Box::new(move |i: &TimelineItem, n: &str| is_type(t, i, n)))
}

/// Whether `needle` starts a word of `hay` (the web worker's
/// `(^|[\s.,!?"'_-])needle`).
fn word_start(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| {
        hay[..i]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || ".,!?\"'_-".contains(c))
    })
}

/// Mobile `_looksLikeFileQuery` (`search_suggestions.dart`).
fn looks_like_file(lower: &str) -> bool {
    if lower.is_empty() {
        return false;
    }
    const PREFIXES: &[&str] = &[
        ".",
        "img_",
        "img-",
        "pxl_",
        "screenshot_",
        "screenshot-",
        "vid_",
        "vid-",
    ];
    const COMMON: &[&str] = &[
        "3fr", "arw", "avi", "avif", "bmp", "cr2", "cr3", "dcr", "dng", "erf", "fff", "gif",
        "heic", "heif", "iiq", "jpeg", "jpg", "kdc", "m4v", "mef", "mkv", "mov", "mp4", "mrw",
        "nef", "nrw", "orf", "pef", "png", "raf", "raw", "rwl", "rw2", "srw", "tif", "tiff",
        "webp", "x3f",
    ];
    if PREFIXES.iter().any(|p| lower.starts_with(p)) || camera_prefix(lower) {
        return true;
    }
    if COMMON.contains(&lower) {
        return true;
    }
    if lower.chars().count() < 3 {
        return false;
    }
    ["live", "photo", "video", "photos", "videos", "live photos"].contains(&lower)
}

/// `^(?:dsc(?:[_\d]|n|f)|\d{8}_)`.
fn camera_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    if let Some(rest) = s.strip_prefix("dsc") {
        let c = rest.bytes().next();
        return c.is_some_and(|c| c == b'_' || c.is_ascii_digit() || c == b'n' || c == b'f');
    }
    b.len() > 8 && b[..8].iter().all(u8::is_ascii_digit) && b[8] == b'_'
}

/// The search screen's sections, in the mobile app's default order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Magic,
    People,
    Places,
    Moments,
    Albums,
    Files,
}

const DEFAULT_ORDER: [Section; 6] = [
    Section::Magic,
    Section::People,
    Section::Places,
    Section::Moments,
    Section::Albums,
    Section::Files,
];

/// `_sectionOrderForQuery`: file-like queries put files first, date-like
/// ones moments first.
fn section_order(lower: &str, today: NaiveDate) -> Vec<Section> {
    let promote = if looks_like_file(lower) {
        Some(Section::Files)
    } else if dates::looks_like_moment(lower, today) {
        Some(Section::Moments)
    } else {
        None
    };
    match promote {
        Some(p) => std::iter::once(p)
            .chain(DEFAULT_ORDER.into_iter().filter(|s| *s != p))
            .collect(),
        None => DEFAULT_ORDER.to_vec(),
    }
}

fn suggestion(
    kind: SearchKind,
    key: impl Into<String>,
    label: impl Into<String>,
    count: u64,
    cover_file_id: Option<i64>,
) -> SearchSuggestion {
    SearchSuggestion {
        kind,
        key: key.into(),
        label: label.into(),
        count,
        cover_file_id,
        cover_face_id: None,
    }
}

/// Count and newest of a set of visible positions.
fn count_and_cover(d: &SearchData, positions: impl Iterator<Item = u32>) -> (u64, Option<i64>) {
    let mut n = 0u64;
    let mut newest: Option<u32> = None;
    for p in positions {
        n += 1;
        newest = Some(newest.map_or(p, |q| q.min(p)));
    }
    (n, newest.map(|p| d.item(p).id))
}

/// A person suggestion, if any of their files is visible.
fn person_suggestion(d: &SearchData, p: &Person, file_ids: &[i64]) -> Option<SearchSuggestion> {
    let (count, newest) =
        count_and_cover(d, file_ids.iter().filter_map(|id| d.by_id.get(id).copied()));
    if count == 0 {
        return None;
    }
    // The avatar face only if its file is visible.
    let cover = p
        .cover
        .as_ref()
        .filter(|c| d.by_id.contains_key(&c.file_id));
    Some(SearchSuggestion {
        kind: SearchKind::Person,
        key: p.id.clone(),
        label: p.name.clone().unwrap_or_default(),
        count,
        cover_file_id: cover.map(|c| c.file_id).or(newest),
        cover_face_id: cover.map(|c| c.face_id.clone()),
    })
}

/// Named people matching the query: word starts first, then most files.
fn people_matching(
    d: &SearchData,
    people: &[Person],
    lower: &str,
    file_ids: &dyn Fn(&str) -> Vec<i64>,
) -> Vec<SearchSuggestion> {
    let mut out: Vec<(bool, SearchSuggestion)> = people
        .iter()
        .filter_map(|p| {
            let name = p.name.as_deref()?.to_lowercase();
            if p.is_hidden || !name.contains(lower) {
                return None;
            }
            let s = person_suggestion(d, p, &file_ids(&p.id))?;
            Some((word_start(&name, lower), s))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.count.cmp(&a.1.count)));
    out.into_iter().map(|x| x.1).collect()
}

/// Visible positions inside a tag.
fn positions_in_tag<'a>(
    d: &'a SearchData,
    e: &'a Extras,
    t: &'a LocationTag,
) -> impl Iterator<Item = u32> + 'a {
    e.locations
        .iter()
        .filter(|(_, lat, long)| t.contains(*lat, *long))
        .filter_map(|(id, _, _)| d.by_id.get(id).copied())
}

fn place_suggestion(d: &SearchData, e: Option<&Extras>, t: &LocationTag) -> SearchSuggestion {
    let (count, cover) = e.map_or((0, None), |e| count_and_cover(d, positions_in_tag(d, e, t)));
    suggestion(
        SearchKind::Place,
        t.id.clone(),
        t.name.clone(),
        count,
        cover,
    )
}

fn date_positions(d: &SearchData, k: DateKey, today: NaiveDate) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for (a, b) in k.ranges(&Local, today.year()) {
        out.extend(d.visible[d.taken_within(a, b)].iter().copied());
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Visible positions whose name matches a file-name query (substring, or
/// the file id like the web app).
fn name_positions<'a>(d: &'a SearchData, lower: &'a str) -> impl Iterator<Item = u32> + 'a {
    let id: Option<i64> = lower.parse().ok();
    d.visible
        .iter()
        .zip(&d.names)
        .filter(move |(p, n)| n.contains(lower) || id.is_some_and(|id| d.item(**p).id == id))
        .map(|(p, _)| *p)
}

fn caption_positions<'a>(
    d: &'a SearchData,
    e: &'a Extras,
    lower: &'a str,
) -> impl Iterator<Item = u32> + 'a {
    e.captions
        .iter()
        .filter(move |(_, c)| c.contains(lower))
        .filter_map(|(id, _)| d.by_id.get(id).copied())
}

fn type_positions<'a>(
    d: &'a SearchData,
    m: &'a dyn Fn(&TimelineItem, &str) -> bool,
) -> impl Iterator<Item = u32> + 'a {
    d.visible
        .iter()
        .zip(&d.names)
        .filter(move |(p, n)| m(d.item(**p), n))
        .map(|(p, _)| *p)
}

// ---------------------------------------------------------------- queries

/// Inputs of a suggestion run that do not come from the cache.
struct Sources<'a> {
    people: &'a [Person],
    person_files: &'a dyn Fn(&str) -> Vec<i64>,
    tags: &'a [LocationTag],
    magic: bool,
    today: NaiveDate,
}

fn suggestions_for(
    d: &SearchData,
    e: Option<&Extras>,
    src: &Sources<'_>,
    query: &str,
    limit: u32,
) -> Vec<SearchSuggestion> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let lower = q.to_lowercase();
    let mut out = Vec::new();
    for section in section_order(&lower, src.today) {
        match section {
            Section::Magic => {
                if src.magic {
                    out.push(suggestion(SearchKind::Magic, q, q, 0, None));
                }
            }
            Section::People => {
                out.extend(people_matching(d, src.people, &lower, src.person_files));
            }
            Section::Places => {
                let mut v: Vec<(bool, SearchSuggestion)> = src
                    .tags
                    .iter()
                    .filter(|t| t.name.to_lowercase().contains(&lower))
                    .map(|t| {
                        (
                            word_start(&t.name.to_lowercase(), &lower),
                            place_suggestion(d, e, t),
                        )
                    })
                    // Without the extras the count is not known yet (0).
                    .filter(|(_, s)| e.is_none() || s.count > 0)
                    .collect();
                v.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.count.cmp(&a.1.count)));
                out.extend(v.into_iter().map(|x| x.1));
            }
            Section::Moments => {
                for k in dates::query_keys(q, src.today) {
                    let pos = date_positions(d, k, src.today);
                    if let Some(&newest) = pos.first() {
                        out.push(suggestion(
                            SearchKind::Date,
                            k.key(),
                            k.label(),
                            pos.len() as u64,
                            Some(d.item(newest).id),
                        ));
                    }
                }
            }
            Section::Albums => {
                let mut v: Vec<(bool, SearchSuggestion)> = d
                    .index
                    .visible_collections()
                    .iter()
                    .filter(|c| c.kind != "uncategorized")
                    .filter_map(|c| {
                        let name = c.name.to_lowercase();
                        name.contains(&lower).then(|| {
                            (
                                word_start(&name, &lower),
                                suggestion(
                                    SearchKind::Album,
                                    c.id.to_string(),
                                    c.name.clone(),
                                    c.count,
                                    c.cover_file_id,
                                ),
                            )
                        })
                    })
                    .take(MAX_ALBUM_RESULTS)
                    .collect();
                v.sort_by_key(|x| std::cmp::Reverse(x.0));
                out.extend(v.into_iter().map(|x| x.1));
            }
            Section::Files => {
                for (key, t, label, words) in TYPES {
                    if !words.iter().any(|w| w.starts_with(lower.as_str())) {
                        continue;
                    }
                    let m = move |i: &TimelineItem, n: &str| is_type(*t, i, n);
                    let (count, cover) = count_and_cover(d, type_positions(d, &m));
                    if count > 0 {
                        out.push(suggestion(SearchKind::FileType, *key, *label, count, cover));
                    }
                }
                if let Some(e) = e {
                    let (count, cover) = count_and_cover(d, caption_positions(d, e, &lower));
                    if count > 0 {
                        out.push(suggestion(SearchKind::Caption, q, q, count, cover));
                    }
                }
                let (count, cover) = count_and_cover(d, name_positions(d, &lower));
                if count > 0 {
                    out.push(suggestion(SearchKind::FileName, q, q, count, cover));
                }
                if lower.starts_with('.') {
                    out.extend(extension_suggestions(d, Some(&lower), false));
                }
            }
        }
    }
    if limit > 0 {
        out.truncate(limit as usize);
    }
    out
}

/// Extension suggestions (mobile `getFileExtensionResults`): with a
/// query, extensions of files whose name contains it; most files first.
/// `plural` labels them like the discovery section ("JPGs").
fn extension_suggestions(
    d: &SearchData,
    query: Option<&str>,
    plural: bool,
) -> Vec<SearchSuggestion> {
    let mut by_ext: std::collections::HashMap<&str, (u64, u32)> = Default::default();
    for (p, n) in d.visible.iter().zip(&d.names) {
        if query.is_some_and(|q| !n.contains(q)) {
            continue;
        }
        if let Some(e) = extension(n) {
            let v = by_ext.entry(e).or_insert((0, *p));
            v.0 += 1;
            v.1 = v.1.min(*p);
        }
    }
    let mut v: Vec<(&str, (u64, u32))> = by_ext.into_iter().collect();
    v.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    v.into_iter()
        .map(|(e, (count, newest))| {
            let up = e.to_uppercase();
            let label = if plural { format!("{up}s") } else { up.clone() };
            suggestion(
                SearchKind::FileType,
                format!("ext:{up}"),
                label,
                count,
                Some(d.item(newest).id),
            )
        })
        .collect()
}

/// The discovery sections: people (named first), places, file types,
/// recent albums, magic prompts.
fn discovery_for(
    d: &SearchData,
    e: Option<&Extras>,
    src: &Sources<'_>,
    prompts: Option<&PromptMatches>,
) -> Vec<SearchSuggestion> {
    let mut out = Vec::new();
    out.extend(
        src.people
            .iter()
            .filter(|p| !p.is_hidden)
            .filter_map(|p| person_suggestion(d, p, &(src.person_files)(&p.id)))
            .take(SECTION_LIMIT),
    );
    if let Some(e) = e {
        let mut places: Vec<SearchSuggestion> = src
            .tags
            .iter()
            .map(|t| place_suggestion(d, Some(e), t))
            .filter(|s| s.count > 0)
            .collect();
        places.sort_by_key(|p| std::cmp::Reverse(p.count));
        out.extend(places.into_iter().take(SECTION_LIMIT));
    }
    let mut types: Vec<SearchSuggestion> = TYPES
        .iter()
        .filter_map(|(key, t, label, _)| {
            let m = move |i: &TimelineItem, n: &str| is_type(*t, i, n);
            let (count, cover) = count_and_cover(d, type_positions(d, &m));
            (count > 0).then(|| suggestion(SearchKind::FileType, *key, *label, count, cover))
        })
        .collect();
    types.extend(extension_suggestions(d, None, true));
    out.extend(types.into_iter().take(SECTION_LIMIT));
    out.extend(
        d.index
            .visible_collections()
            .iter()
            .filter(|c| c.kind != "uncategorized" && c.count > 0)
            .take(SECTION_LIMIT)
            .map(|c| {
                suggestion(
                    SearchKind::Album,
                    c.id.to_string(),
                    c.name.clone(),
                    c.count,
                    c.cover_file_id,
                )
            }),
    );
    for (i, ids) in prompts.into_iter().flatten() {
        let p = &PROMPTS[*i];
        let items = prompt_items(d, p, ids);
        if let Some(first) = items.first() {
            out.push(suggestion(
                SearchKind::Magic,
                p.query,
                p.title,
                items.len() as u64,
                Some(first.id),
            ));
        }
    }
    out
}

/// A prompt's visible matches in its order (`toGenericSearchResult`).
fn prompt_items(d: &SearchData, p: &clip::Prompt, ids: &[i64]) -> Vec<TimelineItem> {
    let mut items: Vec<(u32, &TimelineItem)> = ids
        .iter()
        .filter_map(|id| d.by_id.get(id).map(|&pos| (pos, d.item(pos))))
        .filter(|(_, it)| p.show_video || it.kind != FileKind::Video)
        .collect();
    if p.recent_first {
        items.sort_by_key(|(pos, _)| *pos);
    }
    items.into_iter().map(|(_, it)| it.clone()).collect()
}

/// Magic search, blocking. With `as_prompt`, a discovery prompt's query
/// uses the prompt's threshold, video rule and order.
fn magic(inner: &Inner, query: &str, limit: u32, as_prompt: bool) -> Vec<TimelineItem> {
    let (threshold, q) = clip::split_threshold(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    let prompt = clip::prompt_for(q).filter(|_| as_prompt);
    let threshold = threshold
        .or(prompt.map(|p| p.min_score))
        .unwrap_or(MIN_SIMILARITY);
    let Some(t) = text_embedding_arc(inner, q) else {
        return Vec::new();
    };
    let matches = clip_matches(inner, &t, threshold);
    let d = search_data(inner, None);
    let mut items = match prompt {
        Some(p) => {
            let ids: Vec<i64> = matches.iter().map(|m| m.0).collect();
            prompt_items(&d, p, &ids)
        }
        None => matches
            .iter()
            .filter_map(|(id, _)| d.by_id.get(id).map(|&p| d.item(p).clone()))
            .collect(),
    };
    if limit > 0 {
        items.truncate(limit as usize);
    }
    items
}

/// The files of a suggestion, blocking.
fn results(inner: &Inner, kind: SearchKind, key: &str) -> Vec<TimelineItem> {
    if kind == SearchKind::Magic {
        return magic(inner, key, 0, true);
    }
    let d = search_data(inner, None);
    let at = |pos: Vec<u32>| -> Vec<TimelineItem> {
        pos.into_iter().map(|p| d.item(p).clone()).collect()
    };
    let sorted = |it: &mut dyn Iterator<Item = u32>| -> Vec<u32> {
        let mut v: Vec<u32> = it.collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    match kind {
        SearchKind::Magic => Vec::new(),
        SearchKind::Person => d.items_of(super::faces::person_file_ids(inner, key)),
        SearchKind::Album => {
            let Ok(id) = key.parse::<i64>() else {
                return Vec::new();
            };
            if !d.index.visible_collections().iter().any(|c| c.id == id) {
                return Vec::new();
            }
            d.index
                .collection_items(id)
                .filter(|it| d.by_id.contains_key(&it.id))
                .map(|it| {
                    let mut it = it.clone();
                    it.collection_id = id;
                    it
                })
                .collect()
        }
        SearchKind::Date => {
            let today = Local::now().date_naive();
            DateKey::parse(key).map_or_else(Vec::new, |k| at(date_positions(&d, k, today)))
        }
        SearchKind::Place => {
            let Some(t) = location_tags(inner).into_iter().find(|t| t.id == key) else {
                return Vec::new();
            };
            let Some(e) = extras(inner, None) else {
                return Vec::new();
            };
            at(sorted(&mut positions_in_tag(&d, &e, &t)))
        }
        SearchKind::FileName => {
            let lower = key.trim().to_lowercase();
            if lower.is_empty() {
                return Vec::new();
            }
            at(name_positions(&d, &lower).collect())
        }
        SearchKind::Caption => {
            let lower = key.trim().to_lowercase();
            let Some(e) = extras(inner, None).filter(|_| !lower.is_empty()) else {
                return Vec::new();
            };
            at(sorted(&mut caption_positions(&d, &e, &lower)))
        }
        SearchKind::FileType => match type_matcher(key) {
            Some(m) => at(type_positions(&d, m.as_ref()).collect()),
            None => Vec::new(),
        },
    }
}

fn sources_people(inner: &Inner) -> Vec<Person> {
    super::faces::people(inner, false, MIN_CLUSTER_SIZE)
}

/// Run blocking search work on the runtime's blocking pool.
async fn blocking<T: Send + 'static>(
    inner: &Arc<Inner>,
    f: impl FnOnce(&Inner) -> T + Send + 'static,
) -> Result<T, CoreError> {
    let inner = inner.clone();
    crate::run(async move {
        tokio::task::spawn_blocking(move || f(&inner))
            .await
            .map_err(|e| CoreError::storage(format!("search: {e}")))
    })
    .await
}

#[uniffi::export]
impl Core {
    /// Suggestions for what the user typed, best first (fast, in memory).
    pub fn search_suggestions(&self, query: String, limit: u32) -> Vec<SearchSuggestion> {
        let inner = &self.inner;
        if query.trim().is_empty() {
            return Vec::new();
        }
        let d = search_data(inner, Some(inner));
        let e = extras(inner, Some(inner));
        let people = sources_people(inner);
        let person_files = |id: &str| super::faces::person_file_ids(inner, id);
        let tags = location_tags(inner);
        let src = Sources {
            people: &people,
            person_files: &person_files,
            tags: &tags,
            magic: magic_available(inner),
            today: Local::now().date_naive(),
        };
        suggestions_for(&d, e.as_deref(), &src, &query, limit)
    }

    /// What the search screen shows before typing: people, places, file
    /// types, recent albums and magic-search prompts.
    pub fn search_discovery(&self) -> Vec<SearchSuggestion> {
        let inner = &self.inner;
        let d = search_data(inner, Some(inner));
        let e = extras(inner, Some(inner));
        let people = sources_people(inner);
        let person_files = |id: &str| super::faces::person_file_ids(inner, id);
        let tags = location_tags(inner);
        let prompts = prompt_matches(inner, Some(inner));
        let src = Sources {
            people: &people,
            person_files: &person_files,
            tags: &tags,
            magic: false,
            today: Local::now().date_naive(),
        };
        discovery_for(&d, e.as_deref(), &src, prompts.as_deref())
    }

    /// The files of a suggestion, newest first (magic: best match first).
    pub async fn search_results(
        &self,
        kind: SearchKind,
        key: String,
    ) -> Result<Vec<TimelineItem>, CoreError> {
        blocking(&self.inner, move |inner| results(inner, kind, &key)).await
    }

    /// Magic search: files matching a description, best first. Empty when
    /// the text model is not available.
    pub async fn magic_search(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<TimelineItem>, CoreError> {
        blocking(&self.inner, move |inner| magic(inner, &query, limit, false)).await
    }
}
