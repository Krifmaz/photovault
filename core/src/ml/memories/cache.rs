//! The memories cache, as Ente's `MemoriesCacheService` keeps it: the
//! memories computed for now and for the next update (`toShowMemories`,
//! each with its show window and calculation time) and the logs of what
//! was shown (people, CLIP themes, trips), which the engine uses to rotate
//! people and themes and to rest trips.
//!
//! Stored sealed under the device key in kv [`CACHE_KEY`]. Pure functions
//! here; the parent module does the I/O and threading.
//!
//! Based on Ente (ente.com).

use super::engine::{
    self, CachedTrip, ClipMemoryType, Coordinate, MemoriesRequest, MemoryKind, PeopleMemoryType,
    ShownClipMemory, ShownPerson, ShownTrip, TimeMemory, UPDATE_FREQUENCY_DAYS,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// kv key of the sealed [`StoredCache`] (owned by `memories`).
pub(crate) const CACHE_KEY: &str = "ml:memories_cache";
/// Format of [`StoredCache`]; another value is ignored and recomputed.
pub(crate) const VERSION: u32 = 1;

pub(crate) const MICROS_PER_DAY: i64 = 86_400_000_000;
/// Ente: `kMemoriesUpdateFrequency`.
pub(crate) const UPDATE_FREQUENCY: i64 = UPDATE_FREQUENCY_DAYS * MICROS_PER_DAY;
/// Ente: `maxShowTimeout` = max(person 16 × 3 days, trip 40 × 3 days) × 3.
const MAX_SHOW_TIMEOUT: i64 = 3 * 40 * UPDATE_FREQUENCY_DAYS * MICROS_PER_DAY;
/// Ente: `kTripSurfaceSlots` (trips carried into the next cache).
const TRIP_CARRY_FORWARD_LIMIT: usize = 2;

/// What a cached memory is (enough to title it and log it once shown).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(crate) enum StoredKind {
    OnThisDay,
    People {
        person_id: String,
        unnamed: bool,
        /// [`PeopleMemoryType`] variant name.
        memory_type: String,
        /// [`PeopleActivity`] variant name.
        activity: Option<String>,
        /// `(is_upcoming, new_age)`.
        birthday: Option<(bool, i32)>,
    },
    Trip {
        key: String,
        latitude: f64,
        longitude: f64,
        name: Option<String>,
        year: i32,
    },
    Clip {
        /// [`ClipMemoryType`] variant name.
        clip: String,
    },
    TimeDay {
        year: i32,
        month: i32,
        day: i32,
        years_ago: Option<i32>,
    },
    TimeWeek {
        years_ago: Option<i32>,
    },
    TimeMonth {
        year: i32,
        month: i32,
        years_ago: Option<i32>,
    },
    LastWeek,
    LastMonth,
    Filler {
        years_ago: i32,
    },
}

