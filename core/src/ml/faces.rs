//! Faces and people: Ente's face clustering, people as `cgroup` entities
//! (shared with the Ente apps), face crops.
//!
//! - [`cluster`]: the clustering algorithm (web client's, see there for
//!   which client was followed and why).
//! - [`cgroup`]: the `cgroup` JSON (unknown keys survive rewrites).
//! - `state`: [`rebuild`]: reads the face indexes from the sealed `mldata`
//!   rows (only rows that changed since the last run are decrypted),
//!   clusters, reconciles remote clusters, builds the people snapshot that
//!   the sync calls read.
//! - `suggest`: "review suggestions" for a person.
//! - `ops`: naming, merging, hiding, … (write-through to `/user-entity`).
//! - `crop`: face thumbnails (square JPEG, cached encrypted under the
//!   file key).
//!
//! Unnamed clusters are local (sealed in kv `ml:clusters`, with the
//! clusters the user rejected for each person); people are `cgroup`s.
//!
//! Internal API for `index`, `search` and `memories`: [`refresh`] (after
//! `mldata` sync: pull `cgroup`s, recluster, push cluster changes to
//! `cgroup`s), [`rebuild`] (local only), [`people`], [`person_file_ids`].
//!
//! Based on Ente (ente.com).

pub mod cgroup;
pub mod cluster;
mod crop;
mod ops;
mod state;
mod suggest;

use super::{FaceRef, FileFace, Person};
use crate::client::Inner;
use crate::{Core, CoreError, Result, TimelineItem};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Directory of the face crop cache under the cache dir.
pub(crate) const CROP_CACHE_DIR: &str = "faces";

/// Prefix of the person id of an unnamed (local) cluster.
pub(crate) const CLUSTER_PREFIX: &str = "cluster:";

/// Clusters and people, rebuilt by [`rebuild`].
#[derive(Debug, Default)]
pub(crate) struct FacesCache {
    /// What the sync calls read.
    snapshot: RwLock<Arc<state::FacesState>>,
    /// Face metadata per file (no embeddings), reused across rebuilds.
    meta: Mutex<state::MetaMap>,
    /// Sample means of clusters, for the suggestion prefilter.
    means: Mutex<HashMap<String, suggest::SampleMean>>,
    /// Serialises rebuilds.
    rebuild_lock: Mutex<()>,
    /// Serialises people changes and refreshes (read-modify-write of
    /// `cgroup`s).
    op_lock: tokio::sync::Mutex<()>,
    /// A first rebuild was started (lazily, by a read).
    started: AtomicBool,
}

