//! The people snapshot and how it is rebuilt: face metadata from the
//! sealed `mldata` rows, clustering ([`super::cluster`]), reconciliation of
//! remote clusters (web `reconcileClusters`) and the people list (web
//! `reconstructPeopleState`).
//!
//! Memory stays bounded for large libraries: the snapshot holds face
//! metadata (box, score, blur; no embeddings) and people; embeddings are
//! read per clustering batch and per suggestion sample, then dropped.
//!
//! Based on Ente (ente.com).

use super::cgroup::{CGroupData, FaceCluster};
use super::cluster::{self, ClusterFace, RemoteGroup};
use super::{CLUSTER_PREFIX, suggest};
use crate::client::Inner;
use crate::library::Index;
use crate::ml::Person;
use crate::ml::entity::CGROUP;
use crate::ml::store::kv;
use crate::{Result, TimelineItem};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// One face, without its embedding.
#[derive(Debug, Clone)]
pub(crate) struct FaceMeta {
    pub face_id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    pub blur: f32,
    /// Takes part in clustering ([`cluster::is_clusterable`]).
    pub clusterable: bool,
    /// [`cluster::is_bad_face`].
    pub is_bad: bool,
}

/// The faces of one file and the seal tag of the row they came from.
#[derive(Debug)]
pub(crate) struct FileFaces {
    pub tag: Vec<u8>,
    pub faces: Vec<FaceMeta>,
}

/// Face metadata per file.
pub(crate) type MetaMap = Arc<HashMap<i64, Arc<FileFaces>>>;

/// What a person entry is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Who {
    /// A `cgroup` (its entity id).
    CGroup(String),
    /// A local cluster (its cluster id).
    Cluster(String),
}

/// A person with what the calls need.
#[derive(Debug, Clone)]
pub(crate) struct PersonEntry {
    pub person: Person,
    pub who: Who,
    /// Files, newest first.
    pub file_ids: Vec<i64>,
}

/// A `cgroup` rewrite found by reclustering, not pushed yet.
#[derive(Debug, Clone)]
pub(crate) struct Pending {
    pub id: String,
    /// `updatedAt` of the stored entity it was computed from: dropped if
    /// the entity changed meanwhile (the next rebuild recomputes it).
    pub base_updated_at: i64,
    pub data: CGroupData,
}

/// The people snapshot.
#[derive(Debug, Default)]
pub(crate) struct FacesState {
    pub built: bool,
    pub user_id: i64,
    /// The library index the positions refer to.
    pub index: Arc<Index>,
    /// File id → position in `index.all_items()`.
    pub pos: HashMap<i64, u32>,
    pub meta: MetaMap,
    /// `cgroup` people (pinned first, most files first), then clusters.
    pub people: Vec<PersonEntry>,
    pub by_id: HashMap<String, usize>,
    /// Face id → index in `people`.
    pub person_of_face: HashMap<String, usize>,
    /// Pending `cgroup` rewrites.
    pub pending: Vec<Pending>,
    /// `cgroup` id → suggested cluster person ids, best first.
    pub suggestions: HashMap<String, Vec<String>>,
}

impl FacesState {
    pub(crate) fn person(&self, id: &str) -> Option<&PersonEntry> {
        self.by_id.get(id).map(|&i| &self.people[i])
    }

    pub(crate) fn item(&self, file_id: i64) -> Option<TimelineItem> {
        let p = *self.pos.get(&file_id)?;
        self.index.all_items().get(p as usize).cloned()
    }

    pub(crate) fn face(&self, face_id: &str) -> Option<&FaceMeta> {
        let fid = super::file_id_of(face_id)?;
        self.meta
            .get(&fid)?
            .faces
            .iter()
            .find(|f| f.face_id == face_id)
    }
}

/// The sealed kv value `ml:clusters`: unnamed local clusters, and the
/// clusters the user rejected for each `cgroup` (web `kvdb`
/// `rejectedClustersForCGroup`, mobile "not person" feedback).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Saved {
    #[serde(default)]
    pub clusters: Vec<FaceCluster>,
    #[serde(default)]
    pub rejected: HashMap<String, Vec<String>>,
}

pub(crate) fn load_saved(inner: &Inner) -> Result<Saved> {
    Ok(inner
        .store()
        .get_sealed::<Saved>(kv::CLUSTERS)?
        .unwrap_or_default())
}

