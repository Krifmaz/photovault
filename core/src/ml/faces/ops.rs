//! Changes to people, written through to the `cgroup` entities like the
//! Ente apps do (web `services/ml/index.ts` `addCGroup`,
//! `addClusterToCGroup`, `renameCGroup`, `deleteCGroup`, `ignoreCluster`,
//! `_applyPersonSuggestionUpdates` in `people.ts`; mobile
//! `PersonService.removeFacesFromPerson` and `ClusterFeedbackService` for
//! removing faces).
//!
//! Every change runs under the faces op lock: pull `cgroup`s, read-modify-
//! write the entity (unknown keys kept), update the local clusters, then
//! rebuild the snapshot off the async workers.
//!
//! Based on Ente (ente.com).

use super::cgroup::{CGroupData, FaceCluster, is_valid_birth_date};
use super::cluster::new_cluster_id;
use super::state::{Saved, load_cgroups, load_saved, save_saved};
use super::{CLUSTER_PREFIX, rebuild};
use crate::client::Inner;
use crate::ml::Person;
use crate::ml::entity::{self, CGROUP};
use crate::{CoreError, Result};
use std::collections::HashSet;

/// A person id, parsed.
enum Target {
    CGroup(String),
    Cluster(String),
}

fn target(person_id: &str) -> Result<Target> {
    let id = person_id.trim();
    if id.is_empty() {
        return Err(CoreError::storage("empty person id"));
    }
    Ok(match id.strip_prefix(CLUSTER_PREFIX) {
        Some(c) => Target::Cluster(c.to_string()),
        None => Target::CGroup(id.to_string()),
    })
}

/// Rebuild off the async workers, then let search and memories know.
async fn rebuilt(inner: &Inner) -> Result<()> {
    crate::offload(|| rebuild(inner))?;
    crate::ml::search::invalidate(inner);
    crate::ml::memories::invalidate(inner);
    Ok(())
}

/// [`super::refresh`] with the op lock held.
pub(crate) async fn refresh_locked(inner: &Inner) -> Result<()> {
    entity::sync(inner, CGROUP).await?;
    rebuilt(inner).await?;
    let pending = inner.ml.faces.snapshot().pending.clone();
    if pending.is_empty() {
        return Ok(());
    }
    let stored = load_cgroups(inner)?;
    let mut pushed = false;
    for p in pending {
        // Only on top of the version it was computed from.
        if stored
            .iter()
            .any(|(id, t, _)| *id == p.id && *t == p.base_updated_at)
        {
            entity::update(inner, CGROUP, &p.id, &p.data.to_value()).await?;
            pushed = true;
        }
    }
    if pushed {
        entity::sync(inner, CGROUP).await?;
        rebuilt(inner).await?;
    }
    Ok(())
}

/// Pull `cgroup`s and return the one with `id`.
async fn fresh_cgroup(inner: &Inner, id: &str) -> Result<CGroupData> {
    entity::sync(inner, CGROUP).await?;
    stored_cgroup(inner, id)
}

fn stored_cgroup(inner: &Inner, id: &str) -> Result<CGroupData> {
    load_cgroups(inner)?
        .into_iter()
        .find(|(i, _, _)| i == id)
        .map(|(_, _, d)| d)
        .ok_or_else(|| CoreError::storage(format!("unknown person {id}")))
}

fn take_local(saved: &mut Saved, cluster_id: &str) -> Result<FaceCluster> {
    let i = saved
        .clusters
        .iter()
        .position(|c| c.id == cluster_id)
        .ok_or_else(|| CoreError::storage(format!("unknown cluster {cluster_id}")))?;
    Ok(saved.clusters.remove(i))
}

/// Put clusters back among the local ones (merging into one with the same
/// id).
fn restore_local(saved: &mut Saved, clusters: Vec<FaceCluster>) {
    for c in clusters {
        if c.faces.is_empty() {
            continue;
        }
        match saved.clusters.iter_mut().find(|l| l.id == c.id) {
            Some(l) => {
                let have: HashSet<String> = l.faces.iter().cloned().collect();
                l.faces
                    .extend(c.faces.into_iter().filter(|f| !have.contains(f)));
            }
            None => saved.clusters.push(c),
        }
    }
}

fn unreject(saved: &mut Saved, cgroup_id: &str, cluster_id: &str) {
    if let Some(r) = saved.rejected.get_mut(cgroup_id) {
        r.retain(|c| c != cluster_id);
        if r.is_empty() {
            saved.rejected.remove(cgroup_id);
        }
    }
}

