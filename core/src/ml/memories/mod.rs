//! Memories: Ente's smart memories (on this day, people, trips, time,
//! CLIP themes, fillers), computed on device.
//!
//! - [`engine`]: Ente's memories engine (the Rust port Ente's photos app
//!   runs), unchanged in its rules and constants.
//! - [`source`]: its inputs from the core (visible files with capture
//!   dates and locations, people, CLIP and faces when ML is ready).
//! - [`cache`]: Ente's memories cache (memories for now and the next
//!   update, show windows, shown logs), sealed in kv `ml:memories_cache`.
//! - [`titles`]: Ente's English titles.
//!
//! # Behaviour of [`Core::memories`]
//!
//! Like Ente's `MemoriesCacheService`, memories are computed for "now" and
//! for "in three days" at most once every three days (or when the cache is
//! empty and the library changed since), and each memory shows during its
//! own window. The call itself never computes them:
//!
//! - The list for the caller's local time is cached in memory until the
//!   next local midnight or the next window boundary, whichever is first,
//!   so repeated calls are a clone. It is rebuilt (fast: id lookups in the
//!   library index) when the library index, the people or the seen state
//!   change.
//! - When an update is due, it runs on a background thread (it reads every
//!   file's location and, with ML, scores CLIP themes); meanwhile the call
//!   returns the memories of the previous update that still show, or, on
//!   the very first call, Ente's quick "simple memories" (on this day and
//!   fillers). Call again later (on resume, after a sync) for the result.
//!
//! Unseen memories come first; [`Core::mark_memory_seen`] persists the seen
//! state sealed in kv `ml:memories_seen`.
//!
//! Based on Ente (ente.com).

pub(crate) mod cache;
pub(crate) mod engine;
pub(crate) mod source;
pub(crate) mod titles;

#[cfg(test)]
mod tests;

use self::cache::{CACHE_KEY, MICROS_PER_DAY, StoredCache, StoredKind, VERSION};
use self::engine::{MemoriesRequest, Period};
use super::{Memory, MemoryKind};
use crate::client::Inner;
use crate::library::Index;
use crate::ml::store::kv;
use crate::{Core, CoreError, Result, TimelineItem};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// Seen memories are forgotten this long after they were seen (no memory
/// shows longer than a few weeks, and ids carry their show start).
const SEEN_RETENTION: i64 = 60 * MICROS_PER_DAY;
/// After a failed update, wait this long before trying again.
const RETRY_AFTER: i64 = 3_600_000_000;

/// Memories computed for a day, rebuilt lazily after [`invalidate`].
#[derive(Debug, Default)]
pub(crate) struct MemoriesCache {
    state: Mutex<State>,
    /// An update runs in the background.
    computing: AtomicBool,
}

#[derive(Debug, Default)]
struct State {
    /// Bumped by [`invalidate`] and by finished updates.
    generation: u64,
    /// The stored cache, once loaded.
    stored: Option<Option<Arc<StoredCache>>>,
    /// Seen memory ids → when, once loaded.
    seen: Option<BTreeMap<String, i64>>,
    /// The list handed out, and when it stays valid.
    day: Option<DayList>,
    /// The library index the last update read (an update is redone when
    /// the cache shows nothing and the library changed since).
    computed_with: Weak<Index>,
    /// When the last update failed.
    failed_at: Option<i64>,
}

#[derive(Debug)]
struct DayList {
    offset: i32,
    from: i64,
    until: i64,
    generation: u64,
    index: Weak<Index>,
    memories: Vec<Memory>,
}

impl MemoriesCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Drop the cache (library, people or `mldata` changed). Library index
/// rebuilds are noticed without it; `faces` calls it when people change.
pub(crate) fn invalidate(inner: &Inner) {
    let mut st = inner.ml.memories.lock();
    st.generation += 1;
    st.day = None;
}

/// Forget everything in memory (the stored state is re-read).
fn reset(inner: &Inner) {
    let mut st = inner.ml.memories.lock();
    *st = State {
        generation: st.generation + 1,
        ..State::default()
    };
}