pub(crate) fn save_saved(inner: &Inner, s: &Saved) -> Result<()> {
    inner.store().put_sealed(kv::CLUSTERS, s)
}

/// Stored `cgroup`s: `(id, updatedAt, data)`.
pub(crate) fn load_cgroups(inner: &Inner) -> Result<Vec<(String, i64, CGroupData)>> {
    let mut v: Vec<_> = inner
        .store()
        .entities(CGROUP)?
        .into_iter()
        .map(|(id, t, d)| (id, t, CGroupData::from_value(d)))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(v)
}

/// Re-read the face metadata of rows that changed since the last run.
fn refresh_meta(inner: &Inner) -> Result<MetaMap> {
    let cache = &inner.ml.faces;
    let old = cache.meta.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let tags = inner.store().ml_row_tags()?;
    let opener = inner.store().opener();
    let mut map = HashMap::with_capacity(tags.len());
    for (fid, tag) in tags {
        if let Some(f) = old.get(&fid)
            && f.tag == tag
        {
            map.insert(fid, f.clone());
            continue;
        }
        let row = inner.store().ml_row(fid)?;
        let Some(row) = row else { continue };
        let faces = match row.open(&opener) {
            Ok(d) => d
                .current_face()
                .map(|fi| fi.faces.iter().map(face_meta).collect())
                .unwrap_or_default(),
            // An undecodable row has no faces (the indexer replaces it).
            Err(_) => Vec::new(),
        };
        map.insert(
            fid,
            Arc::new(FileFaces {
                tag: row.seal_tag().to_vec(),
                faces,
            }),
        );
    }
    let map = Arc::new(map);
    *cache.meta.lock().unwrap_or_else(|e| e.into_inner()) = map.clone();
    Ok(map)
}

fn face_meta(f: &crate::ml::mldata::Face) -> FaceMeta {
    let b = &f.detection.face_box;
    let landmarks: Vec<(f64, f64)> = f.detection.landmarks.iter().map(|p| (p.x, p.y)).collect();
    FaceMeta {
        face_id: f.face_id.clone(),
        x: b.x as f32,
        y: b.y as f32,
        w: b.width as f32,
        h: b.height as f32,
        score: f.score as f32,
        blur: f.blur as f32,
        clusterable: cluster::is_clusterable(f.score, f.blur),
        is_bad: cluster::is_bad_face(f.score, f.blur, &landmarks),
    }
}

/// Embeddings of faces (aligned with `face_ids`; `None` when missing),
/// reading each file's row once.
pub(crate) fn load_embeddings(inner: &Inner, face_ids: &[&str]) -> Vec<Option<Vec<f32>>> {
    let mut by_file: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, id) in face_ids.iter().enumerate() {
        if let Some(fid) = super::file_id_of(id) {
            by_file.entry(fid).or_default().push(i);
        }
    }
    let mut out: Vec<Option<Vec<f32>>> = vec![None; face_ids.len()];
    let opener = inner.store().opener();
    for (fid, idxs) in by_file {
        let Ok(Some(row)) = inner.store().ml_row(fid) else {
            continue;
        };
        let Ok(d) = row.open(&opener) else { continue };
        let Some(fi) = d.current_face() else { continue };
        for i in idxs {
            if let Some(f) = fi.faces.iter().find(|f| f.face_id == face_ids[i]) {
                out[i] = Some(f.embedding.iter().map(|&x| x as f32).collect());
            }
        }
    }
    out
}