fn reject(saved: &mut Saved, cgroup_id: &str, cluster_id: &str) {
    let r = saved.rejected.entry(cgroup_id.to_string()).or_default();
    if !r.iter().any(|c| c == cluster_id) {
        r.push(cluster_id.to_string());
    }
}

/// `assigned += cluster`, its faces no longer rejected (web
/// `addClusterToCGroup`).
fn add_cluster(d: &mut CGroupData, cluster: FaceCluster) {
    let faces: HashSet<&str> = cluster.faces.iter().map(String::as_str).collect();
    let rejected: Vec<String> = d
        .rejected_face_ids()
        .into_iter()
        .filter(|f| !faces.contains(f.as_str()))
        .collect();
    let mut assigned = d.assigned();
    match assigned.iter_mut().find(|c| c.id == cluster.id) {
        Some(c) => {
            let have: HashSet<String> = c.faces.iter().cloned().collect();
            c.faces
                .extend(cluster.faces.iter().filter(|f| !have.contains(*f)).cloned());
        }
        None => assigned.push(cluster.clone()),
    }
    d.set_assigned(assigned);
    d.set_rejected_face_ids(rejected);
}

/// The person after a change (a placeholder if it has no visible faces).
fn person_or_placeholder(inner: &Inner, id: &str, name: Option<&str>) -> Person {
    super::snapshot(inner)
        .and_then(|s| s.person(id).map(|p| p.person.clone()))
        .unwrap_or_else(|| Person {
            id: id.to_string(),
            name: name.map(str::to_string),
            is_hidden: false,
            file_count: 0,
            cover: None,
            birth_date: None,
        })
}

pub(crate) async fn name_person(inner: &Inner, person_id: &str, name: &str) -> Result<Person> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::storage("a name is required"));
    }
    let t = target(person_id)?;
    let _g = inner.ml.faces.op_lock.lock().await;
    let id = match t {
        Target::Cluster(cid) => {
            entity::sync(inner, CGROUP).await?;
            let mut saved = load_saved(inner)?;
            let cluster = take_local(&mut saved, &cid)?;
            let id = entity::create(
                inner,
                CGROUP,
                &CGroupData::new(name, cluster, false).to_value(),
            )
            .await?;
            save_saved(inner, &saved)?;
            id
        }
        Target::CGroup(id) => {
            let mut d = fresh_cgroup(inner, &id).await?;
            // Naming an ignored cluster (no name) shows it again.
            if d.name().is_none() {
                d.set_hidden(false);
            }
            d.set_name(name);
            entity::update(inner, CGROUP, &id, &d.to_value()).await?;
            id
        }
    };
    rebuilt(inner).await?;
    Ok(person_or_placeholder(inner, &id, Some(name)))
}

pub(crate) async fn merge_people(inner: &Inner, into_id: &str, from_id: &str) -> Result<Person> {
    let (into, from) = (target(into_id)?, target(from_id)?);
    let _g = inner.ml.faces.op_lock.lock().await;
    entity::sync(inner, CGROUP).await?;
    let mut saved = load_saved(inner)?;
    let result = match (into, from) {
        (Target::CGroup(a), Target::CGroup(b)) => {
            if a == b {
                return Err(CoreError::storage("cannot merge a person into itself"));
            }
            let mut da = stored_cgroup(inner, &a)?;
            let db = stored_cgroup(inner, &b)?;
            for c in db.effective_clusters() {
                add_cluster(&mut da, c);
            }
            let mut manual = da.manually_assigned();
            for f in db.manually_assigned() {
                if !manual.contains(&f) {
                    manual.push(f);
                }
            }
            da.set_manually_assigned(manual);
            if da.birth_date().is_none()
                && let Some(bd) = db.birth_date()
            {
                da.set_birth_date(Some(bd));
            }
            entity::update(inner, CGROUP, &a, &da.to_value()).await?;
            entity::delete(inner, CGROUP, &b).await?;
            if let Some(r) = saved.rejected.remove(&b) {
                for c in r {
                    reject(&mut saved, &a, &c);
                }
            }
            a
        }
        (Target::CGroup(g), Target::Cluster(c)) | (Target::Cluster(c), Target::CGroup(g)) => {
            let mut d = stored_cgroup(inner, &g)?;
            let cluster = take_local(&mut saved, &c)?;
            unreject(&mut saved, &g, &cluster.id);
            add_cluster(&mut d, cluster);
            entity::update(inner, CGROUP, &g, &d.to_value()).await?;
            g
        }
        (Target::Cluster(a), Target::Cluster(b)) => {
            if a == b {
                return Err(CoreError::storage("cannot merge a cluster into itself"));
            }
            let from = take_local(&mut saved, &b)?;
            if !saved.clusters.iter().any(|c| c.id == a) {
                return Err(CoreError::storage(format!("unknown cluster {a}")));
            }
            restore_local(
                &mut saved,
                vec![FaceCluster {
                    id: a.clone(),
                    faces: from.faces,
                }],
            );
            format!("{CLUSTER_PREFIX}{a}")
        }
    };
    save_saved(inner, &saved)?;
    rebuilt(inner).await?;
    Ok(person_or_placeholder(inner, &result, None))
}