fn same_index(weak: &Weak<Index>, index: &Arc<Index>) -> bool {
    std::ptr::eq(weak.as_ptr(), Arc::as_ptr(index))
}

/// The memories to show at `now_us` (see the module docs).
fn memories(inner: &Arc<Inner>, now_us: i64, utc_offset_secs: i32) -> Vec<Memory> {
    let Ok(session) = inner.session() else {
        reset(inner);
        return Vec::new();
    };
    let user_id = session.account.user_id;
    let index = inner.index();
    {
        let st = inner.ml.memories.lock();
        if let Some(d) = &st.day
            && d.offset == utc_offset_secs
            && d.generation == st.generation
            && (d.from..d.until).contains(&now_us)
            && same_index(&d.index, &index)
        {
            return d.memories.clone();
        }
    }
    // People come from `faces`, which may call `invalidate`: read them
    // without holding this module's lock.
    let names = person_names(inner);
    let me = me_person_id(inner, &names, &session.account.email);
    let mut st = inner.ml.memories.lock();
    if st.stored.is_none() {
        let loaded: Option<StoredCache> = inner.store().get_sealed(CACHE_KEY).ok().flatten();
        st.stored = Some(
            loaded
                .filter(|c| c.version == VERSION && c.user_id == user_id)
                .map(Arc::new),
        );
    }
    if st.seen.is_none() {
        st.seen = Some(load_seen(inner));
    }
    let stored = st.stored.clone().flatten();
    let seen = st.seen.clone().unwrap_or_default();
    let local_year = engine::local_date(now_us + i64::from(utc_offset_secs) * 1_000_000).year;

    let (list, until) = match &stored {
        Some(c) => (
            present(c, now_us, &index, &seen, &names, me.as_deref(), local_year),
            c.next_change(now_us),
        ),
        None => (Vec::new(), i64::MAX),
    };
    let library_changed = !same_index(&st.computed_with, &index);
    let due = match &stored {
        None => true,
        Some(c) => c.due(now_us) || (list.is_empty() && library_changed),
    };
    let may_retry = st
        .failed_at
        .is_none_or(|t| now_us - t >= RETRY_AFTER || now_us < t);
    if due && may_retry {
        start_update(inner, now_us, utc_offset_secs, user_id);
    }
    let list = if stored.is_none() && list.is_empty() {
        simple_memories(inner, now_us, utc_offset_secs, &seen)
    } else {
        list
    };
    let local_midnight = {
        let local = now_us + i64::from(utc_offset_secs) * 1_000_000;
        (local.div_euclid(MICROS_PER_DAY) + 1) * MICROS_PER_DAY
            - i64::from(utc_offset_secs) * 1_000_000
    };
    st.day = Some(DayList {
        offset: utc_offset_secs,
        from: now_us,
        // A running update bumps the generation when it finishes.
        until: until.min(local_midnight),
        generation: st.generation,
        index: Arc::downgrade(&index),
        memories: list.clone(),
    });
    list
}

/// Start a background update unless one runs.
fn start_update(inner: &Arc<Inner>, now_us: i64, utc_offset_secs: i32, user_id: i64) {
    let cache = &inner.ml.memories;
    if cache
        .computing
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let inner = inner.clone();
    crate::runtime().spawn_blocking(move || {
        let index = inner.index();
        let result = update(&inner, now_us, utc_offset_secs, user_id);
        let mut st = inner.ml.memories.lock();
        match result {
            Ok(Some(c)) => {
                st.stored = Some(Some(Arc::new(c)));
                st.computed_with = Arc::downgrade(&index);
                st.failed_at = None;
            }
            // Signed out or another account meanwhile: drop it.
            Ok(None) => st.stored = None,
            Err(_) => st.failed_at = Some(now_us),
        }
        st.generation += 1;
        st.day = None;
        drop(st);
        inner.ml.memories.computing.store(false, Ordering::Release);
    });
}