impl FacesCache {
    fn snapshot(&self) -> Arc<state::FacesState> {
        self.snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_snapshot(&self, s: state::FacesState) {
        *self.snapshot.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(s);
    }
}

/// The snapshot, if it belongs to the signed-in account.
fn snapshot(inner: &Inner) -> Option<Arc<state::FacesState>> {
    let user = inner.session().ok()?.account.user_id;
    let s = inner.ml.faces.snapshot();
    (s.built && s.user_id == user).then_some(s)
}

/// Recluster and rebuild people after `mldata` or `cgroup` changes. Local
/// only: changes to remote clusters are kept as pending until
/// [`refresh`] pushes them. CPU and disk work: call it off the async
/// workers (`crate::offload` / `spawn_blocking`), with the op lock held
/// (it rewrites the local clusters and rejections in kv `ml:clusters`,
/// which the people changes read-modify-write), or use [`rebuild_now`].
pub(crate) fn rebuild(inner: &Inner) -> Result<()> {
    state::rebuild(inner)
}

/// [`rebuild`] under the op lock, on the blocking pool.
pub(crate) async fn rebuild_now(inner: &Arc<Inner>) -> Result<()> {
    let _g = inner.ml.faces.op_lock.lock().await;
    let me = inner.clone();
    tokio::task::spawn_blocking(move || rebuild(&me))
        .await
        .map_err(|e| CoreError::storage(format!("task failed: {e}")))?
}

/// Pull `cgroup`s, recluster, and push the faces new to people's clusters
/// to their `cgroup`s (Ente's `updateClustersAndPeople` +
/// `reconcileClusters`). Call after every `mldata` sync.
pub(crate) async fn refresh(inner: &Inner) -> Result<()> {
    let _g = inner.ml.faces.op_lock.lock().await;
    ops::refresh_locked(inner).await
}

/// Whether [`refresh`] has work even if no `mldata` or `cgroup` changed:
/// nothing was built for this account yet, the library index changed
/// since the last build, or cluster changes still wait to be pushed.
pub(crate) fn needs_refresh(inner: &Inner) -> bool {
    let Some(s) = snapshot(inner) else {
        return true;
    };
    !Arc::ptr_eq(&s.index, &inner.index()) || !s.pending.is_empty()
}

/// People (named first, then unnamed clusters with at least
/// `min_files` files), hidden ones only if `include_hidden`.
pub(crate) fn people(inner: &Inner, include_hidden: bool, min_files: u32) -> Vec<Person> {
    let Some(s) = snapshot(inner) else {
        return Vec::new();
    };
    s.people
        .iter()
        .filter(|p| match p.who {
            state::Who::CGroup(_) => include_hidden || !p.person.is_hidden,
            state::Who::Cluster(_) => p.person.file_count >= u64::from(min_files),
        })
        .map(|p| p.person.clone())
        .collect()
}

/// Files of a person or cluster, newest first.
pub(crate) fn person_file_ids(inner: &Inner, person_id: &str) -> Vec<i64> {
    snapshot(inner)
        .and_then(|s| s.person(person_id).map(|p| p.file_ids.clone()))
        .unwrap_or_default()
}

/// Face id → file id (`<fileID>_…`).
pub(crate) fn file_id_of(face_id: &str) -> Option<i64> {
    face_id.split('_').next()?.parse().ok()
}

impl Core {
    /// Start the first rebuild in the background if nothing has built the
    /// people yet (e.g. right after the app starts).
    fn ensure_faces_built(&self) {
        let cache = &self.inner.ml.faces;
        if snapshot(&self.inner).is_some()
            || self.inner.session().is_err()
            || cache.started.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let inner = self.inner.clone();
        crate::runtime().spawn_blocking(move || {
            // Not concurrently with a people change: both rewrite
            // `ml:clusters`, and the older copy would win.
            let g = inner.ml.faces.op_lock.blocking_lock();
            let _ = rebuild(&inner);
            drop(g);
            inner.ml.faces.started.store(false, Ordering::Release);
        });
    }

    /// Put decrypted `mldata` JSON for a file into the store, as the
    /// indexer does after a pull. For tests of the people flows.
    #[doc(hidden)]
    pub fn ml_testing_put_mldata(&self, file_id: i64, json: String) -> Result<(), CoreError> {
        let d = super::mldata::RemoteMlData::from_json(json.as_bytes(), None)
            .map_err(|e| CoreError::storage(format!("mldata: {e}")))?;
        self.inner.store().put_ml_data(file_id, &d)
    }

    /// Run [`refresh`] now. For tests of the people flows.
    #[doc(hidden)]
    pub async fn ml_testing_refresh_faces(&self) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        crate::run(async move { refresh(&inner).await }).await
    }
}

#[uniffi::export]
impl Core {
    /// People and unnamed clusters with at least `min_files` files, named
    /// first (most files first), hidden ones only with `include_hidden`.
    pub fn people(&self, include_hidden: bool, min_files: u32) -> Vec<Person> {
        self.ensure_faces_built();
        people(&self.inner, include_hidden, min_files)
    }

    /// One person or cluster.
    pub fn person(&self, person_id: String) -> Option<Person> {
        self.ensure_faces_built();
        snapshot(&self.inner)?
            .person(&person_id)
            .map(|p| p.person.clone())
    }

    /// Files of a person or cluster, newest first.
    pub fn person_files(&self, person_id: String) -> Vec<TimelineItem> {
        self.ensure_faces_built();
        let Some(s) = snapshot(&self.inner) else {
            return Vec::new();
        };
        let Some(p) = s.person(&person_id) else {
            return Vec::new();
        };
        p.file_ids.iter().filter_map(|id| s.item(*id)).collect()
    }