pub(crate) async fn answer_suggestion(
    inner: &Inner,
    person_id: &str,
    cluster_id: &str,
    accept: bool,
) -> Result<()> {
    let Target::CGroup(g) = target(person_id)? else {
        return Err(CoreError::storage("suggestions are for named people"));
    };
    let cid = cluster_id
        .strip_prefix(CLUSTER_PREFIX)
        .unwrap_or(cluster_id)
        .to_string();
    let _g = inner.ml.faces.op_lock.lock().await;
    let mut d = fresh_cgroup(inner, &g).await?;
    let mut saved = load_saved(inner)?;
    if accept {
        // "assign": the cluster joins the person; no longer rejected.
        let cluster = take_local(&mut saved, &cid)?;
        unreject(&mut saved, &g, &cid);
        add_cluster(&mut d, cluster);
        entity::update(inner, CGROUP, &g, &d.to_value()).await?;
    } else {
        // "rejectSuggestion": unassign if assigned (restored locally), and
        // never suggest it again.
        let mut assigned = d.assigned();
        if let Some(i) = assigned.iter().position(|c| c.id == cid) {
            let c = assigned.remove(i);
            d.set_assigned(assigned);
            entity::update(inner, CGROUP, &g, &d.to_value()).await?;
            restore_local(&mut saved, vec![c]);
        }
        reject(&mut saved, &g, &cid);
    }
    save_saved(inner, &saved)?;
    rebuilt(inner).await
}

pub(crate) async fn set_person_hidden(inner: &Inner, person_id: &str, hidden: bool) -> Result<()> {
    let t = target(person_id)?;
    let _g = inner.ml.faces.op_lock.lock().await;
    match t {
        Target::CGroup(id) => {
            let mut d = fresh_cgroup(inner, &id).await?;
            if !hidden && d.name().is_none() {
                // An ignored cluster: un-ignoring makes its faces an unnamed
                // cluster again.
                entity::delete(inner, CGROUP, &id).await?;
                let mut saved = load_saved(inner)?;
                restore_local(&mut saved, d.effective_clusters());
                saved.rejected.remove(&id);
                save_saved(inner, &saved)?;
            } else if d.hidden_flag() != hidden {
                d.set_hidden(hidden);
                entity::update(inner, CGROUP, &id, &d.to_value()).await?;
            }
        }
        Target::Cluster(cid) => {
            if hidden {
                ignore_cluster(inner, &cid).await?;
            }
        }
    }
    rebuilt(inner).await
}

/// Web `ignoreCluster`: a nameless hidden `cgroup` holding the cluster.
async fn ignore_cluster(inner: &Inner, cluster_id: &str) -> Result<String> {
    entity::sync(inner, CGROUP).await?;
    let mut saved = load_saved(inner)?;
    let cluster = take_local(&mut saved, cluster_id)?;
    let id = entity::create(
        inner,
        CGROUP,
        &CGroupData::new("", cluster, true).to_value(),
    )
    .await?;
    save_saved(inner, &saved)?;
    Ok(id)
}

pub(crate) async fn set_person_cover(inner: &Inner, person_id: &str, face_id: &str) -> Result<()> {
    let Target::CGroup(id) = target(person_id)? else {
        return Err(CoreError::storage("name this person first"));
    };
    let _g = inner.ml.faces.op_lock.lock().await;
    let mut d = fresh_cgroup(inner, &id).await?;
    let mine = d
        .effective_clusters()
        .iter()
        .any(|c| c.faces.iter().any(|f| f == face_id));
    if !mine {
        return Err(CoreError::storage("that face is not this person's"));
    }
    d.set_avatar_face_id(face_id);
    entity::update(inner, CGROUP, &id, &d.to_value()).await?;
    rebuilt(inner).await
}