/// Ente's `updateCache`: process the old cache, compute now and next,
/// store the merged cache. `None` when the account changed meanwhile.
fn update(
    inner: &Inner,
    now_us: i64,
    utc_offset_secs: i32,
    user_id: i64,
) -> Result<Option<StoredCache>> {
    let old: Option<StoredCache> = inner.store().get_sealed(CACHE_KEY).ok().flatten();
    let processed = cache::process_old(
        old.as_ref().filter(|c| c.version == VERSION),
        now_us,
        user_id,
    );
    let email = inner.session()?.account.email.clone();
    let mut request = MemoriesRequest {
        files: source::memory_files(inner, utc_offset_secs)?,
        excluded_collection_ids: source::excluded_collection_ids(inner),
        seed: seed(now_us, user_id),
        ..MemoriesRequest::default()
    };
    cache::fill_request(&mut request, &processed);
    let next_us = now_us + cache::UPDATE_FREQUENCY;
    let periods = [
        Period {
            now: now_us,
            utc_offset_seconds: utc_offset_secs,
        },
        Period {
            now: next_us,
            utc_offset_seconds: utc_offset_secs,
        },
    ];
    let ml_ready = source::ml_ready(inner);
    let clip = source::InnerClip { inner };
    let ml = if ml_ready {
        source::add_people(&mut request, &source::people(inner), &email);
        Some(source::ml_inputs(inner, &clip, &request)?)
    } else {
        None
    };
    let mut result = engine::compute_smart_memories(&request, &periods, ml.as_ref(), None)?;
    drop(request);
    let next_memories = result.periods.pop().unwrap_or_default();
    let now_memories = result.periods.pop().unwrap_or_default();
    let merged = cache::merge(processed, now_us, next_us, now_memories, next_memories);
    if inner.session().map(|s| s.account.user_id).ok() != Some(user_id) {
        return Ok(None);
    }
    inner.store().put_sealed(CACHE_KEY, &merged)?;
    Ok(Some(merged))
}