/// One cached memory (Ente's `ToShowMemory`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredMemory {
    pub id: String,
    pub kind: StoredKind,
    pub file_ids: Vec<i64>,
    /// Shown from (inclusive), µs.
    pub first: i64,
    /// Shown until (exclusive), µs.
    pub last: i64,
    /// When it was calculated for (now or the next update), µs.
    pub calc: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PeopleLog {
    pub person_id: String,
    pub memory_type: String,
    pub last_shown: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ClipLog {
    pub clip: String,
    pub last_shown: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct TripLog {
    pub key: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub last_shown: i64,
}

/// Ente's `MemoriesCache`, plus when and for whom it was computed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredCache {
    pub version: u32,
    /// Account it belongs to.
    pub user_id: i64,
    /// Last update (Ente: `lastMemoriesCacheUpdateTime`), µs.
    pub updated_at: i64,
    pub memories: Vec<StoredMemory>,
    pub people_logs: Vec<PeopleLog>,
    pub clip_logs: Vec<ClipLog>,
    pub trip_logs: Vec<TripLog>,
}

impl StoredCache {
    pub(crate) fn empty(user_id: i64) -> Self {
        Self {
            version: VERSION,
            user_id,
            updated_at: 0,
            memories: Vec::new(),
            people_logs: Vec::new(),
            clip_logs: Vec::new(),
            trip_logs: Vec::new(),
        }
    }

    /// Ente's `_timeToUpdateCache`: the last update is older than the
    /// update frequency (or in the future: the clock moved back).
    pub(crate) fn due(&self, now: i64) -> bool {
        now >= self.updated_at + UPDATE_FREQUENCY || now < self.updated_at
    }

    /// The memories to show at `now` (Ente's `shouldShowNow`), in cache
    /// order.
    pub(crate) fn showing(&self, now: i64) -> impl Iterator<Item = &StoredMemory> {
        self.memories.iter().filter(move |m| m.shows_at(now))
    }

    /// The next instant after `now` at which [`Self::showing`] can change.
    pub(crate) fn next_change(&self, now: i64) -> i64 {
        self.memories
            .iter()
            .flat_map(|m| [m.first, m.last, m.calc, m.calc + UPDATE_FREQUENCY])
            .filter(|&t| t > now)
            .min()
            .unwrap_or(i64::MAX)
    }
}

impl StoredMemory {
    fn relevant_at(&self, t: i64) -> bool {
        t >= self.first && t < self.last
    }

    /// Ente's `shouldShowNow`: in its window and calculated for this
    /// update (on this day and trips keep their own windows).
    pub(crate) fn shows_at(&self, now: i64) -> bool {
        let calculated_for_now = now >= self.calc && now < self.calc + UPDATE_FREQUENCY;
        self.relevant_at(now)
            && (calculated_for_now
                || matches!(self.kind, StoredKind::OnThisDay | StoredKind::Trip { .. }))
    }

    fn is_old(&self, now: i64) -> bool {
        now > self.last
    }

    fn trip_identity_key(&self) -> Option<String> {
        match &self.kind {
            StoredKind::Trip { key, .. } if !key.is_empty() => Some(format!("trip:{key}")),
            _ => None,
        }
    }
}

/// Ente's `_processOldCache`: keep recent shown logs, log the memories
/// whose window ended, carry forward active trips.
pub(crate) fn process_old(old: Option<&StoredCache>, now: i64, user_id: i64) -> StoredCache {
    let mut cache = StoredCache::empty(user_id);
    let Some(old) = old.filter(|o| o.user_id == user_id) else {
        return cache;
    };
    let recent = |t: i64| now - t < MAX_SHOW_TIMEOUT;
    cache.people_logs = old
        .people_logs
        .iter()
        .filter(|l| recent(l.last_shown))
        .cloned()
        .collect();
    cache.trip_logs = old
        .trip_logs
        .iter()
        .filter(|l| recent(l.last_shown))
        .cloned()
        .collect();
    cache.clip_logs = old
        .clip_logs
        .iter()
        .filter(|l| recent(l.last_shown))
        .cloned()
        .collect();
    cache.memories = active_trips(&old.memories, now);
    for m in old.memories.iter().filter(|m| m.is_old(now)) {
        match &m.kind {
            StoredKind::People {
                person_id,
                memory_type,
                ..
            } => cache.people_logs.push(PeopleLog {
                person_id: person_id.clone(),
                memory_type: memory_type.clone(),
                last_shown: m.last,
            }),
            StoredKind::Clip { clip } => cache.clip_logs.push(ClipLog {
                clip: clip.clone(),
                last_shown: m.last,
            }),
            StoredKind::Trip {
                key,
                latitude,
                longitude,
                ..
            } => cache.trip_logs.push(TripLog {
                key: Some(key.clone()).filter(|k| !k.is_empty()),
                latitude: *latitude,
                longitude: *longitude,
                last_shown: m.last,
            }),
            _ => {}
        }
    }
    cache
}

/// Ente's `_activeTripEntriesForTime`.
fn active_trips(memories: &[StoredMemory], now: i64) -> Vec<StoredMemory> {
    let active: Vec<StoredMemory> = memories
        .iter()
        .filter(|m| m.trip_identity_key().is_some() && m.relevant_at(now))
        .cloned()
        .collect();
    let mut active = dedupe_trips(active, now);
    active.sort_by_key(|m| (m.first, m.last));
    active.truncate(TRIP_CARRY_FORWARD_LIMIT);
    active
}

/// Ente's `_shouldPreferTripCacheEntry`.
fn prefer_trip(candidate: &StoredMemory, existing: &StoredMemory, now: i64) -> bool {
    let (a, b) = (candidate.relevant_at(now), existing.relevant_at(now));
    if a != b {
        return a;
    }
    if candidate.first != existing.first {
        return candidate.first < existing.first;
    }
    if candidate.last != existing.last {
        return candidate.last > existing.last;
    }
    false
}

/// Ente's `_dedupeTripCacheEntriesInOrder`: one entry per trip.
fn dedupe_trips(memories: Vec<StoredMemory>, now: i64) -> Vec<StoredMemory> {
    let mut preferred: Vec<(String, usize)> = Vec::new();
    for (i, m) in memories.iter().enumerate() {
        let Some(key) = m.trip_identity_key() else {
            continue;
        };
        match preferred.iter_mut().find(|(k, _)| *k == key) {
            Some((_, best)) => {
                if prefer_trip(m, &memories[*best], now) {
                    *best = i;
                }
            }
            None => preferred.push((key, i)),
        }
    }
    let keep: HashSet<usize> = preferred.into_iter().map(|(_, i)| i).collect();
    memories
        .into_iter()
        .enumerate()
        .filter(|(i, m)| m.trip_identity_key().is_none() || keep.contains(i))
        .map(|(_, m)| m)
        .collect()
}

/// The shown logs and carried trips of a processed cache, as the engine's
/// request fields.
pub(crate) fn fill_request(request: &mut MemoriesRequest, cache: &StoredCache) {
    request.shown_people = cache
        .people_logs
        .iter()
        .filter_map(|l| {
            Some(ShownPerson {
                person_id: l.person_id.clone(),
                memory_type: people_type(&l.memory_type)?,
                last_time_shown: l.last_shown,
            })
        })
        .collect();
    request.shown_trips = cache
        .trip_logs
        .iter()
        .map(|l| ShownTrip {
            key: l.key.clone(),
            location: Coordinate::new(l.latitude, l.longitude),
            last_time_shown: l.last_shown,
        })
        .collect();
    request.cached_trips = cache
        .memories
        .iter()
        .filter_map(|m| {
            Some(CachedTrip {
                identity_key: m.trip_identity_key()?,
                first_date_to_show: m.first,
                last_date_to_show: m.last,
            })
        })
        .collect();
    request.shown_clip_memories = cache
        .clip_logs
        .iter()
        .filter_map(|l| {
            Some(ShownClipMemory {
                kind: clip_type(&l.clip)?,
                last_time_shown: l.last_shown,
            })
        })
        .collect();
}

/// Ente's `updateCache` after the computation: the memories for now (with
/// the carried trips kept among the trips), then those for the next
/// update, one entry per trip.
pub(crate) fn merge(
    mut cache: StoredCache,
    now: i64,
    next: i64,
    now_memories: Vec<engine::Memory>,
    next_memories: Vec<engine::Memory>,
) -> StoredCache {
    let carried = std::mem::take(&mut cache.memories);
    let now_entries: Vec<StoredMemory> = now_memories.into_iter().map(|m| stored(m, now)).collect();
    let insert_at = now_entries
        .iter()
        .position(|e| matches!(e.kind, StoredKind::Trip { .. }))
        .or_else(|| {
            now_entries
                .iter()
                .position(|e| !matches!(e.kind, StoredKind::OnThisDay | StoredKind::People { .. }))
        })
        .unwrap_or(now_entries.len());
    let mut all = Vec::with_capacity(now_entries.len() + carried.len());
    let mut now_entries = now_entries.into_iter();
    all.extend(now_entries.by_ref().take(insert_at));
    all.extend(carried);
    all.extend(now_entries);
    all.extend(next_memories.into_iter().map(|m| stored(m, next)));
    let mut all = dedupe_trips(all, now);
    let mut ids = HashSet::new();
    all.retain(|m| !m.file_ids.is_empty() && ids.insert(m.id.clone()));
    cache.memories = all;
    cache.updated_at = now;
    cache
}

/// A cached memory from the engine's output.
pub(crate) fn stored(m: engine::Memory, calc: i64) -> StoredMemory {
    let kind = stored_kind(&m.kind);
    StoredMemory {
        id: memory_id(&kind, m.first_date_to_show),
        kind,
        file_ids: m.file_ids,
        first: m.first_date_to_show,
        last: m.last_date_to_show,
        calc,
    }
}

fn stored_kind(kind: &MemoryKind) -> StoredKind {
    match kind {
        MemoryKind::OnThisDay => StoredKind::OnThisDay,
        MemoryKind::People(p) => StoredKind::People {
            person_id: p.person_id.clone(),
            unnamed: p.is_unnamed_cluster,
            memory_type: format!("{:?}", p.memory_type),
            activity: p.activity.map(|a| format!("{a:?}")),
            birthday: p.birthday.map(|b| (b.is_upcoming, b.new_age)),
        },
        MemoryKind::Trip(t) => StoredKind::Trip {
            key: t.key.clone(),
            latitude: t.location.latitude,
            longitude: t.location.longitude,
            name: t.location_name.clone(),
            year: t.year,
        },
        // Only produced when surfacing everything (a debug mode).
        MemoryKind::Base(b) => StoredKind::Trip {
            key: String::new(),
            latitude: b.location.latitude,
            longitude: b.location.longitude,
            name: b.location_name.clone(),
            year: 0,
        },
        MemoryKind::Clip(c) => StoredKind::Clip {
            clip: format!("{c:?}"),
        },
        MemoryKind::Time(TimeMemory::Day { date, years_ago }) => StoredKind::TimeDay {
            year: date.year,
            month: date.month,
            day: date.day,
            years_ago: *years_ago,
        },
        MemoryKind::Time(TimeMemory::Week { years_ago }) => StoredKind::TimeWeek {
            years_ago: *years_ago,
        },
        MemoryKind::Time(TimeMemory::Month {
            year,
            month,
            years_ago,
        }) => StoredKind::TimeMonth {
            year: *year,
            month: *month,
            years_ago: *years_ago,
        },
        MemoryKind::Time(TimeMemory::LastWeek) => StoredKind::LastWeek,
        MemoryKind::Time(TimeMemory::LastMonth) => StoredKind::LastMonth,
        MemoryKind::Filler { years_ago } => StoredKind::Filler {
            years_ago: *years_ago,
        },
    }
}

/// A stable id: the kind, its show start and a hash of what it is.
fn memory_id(kind: &StoredKind, first: i64) -> String {
    let slug = match kind {
        StoredKind::OnThisDay => "otd",
        StoredKind::People { .. } => "people",
        StoredKind::Trip { .. } => "trip",
        StoredKind::Clip { .. } => "clip",
        StoredKind::TimeDay { .. }
        | StoredKind::TimeWeek { .. }
        | StoredKind::TimeMonth { .. }
        | StoredKind::LastWeek
        | StoredKind::LastMonth => "time",
        StoredKind::Filler { .. } => "filler",
    };
    // FNV-1a over the kind's JSON: stable across runs and builds.
    let json = serde_json::to_string(kind).unwrap_or_default();
    let hash = json.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{slug}-{first}-{hash:016x}")
}

fn people_type(name: &str) -> Option<PeopleMemoryType> {
    [
        PeopleMemoryType::YouAndThem,
        PeopleMemoryType::DoingSomethingTogether,
        PeopleMemoryType::Spotlight,
        PeopleMemoryType::LastTimeYouSawThem,
    ]
    .into_iter()
    .find(|t| format!("{t:?}") == name)
}

fn clip_type(name: &str) -> Option<ClipMemoryType> {
    ClipMemoryType::ALL
        .into_iter()
        .find(|t| format!("{t:?}") == name)
}

/// The variant name of a [`PeopleActivity`] (for tests and titles).
#[cfg(test)]
pub(crate) fn activity_name(a: engine::PeopleActivity) -> String {
    format!("{a:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ml::memories::engine::{Memory, PeopleActivity, PeopleMemory, TripMemory};

    const DAY: i64 = MICROS_PER_DAY;

    fn mem(kind: MemoryKind, first: i64, last: i64) -> Memory {
        Memory {
            kind,
            file_ids: vec![1, 2, 3],
            first_date_to_show: first,
            last_date_to_show: last,
        }
    }

    fn trip(key: &str) -> MemoryKind {
        MemoryKind::Trip(TripMemory {
            location: Coordinate::new(1.0, 2.0),
            location_name: None,
            year: 2020,
            key: key.into(),
        })
    }

    fn person(id: &str) -> MemoryKind {
        MemoryKind::People(PeopleMemory {
            person_id: id.into(),
            is_unnamed_cluster: false,
            memory_type: PeopleMemoryType::Spotlight,
            activity: Some(PeopleActivity::Hiking),
            birthday: None,
        })
    }

    #[test]
    fn windows_logs_and_carried_trips_follow_ente() {
        let now = 1_000 * DAY;
        let next = now + UPDATE_FREQUENCY;
        let cache = merge(
            StoredCache::empty(7),
            now,
            next,
            vec![
                mem(MemoryKind::OnThisDay, now, now + DAY),
                mem(person("p"), now, now + 3 * DAY),
                mem(trip("t1"), now, now + 10 * DAY),
                mem(MemoryKind::Clip(ClipMemoryType::Beach), now, now + 3 * DAY),
                mem(MemoryKind::Filler { years_ago: 2 }, now, now + 3 * DAY),
            ],
            vec![mem(
                MemoryKind::Filler { years_ago: 3 },
                next,
                next + 3 * DAY,
            )],
        );
        assert_eq!(cache.memories.len(), 6);
        assert_eq!(cache.showing(now).count(), 5);
        assert_eq!(cache.showing(now + DAY).count(), 4);
        assert!(!cache.due(now + DAY));
        assert!(cache.due(next));
        // The next update's filler shows after the update time only.
        let later: Vec<_> = cache.showing(next).map(|m| m.kind.clone()).collect();
        assert_eq!(
            later,
            [
                StoredKind::Trip {
                    key: "t1".into(),
                    latitude: 1.0,
                    longitude: 2.0,
                    name: None,
                    year: 2020
                },
                StoredKind::Filler { years_ago: 3 }
            ]
        );
        assert_eq!(cache.next_change(now), now + DAY);

        // Three days later: people and clip are logged, the trip carries.
        let processed = process_old(Some(&cache), next + 1, 7);
        assert_eq!(processed.people_logs.len(), 1);
        assert_eq!(processed.people_logs[0].memory_type, "Spotlight");
        assert_eq!(processed.clip_logs[0].clip, "Beach");
        assert_eq!(processed.memories.len(), 1);
        let mut request = MemoriesRequest::default();
        fill_request(&mut request, &processed);
        assert_eq!(request.cached_trips[0].identity_key, "trip:t1");
        assert_eq!(
            request.shown_people[0].memory_type,
            PeopleMemoryType::Spotlight
        );
        assert_eq!(request.shown_clip_memories[0].kind, ClipMemoryType::Beach);

        // Another account's cache is not reused.
        assert!(process_old(Some(&cache), next, 8).memories.is_empty());

        // The carried trip goes among the trips of the new update, once.
        let merged = merge(
            processed,
            next + 1,
            next + 1 + UPDATE_FREQUENCY,
            vec![
                mem(MemoryKind::OnThisDay, next, next + DAY),
                mem(trip("t1"), next + 1, next + 11 * DAY),
                mem(MemoryKind::Filler { years_ago: 2 }, next, next + 3 * DAY),
            ],
            vec![],
        );
        let kinds: Vec<&str> = merged
            .memories
            .iter()
            .map(|m| m.id.split('-').next().unwrap())
            .collect();
        assert_eq!(kinds, ["otd", "trip", "filler"]);
        assert_eq!(merged.memories[1].first, now);
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        let a = stored(mem(MemoryKind::Filler { years_ago: 2 }, 5, 6), 5);
        let b = stored(mem(MemoryKind::Filler { years_ago: 2 }, 5, 6), 9);
        let c = stored(mem(MemoryKind::Filler { years_ago: 3 }, 5, 6), 5);
        assert_eq!(a.id, b.id);
        assert_ne!(a.id, c.id);
        assert!(a.id.starts_with("filler-5-"));
        assert_eq!(activity_name(PeopleActivity::Feast), "Feast");
    }
}