    /// Faces found on a file, with who they are.
    pub fn file_faces(&self, file_id: i64) -> Vec<FileFace> {
        self.ensure_faces_built();
        let Some(s) = snapshot(&self.inner) else {
            return Vec::new();
        };
        let Some(ff) = s.meta.get(&file_id) else {
            return Vec::new();
        };
        ff.faces
            .iter()
            .filter_map(|m| {
                let who = s.person_of_face.get(&m.face_id).map(|&i| &s.people[i]);
                // Placeholders and weak detections are not shown unless
                // someone is assigned to them.
                if who.is_none() && m.score <= cluster::MIN_SCORE as f32 {
                    return None;
                }
                Some(FileFace {
                    face: m.face_ref(file_id),
                    person_id: who.map(|p| p.person.id.clone()),
                    person_name: who.and_then(|p| p.person.name.clone()),
                })
            })
            .collect()
    }

    /// Unnamed clusters that look like this person, best first
    /// ("Review suggestions").
    pub fn person_suggestions(&self, person_id: String) -> Vec<Person> {
        self.ensure_faces_built();
        let Some(s) = snapshot(&self.inner) else {
            return Vec::new();
        };
        s.suggestions
            .get(&person_id)
            .into_iter()
            .flatten()
            .filter_map(|id| s.person(id).map(|p| p.person.clone()))
            .collect()
    }

    /// A square JPEG crop of a face (cached, encrypted at rest).
    pub async fn face_thumbnail(&self, face_id: String) -> Result<Vec<u8>, CoreError> {
        let inner = self.inner.clone();
        crate::run(async move { crop::face_thumbnail(&inner, &face_id).await }).await
    }

    /// Name a cluster (creates a `cgroup`) or rename a person.
    pub async fn name_person(&self, person_id: String, name: String) -> Result<Person, CoreError> {
        let inner = self.inner.clone();
        crate::run(async move { ops::name_person(&inner, &person_id, &name).await }).await
    }

    /// Merge `from_id` (person or cluster) into `into_id`.
    pub async fn merge_people(
        &self,
        into_id: String,
        from_id: String,
    ) -> Result<Person, CoreError> {
        let inner = self.inner.clone();
        crate::run(async move { ops::merge_people(&inner, &into_id, &from_id).await }).await
    }

    /// Accept or reject a suggested cluster for a person.
    pub async fn answer_suggestion(
        &self,
        person_id: String,
        cluster_id: String,
        accept: bool,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        crate::run(
            async move { ops::answer_suggestion(&inner, &person_id, &cluster_id, accept).await },
        )
        .await
    }

    /// Hide or unhide a person (Ente's "ignore").
    pub async fn set_person_hidden(
        &self,
        person_id: String,
        hidden: bool,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        crate::run(async move { ops::set_person_hidden(&inner, &person_id, hidden).await }).await
    }

    /// Choose a person's avatar face.
    pub async fn set_person_cover(
        &self,
        person_id: String,
        face_id: String,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        crate::run(async move { ops::set_person_cover(&inner, &person_id, &face_id).await }).await
    }

    /// Set or clear a person's birth date (`YYYY-MM-DD`).
    pub async fn set_person_birth_date(
        &self,
        person_id: String,
        birth_date: Option<String>,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        crate::run(async move {
            ops::set_person_birth_date(&inner, &person_id, birth_date.as_deref()).await
        })
        .await
    }

    /// Remove faces that are not this person (they are rejected for it).
    pub async fn remove_faces_from_person(
        &self,
        person_id: String,
        face_ids: Vec<String>,
    ) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        crate::run(
            async move { ops::remove_faces_from_person(&inner, &person_id, &face_ids).await },
        )
        .await
    }

    /// Delete a person (its `cgroup`); the faces become unnamed clusters.
    pub async fn delete_person(&self, person_id: String) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        crate::run(async move { ops::delete_person(&inner, &person_id).await }).await
    }
}

impl state::FaceMeta {
    /// The record for this face.
    pub(crate) fn face_ref(&self, file_id: i64) -> FaceRef {
        FaceRef {
            face_id: self.face_id.clone(),
            file_id,
            x: self.x,
            y: self.y,
            width: self.w,
            height: self.h,
            score: self.score,
            blur: self.blur,
        }
    }
}