/// Ente seeds each computation randomly; this mixes the time and account.
fn seed(now_us: i64, user_id: i64) -> u64 {
    (now_us as u64).rotate_left(17) ^ (user_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The cached memories that show at `now_us`, as records: items from the
/// library index (visible files only), unseen first.
fn present(
    cache: &StoredCache,
    now_us: i64,
    index: &Index,
    seen: &BTreeMap<String, i64>,
    names: &HashMap<String, Option<String>>,
    me: Option<&str>,
    local_year: i32,
) -> Vec<Memory> {
    let showing: Vec<&cache::StoredMemory> = cache.showing(now_us).collect();
    let wanted: HashSet<i64> = showing
        .iter()
        .flat_map(|m| m.file_ids.iter().copied())
        .collect();
    let items: HashMap<i64, &TimelineItem> = index
        .timeline_items()
        .filter(|i| wanted.contains(&i.id))
        .map(|i| (i.id, i))
        .collect();
    let mut out: Vec<Memory> = showing
        .into_iter()
        .filter_map(|m| {
            let items: Vec<TimelineItem> = m
                .file_ids
                .iter()
                .filter_map(|id| items.get(id).map(|&i| i.clone()))
                .collect();
            let cover = items.first()?.id;
            let person_id = match &m.kind {
                StoredKind::People { person_id, .. } => Some(person_id.clone()),
                _ => None,
            };
            let name = person_id
                .as_ref()
                .and_then(|p| names.get(p).cloned().flatten());
            let is_me = person_id.is_some() && person_id.as_deref() == me;
            Some(Memory {
                id: m.id.clone(),
                kind: kind_of(&m.kind),
                title: titles::title(&m.kind, name.as_deref(), is_me, local_year),
                subtitle: None,
                items,
                cover_file_id: cover,
                seen: seen.contains_key(&m.id),
                person_id,
            })
        })
        .collect();
    // Unseen first, otherwise in Ente's order.
    out.sort_by_key(|m| m.seen);
    out
}

fn kind_of(kind: &StoredKind) -> MemoryKind {
    match kind {
        StoredKind::OnThisDay => MemoryKind::OnThisDay,
        StoredKind::People { .. } => MemoryKind::People,
        StoredKind::Trip { .. } => MemoryKind::Trip,
        StoredKind::Clip { .. } => MemoryKind::Clip,
        StoredKind::Filler { .. } => MemoryKind::Filler,
        StoredKind::TimeDay { .. }
        | StoredKind::TimeWeek { .. }
        | StoredKind::TimeMonth { .. }
        | StoredKind::LastWeek
        | StoredKind::LastMonth => MemoryKind::Time,
    }
}

/// Ente's simple memories (the first on-this-day that shows today, then
/// fillers) from the index alone, while the first update runs.
fn simple_memories(
    inner: &Inner,
    now_us: i64,
    utc_offset_secs: i32,
    seen: &BTreeMap<String, i64>,
) -> Vec<Memory> {
    let index = inner.index();
    let request = MemoriesRequest {
        files: source::quick_files(inner, utc_offset_secs),
        excluded_collection_ids: source::excluded_collection_ids(inner),
        ..MemoriesRequest::default()
    };
    let period = Period {
        now: now_us,
        utc_offset_seconds: utc_offset_secs,
    };
    let memories = engine::compute_simple_memories(&request, period);
    let mut cache = StoredCache::empty(0);
    cache.memories = memories
        .into_iter()
        .map(|m| cache::stored(m, now_us))
        .collect();
    let year = engine::local_date(now_us + i64::from(utc_offset_secs) * 1_000_000).year;
    present(&cache, now_us, &index, seen, &HashMap::new(), None, year)
}

/// Current names of people (`None` for unnamed clusters).
fn person_names(inner: &Inner) -> HashMap<String, Option<String>> {
    crate::ml::faces::people(inner, true, 0)
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect()
}

/// The person who is the account holder (their `cgroup` email).
fn me_person_id(
    inner: &Inner,
    names: &HashMap<String, Option<String>>,
    email: &str,
) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    inner
        .store()
        .entities(crate::ml::entity::CGROUP)
        .ok()?
        .into_iter()
        .find(|(id, _, data)| {
            names.contains_key(id)
                && data
                    .get("email")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|e| e.eq_ignore_ascii_case(email))
        })
        .map(|(id, _, _)| id)
}

fn load_seen(inner: &Inner) -> BTreeMap<String, i64> {
    inner
        .store()
        .get_sealed(kv::MEMORIES_SEEN)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn mark_seen(inner: &Inner, memory_id: String) -> Result<()> {
    inner.session()?;
    let now = crate::store::now_us();
    let mut st = inner.ml.memories.lock();
    let mut seen = st.seen.take().unwrap_or_else(|| load_seen(inner));
    seen.retain(|_, t| now - *t < SEEN_RETENTION);
    seen.entry(memory_id.clone()).or_insert(now);
    let saved = inner.store().put_sealed(kv::MEMORIES_SEEN, &seen);
    st.seen = Some(seen);
    saved?;
    if let Some(d) = &mut st.day {
        for m in d.memories.iter_mut().filter(|m| m.id == memory_id) {
            m.seen = true;
        }
        d.memories.sort_by_key(|m| m.seen);
    }
    Ok(())
}

#[uniffi::export]
impl Core {
    /// Today's memories for the user's local time (`now_us` µs since the
    /// epoch, `utc_offset_secs` the local offset), unseen first.
    ///
    /// Fast: answered from a cache kept per local day; updates (every
    /// three days, like Ente) run in the background, see
    /// [`crate::ml::memories`].
    pub fn memories(&self, now_us: i64, utc_offset_secs: i32) -> Vec<Memory> {
        memories(&self.inner, now_us, utc_offset_secs)
    }

    /// Mark a memory as seen.
    pub fn mark_memory_seen(&self, memory_id: String) -> Result<(), CoreError> {
        mark_seen(&self.inner, memory_id)
    }
}