pub(crate) async fn set_person_birth_date(
    inner: &Inner,
    person_id: &str,
    birth_date: Option<&str>,
) -> Result<()> {
    let Target::CGroup(id) = target(person_id)? else {
        return Err(CoreError::storage("name this person first"));
    };
    let date = birth_date.map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = date
        && !is_valid_birth_date(s)
    {
        return Err(CoreError::storage("the birth date must be YYYY-MM-DD"));
    }
    let _g = inner.ml.faces.op_lock.lock().await;
    let mut d = fresh_cgroup(inner, &id).await?;
    d.set_birth_date(date);
    entity::update(inner, CGROUP, &id, &d.to_value()).await?;
    rebuilt(inner).await
}

pub(crate) async fn remove_faces_from_person(
    inner: &Inner,
    person_id: &str,
    face_ids: &[String],
) -> Result<()> {
    let t = target(person_id)?;
    let remove: HashSet<&str> = face_ids.iter().map(String::as_str).collect();
    if remove.is_empty() {
        return Ok(());
    }
    let _g = inner.ml.faces.op_lock.lock().await;
    match t {
        Target::CGroup(id) => {
            // Mobile `removeFacesFromPerson`: out of the assigned clusters
            // (emptied clusters dropped), into rejectedFaceIDs.
            let mut d = fresh_cgroup(inner, &id).await?;
            let mut assigned = d.assigned();
            let mut removed: Vec<String> = Vec::new();
            for c in &mut assigned {
                c.faces.retain(|f| {
                    let r = remove.contains(f.as_str());
                    if r {
                        removed.push(f.clone());
                    }
                    !r
                });
            }
            assigned.retain(|c| !c.faces.is_empty());
            d.set_assigned(assigned);
            let mut rejected = d.rejected_face_ids();
            for f in face_ids {
                if !rejected.contains(f) {
                    rejected.push(f.clone());
                }
            }
            d.set_rejected_face_ids(rejected);
            entity::update(inner, CGROUP, &id, &d.to_value()).await?;
            // Locally the faces get their own cluster, rejected for this
            // person ("not person" feedback), so they are not suggested.
            removed.sort();
            removed.dedup();
            if !removed.is_empty() {
                let mut saved = load_saved(inner)?;
                for c in &mut saved.clusters {
                    c.faces.retain(|f| !remove.contains(f.as_str()));
                }
                saved.clusters.retain(|c| !c.faces.is_empty());
                let cid = new_cluster_id();
                saved.clusters.push(FaceCluster {
                    id: cid.clone(),
                    faces: removed,
                });
                reject(&mut saved, &id, &cid);
                save_saved(inner, &saved)?;
            }
        }
        Target::Cluster(cid) => {
            // Mobile `removeFilesFromCluster`: the faces leave for a new
            // cluster.
            let mut saved = load_saved(inner)?;
            let c = saved
                .clusters
                .iter_mut()
                .find(|c| c.id == cid)
                .ok_or_else(|| CoreError::storage(format!("unknown cluster {cid}")))?;
            let mut moved = Vec::new();
            c.faces.retain(|f| {
                let r = remove.contains(f.as_str());
                if r {
                    moved.push(f.clone());
                }
                !r
            });
            saved.clusters.retain(|c| !c.faces.is_empty());
            if !moved.is_empty() {
                saved.clusters.push(FaceCluster {
                    id: new_cluster_id(),
                    faces: moved,
                });
            }
            save_saved(inner, &saved)?;
        }
    }
    rebuilt(inner).await
}

pub(crate) async fn delete_person(inner: &Inner, person_id: &str) -> Result<()> {
    match target(person_id)? {
        Target::CGroup(id) => {
            let _g = inner.ml.faces.op_lock.lock().await;
            let d = fresh_cgroup(inner, &id).await?;
            entity::delete(inner, CGROUP, &id).await?;
            // The faces become unnamed clusters again.
            let mut saved = load_saved(inner)?;
            restore_local(&mut saved, d.effective_clusters());
            saved.rejected.remove(&id);
            save_saved(inner, &saved)?;
            rebuilt(inner).await
        }
        // An unnamed cluster cannot be deleted (its faces would just cluster
        // again): it is ignored, as Ente does.
        Target::Cluster(cid) => {
            let _g = inner.ml.faces.op_lock.lock().await;
            ignore_cluster(inner, &cid).await?;
            rebuilt(inner).await
        }
    }
}