/// Rebuild the snapshot (see [`super::rebuild`]).
pub(crate) fn rebuild(inner: &Inner) -> Result<()> {
    let cache = &inner.ml.faces;
    let _g = cache.rebuild_lock.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(session) = inner.session() else {
        *cache.meta.lock().unwrap_or_else(|e| e.into_inner()) = MetaMap::default();
        cache.set_snapshot(FacesState::default());
        return Ok(());
    };
    let user_id = session.account.user_id;
    drop(session);

    let index = inner.index();
    let pos: HashMap<i64, u32> = index
        .all_items()
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id, i as u32))
        .collect();
    let meta = refresh_meta(inner)?;
    let cgroups = load_cgroups(inner)?;
    let saved = load_saved(inner)?;

    // Clusterable faces of files in the library, newest first.
    let mut faces: Vec<(u32, &FaceMeta)> = meta
        .iter()
        .filter_map(|(fid, ff)| pos.get(fid).map(|p| (*p, ff)))
        .flat_map(|(p, ff)| {
            ff.faces
                .iter()
                .filter(|f| f.clusterable)
                .map(move |f| (p, f))
        })
        .collect();
    faces.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.face_id.cmp(&b.1.face_id)));
    let known: HashSet<&str> = faces.iter().map(|(_, f)| f.face_id.as_str()).collect();

    // Remote clusters.
    let groups: Vec<RemoteGroup> = cgroups
        .iter()
        .map(|(_, t, d)| RemoteGroup {
            updated_at: *t,
            assigned: d.assigned(),
            rejected_face_ids: d.rejected_face_ids(),
        })
        .collect();
    let remote_ids: HashSet<String> = groups
        .iter()
        .flat_map(|g| g.assigned.iter().map(|c| c.id.clone()))
        .collect();
    let remote_faces: HashSet<String> = cgroups
        .iter()
        .flat_map(|(_, _, d)| d.effective_clusters())
        .flat_map(|c| c.faces)
        .collect();

    // Local clusters, without faces that are gone or now belong to a
    // remote cluster (remote assignments win, as in Ente).
    let mut local: Vec<FaceCluster> = saved
        .clusters
        .iter()
        .filter(|c| !remote_ids.contains(&c.id))
        .cloned()
        .collect();
    for c in &mut local {
        c.faces
            .retain(|f| known.contains(f.as_str()) && !remote_faces.contains(f));
    }
    local.retain(|c| !c.faces.is_empty());
    drop(known);

    let cfaces: Vec<ClusterFace> = faces
        .iter()
        .map(|(_, f)| ClusterFace {
            face_id: f.face_id.clone(),
            is_bad: f.is_bad,
        })
        .collect();
    let result = cluster::cluster_faces(
        &cfaces,
        &groups,
        local,
        |idxs| {
            let ids: Vec<&str> = idxs.iter().map(|&i| cfaces[i].face_id.as_str()).collect();
            load_embeddings(inner, &ids)
        },
        cluster::new_cluster_id,
    );
    drop(cfaces);
    drop(faces);

    // Reconcile: cgroups whose clusters got faces are rewritten.
    let by_id: HashMap<&str, &FaceCluster> =
        result.clusters.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut pending = Vec::new();
    for (id, t, d) in &cgroups {
        let assigned = d.assigned();
        if !assigned.iter().any(|c| result.modified.contains(&c.id)) {
            continue;
        }
        let mut data = d.clone();
        data.set_assigned(
            assigned
                .into_iter()
                .map(|c| by_id.get(c.id.as_str()).map_or(c, |&n| n.clone()))
                .collect(),
        );
        pending.push(Pending {
            id: id.clone(),
            base_updated_at: *t,
            data,
        });
    }
    let new_local: Vec<FaceCluster> = result
        .clusters
        .iter()
        .filter(|c| !remote_ids.contains(&c.id))
        .cloned()
        .collect();
    drop(by_id);
    let mut new_saved = Saved {
        clusters: new_local,
        rejected: saved.rejected.clone(),
    };
    // Rejections of cgroups that are gone are dropped.
    let cgroup_ids: HashSet<&str> = cgroups.iter().map(|(id, _, _)| id.as_str()).collect();
    new_saved
        .rejected
        .retain(|k, _| cgroup_ids.contains(k.as_str()));
    if new_saved != saved {
        save_saved(inner, &new_saved)?;
    }

    // The cgroups as they will be once the pending rewrites are pushed.
    let mut effective: Vec<(String, CGroupData)> =
        cgroups.into_iter().map(|(id, _, d)| (id, d)).collect();
    for p in &pending {
        if let Some(e) = effective.iter_mut().find(|e| e.0 == p.id) {
            e.1 = p.data.clone();
        }
    }

    let mut st = people_state(&index, pos, meta, &effective, &new_saved.clusters);
    st.user_id = user_id;
    st.pending = pending;
    st.suggestions = suggest::compute(inner, &st, &effective, &new_saved);
    st.built = true;
    cache.set_snapshot(st);
    Ok(())
}

/// Ente's `reconstructPeopleState`.
fn people_state(
    index: &Arc<Index>,
    pos: HashMap<i64, u32>,
    meta: MetaMap,
    cgroups: &[(String, CGroupData)],
    local: &[FaceCluster],
) -> FacesState {
    // Files of non-hidden collections (Ente's "normal collection files").
    let mut allowed: HashSet<i64> = HashSet::new();
    for c in index.visible_collections() {
        allowed.extend(index.collection_items(c.id).map(|t| t.id));
    }
    let lookup = |face_id: &str| -> Option<(i64, &FaceMeta, u32)> {
        let fid = super::file_id_of(face_id)?;
        if !allowed.contains(&fid) {
            return None;
        }
        let m = meta
            .get(&fid)?
            .faces
            .iter()
            .find(|f| f.face_id == face_id)?;
        Some((fid, m, *pos.get(&fid)?))
    };
    // Newest first; equal times: higher score first.
    let sorted_faces = |ids: &mut dyn Iterator<Item = &String>| {
        let mut v: Vec<(i64, &FaceMeta, u32)> = ids.filter_map(|id| lookup(id)).collect();
        v.sort_by(|a, b| {
            a.2.cmp(&b.2)
                .then_with(|| b.1.score.total_cmp(&a.1.score))
                .then_with(|| a.1.face_id.cmp(&b.1.face_id))
        });
        v
    };
    let files_newest_first = |mut ids: Vec<i64>| {
        ids.sort_by_key(|id| (pos.get(id).copied().unwrap_or(u32::MAX), *id));
        ids.dedup();
        ids
    };

    let mut named: Vec<(bool, PersonEntry)> = Vec::new();
    let mut face_owner: Vec<(Vec<String>, String)> = Vec::new();
    for (id, d) in cgroups {
        let clusters = d.effective_clusters();
        let faces = sorted_faces(&mut clusters.iter().flat_map(|c| c.faces.iter()));
        let Some(most_recent) = faces.first() else {
            continue;
        };
        let mut file_ids: Vec<i64> = faces.iter().map(|f| f.0).collect();
        file_ids.extend(
            d.manually_assigned()
                .into_iter()
                .filter(|f| allowed.contains(f)),
        );
        let file_ids = files_newest_first(file_ids);
        let avatar = d.avatar_face_id().and_then(lookup);
        let display = avatar.unwrap_or(*most_recent);
        named.push((
            d.is_pinned(),
            PersonEntry {
                person: Person {
                    id: id.clone(),
                    name: d.name().map(str::to_string),
                    is_hidden: d.is_ignored(),
                    file_count: file_ids.len() as u64,
                    cover: Some(display.1.face_ref(display.0)),
                    birth_date: d.birth_date().map(str::to_string),
                },
                who: Who::CGroup(id.clone()),
                file_ids,
            },
        ));
        face_owner.push((
            clusters.into_iter().flat_map(|c| c.faces).collect(),
            id.clone(),
        ));
    }
    let mut unnamed: Vec<PersonEntry> = Vec::new();
    for c in local {
        let faces = sorted_faces(&mut c.faces.iter());
        let Some(most_recent) = faces.first() else {
            continue;
        };
        let file_ids = files_newest_first(faces.iter().map(|f| f.0).collect());
        let id = format!("{CLUSTER_PREFIX}{}", c.id);
        unnamed.push(PersonEntry {
            person: Person {
                id: id.clone(),
                name: None,
                is_hidden: false,
                file_count: file_ids.len() as u64,
                cover: Some(most_recent.1.face_ref(most_recent.0)),
                birth_date: None,
            },
            who: Who::Cluster(c.id.clone()),
            file_ids,
        });
        face_owner.push((c.faces.clone(), id));
    }
    named.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.file_ids.len().cmp(&a.1.file_ids.len()))
            .then_with(|| a.1.person.id.cmp(&b.1.person.id))
    });
    unnamed.sort_by(|a, b| {
        b.file_ids
            .len()
            .cmp(&a.file_ids.len())
            .then_with(|| a.person.id.cmp(&b.person.id))
    });
    let people: Vec<PersonEntry> = named.into_iter().map(|(_, p)| p).chain(unnamed).collect();
    let by_id: HashMap<String, usize> = people
        .iter()
        .enumerate()
        .map(|(i, p)| (p.person.id.clone(), i))
        .collect();
    let mut person_of_face = HashMap::new();
    // cgroups come first in `face_owner`: their assignment wins.
    for (faces, id) in face_owner {
        let Some(&i) = by_id.get(&id) else { continue };
        for f in faces {
            person_of_face.entry(f).or_insert(i);
        }
    }
    FacesState {
        built: false,
        user_id: 0,
        index: index.clone(),
        pos,
        meta,
        people,
        by_id,
        person_of_face,
        pending: Vec::new(),
        suggestions: HashMap::new(),
    }
}
