//! Ente's face clustering, ported from the web/desktop client
//! (`web/packages/new/photos/services/ml/cluster.ts`, `face.ts`).
//!
//! Why the web client: its algorithm is the documented, deterministic
//! reference that runs over the shared `mldata`, and its people model
//! (local clusters + `cgroup` entities) is the one the contract uses. The
//! mobile app (`face_clustering_service.dart`) clusters with a different
//! linear pass and distance thresholds over its own local DB; both produce
//! `cgroup`s in the same format, so either app reads ours.
//!
//! - Faces enter clustering only if `blur > 10 && score > 0.8`.
//! - A face is "bad" (`blur < 50`, or `blur < 200 && score < 0.85`, or not
//!   looking straight) and then needs a cosine similarity of 0.84 to be
//!   joined, else 0.76.
//! - Faces are processed newest first, in overlapping batches of 10 000
//!   (step 7 500); in a batch the already clustered faces come first, and
//!   each new face joins the cluster of its most similar earlier face
//!   above the threshold (never a cluster its `cgroup` rejected it from),
//!   or starts a new cluster.
//! - Remote (`cgroup`) clusters come first, newest `cgroup` first, minus
//!   their rejected faces; the first assignment of a face wins.
//!
//! Embeddings are loaded per batch through a callback, so memory stays at
//! one batch (10 000 × 192 floats) however large the library is.
//!
//! Based on Ente (ente.com).

use super::cgroup::FaceCluster;
use std::collections::{HashMap, HashSet};

/// Faces below this blur variance are not clustered.
pub const MIN_BLUR: f64 = 10.0;
/// Faces at or below this detection score are not clustered.
pub const MIN_SCORE: f64 = 0.8;
/// Similarity needed to join a good face.
pub const GOOD_FACE_THRESHOLD: f32 = 0.76;
/// Similarity needed to join a bad (blurry, low score, sideways) face.
pub const BAD_FACE_THRESHOLD: f32 = 0.84;
/// Faces per batch.
pub const BATCH_SIZE: usize = 10_000;
/// Batch step (batches overlap so clusters link across boundaries).
pub const OFFSET_INCREMENT: usize = 7_500;

/// Whether a face takes part in clustering (`enumerateFaces`).
#[must_use]
pub fn is_clusterable(score: f64, blur: f64) -> bool {
    blur > MIN_BLUR && score > MIN_SCORE
}

/// Where a face looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceDirection {
    /// To the left.
    Left,
    /// To the right.
    Right,
    /// Straight at the camera.
    Straight,
}

/// Ente's `faceDirection` from the five landmarks (left eye, right eye,
/// nose, left and right mouth corner). Faces with fewer landmarks count as
/// straight.
#[must_use]
pub fn face_direction(landmarks: &[(f64, f64)]) -> FaceDirection {
    let [le, re, nose, lm, rm] = match landmarks {
        [a, b, c, d, e, ..] => [*a, *b, *c, *d, *e],
        _ => return FaceDirection::Straight,
    };
    let eye_dx = (re.0 - le.0).abs();
    let eye_dy = (re.1 - le.1).abs();
    let mouth_dy = (rm.1 - lm.1).abs();
    let upright =
        le.1.max(re.1) + 0.5 * eye_dy < nose.1 && nose.1 + 0.5 * mouth_dy < lm.1.min(rm.1);
    let out_left = nose.0 < le.0.min(re.0) && nose.0 < lm.0.min(rm.0);
    let out_right = nose.0 > le.0.max(re.0) && nose.0 > lm.0.max(rm.0);
    let close_left = (nose.0 - le.0).abs() < 0.2 * eye_dx;
    let close_right = (nose.0 - re.0).abs() < 0.2 * eye_dx;
    if out_left || (upright && close_left) {
        FaceDirection::Left
    } else if out_right || (upright && close_right) {
        FaceDirection::Right
    } else {
        FaceDirection::Straight
    }
}

/// Ente's `isBadFace`.
#[must_use]
pub fn is_bad_face(score: f64, blur: f64, landmarks: &[(f64, f64)]) -> bool {
    blur < 50.0
        || (blur < 200.0 && score < 0.85)
        || face_direction(landmarks) != FaceDirection::Straight
}

/// A face to cluster (its embedding comes from the loader).
#[derive(Debug, Clone)]
pub struct ClusterFace {
    /// Face id.
    pub face_id: String,
    /// [`is_bad_face`].
    pub is_bad: bool,
}

/// The clusters of one `cgroup`.
#[derive(Debug, Clone)]
pub struct RemoteGroup {
    /// The entity's `updatedAt` (newest wins).
    pub updated_at: i64,
    /// `assigned`.
    pub assigned: Vec<FaceCluster>,
    /// `rejectedFaceIDs`.
    pub rejected_face_ids: Vec<String>,
}

/// The result of [`cluster_faces`].
#[derive(Debug, Clone, Default)]
pub struct Clustering {
    /// Remote clusters (without rejected faces) first, then local ones.
    pub clusters: Vec<FaceCluster>,
    /// Clusters that got faces in this run.
    pub modified: HashSet<String>,
}

/// Cosine similarity of normalised embeddings.
#[inline]
#[must_use]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    // Four accumulators: lets the compiler vectorise.
    let mut acc = [0f32; 4];
    let (ca, ra) = a.split_at(a.len() - a.len() % 4);
    let (cb, rb) = b.split_at(ca.len().min(b.len()));
    for (x, y) in ca.chunks_exact(4).zip(cb.chunks_exact(4)) {
        acc[0] += x[0] * y[0];
        acc[1] += x[1] * y[1];
        acc[2] += x[2] * y[2];
        acc[3] += x[3] * y[3];
    }
    let mut s = (acc[0] + acc[1]) + (acc[2] + acc[3]);
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

/// Cluster `faces` (newest first). `embeddings(indices)` returns the
/// embeddings of `faces[i]` for the given indices (`None` when missing:
/// the face is skipped). `new_id` makes cluster ids.
pub fn cluster_faces(
    faces: &[ClusterFace],
    cgroups: &[RemoteGroup],
    local: Vec<FaceCluster>,
    mut embeddings: impl FnMut(&[usize]) -> Vec<Option<Vec<f32>>>,
    mut new_id: impl FnMut() -> String,
) -> Clustering {
    let mut sorted: Vec<&RemoteGroup> = cgroups.iter().collect();
    // A face can occur in multiple remote clusters; newest assignment wins.
    sorted.sort_by_key(|g| std::cmp::Reverse(g.updated_at));

    let mut clusters: Vec<FaceCluster> = Vec::new();
    let mut rejected_clusters_of_face: HashMap<&str, HashSet<String>> = HashMap::new();
    for g in &sorted {
        if g.rejected_face_ids.is_empty() {
            clusters.extend(g.assigned.iter().cloned());
        } else {
            let rejected: HashSet<&str> = g.rejected_face_ids.iter().map(String::as_str).collect();
            clusters.extend(g.assigned.iter().map(|c| {
                FaceCluster {
                    id: c.id.clone(),
                    faces: c
                        .faces
                        .iter()
                        .filter(|f| !rejected.contains(f.as_str()))
                        .cloned()
                        .collect(),
                }
            }));
            for f in &g.rejected_face_ids {
                let s = rejected_clusters_of_face.entry(f.as_str()).or_default();
                s.extend(g.assigned.iter().map(|c| c.id.clone()));
            }
        }
    }
    clusters.extend(local);

    // Remote assignments precede and override duplicate local assignments.
    let mut cluster_of_face: HashMap<String, usize> = HashMap::new();
    for (i, c) in clusters.iter().enumerate() {
        for f in &c.faces {
            cluster_of_face.entry(f.clone()).or_insert(i);
        }
    }

    let mut modified = HashSet::new();
    let total = faces.len();
    let mut offset = 0;
    while offset < total {
        let end = (offset + BATCH_SIZE).min(total);
        let batch = offset..end;
        let (mut order, unclustered): (Vec<usize>, Vec<usize>) = batch
            .clone()
            .partition(|&i| cluster_of_face.contains_key(&faces[i].face_id));
        // Common incremental case; avoids the O(n²) loop.
        if !unclustered.is_empty() {
            // Existing clusters must come first because matches only look
            // backward.
            order.extend(unclustered);
            let embs = embeddings(&order);
            cluster_batch(
                faces,
                &order,
                &embs,
                &mut clusters,
                &mut cluster_of_face,
                &rejected_clusters_of_face,
                &mut modified,
                &mut new_id,
            );
        }
        offset += OFFSET_INCREMENT;
    }
    Clustering { clusters, modified }
}

#[allow(clippy::too_many_arguments)]
fn cluster_batch(
    faces: &[ClusterFace],
    order: &[usize],
    embs: &[Option<Vec<f32>>],
    clusters: &mut Vec<FaceCluster>,
    cluster_of_face: &mut HashMap<String, usize>,
    rejected_clusters_of_face: &HashMap<&str, HashSet<String>>,
    modified: &mut HashSet<String>,
    new_id: &mut impl FnMut() -> String,
) {
    for (i, &fi) in order.iter().enumerate() {
        let face = &faces[fi];
        if cluster_of_face.contains_key(&face.face_id) {
            continue;
        }
        let Some(ei) = embs.get(i).and_then(Option::as_ref) else {
            continue;
        };
        let rejected = rejected_clusters_of_face.get(face.face_id.as_str());
        let mut nn: Option<usize> = None;
        let mut nn_sim = 0f32;
        for j in (0..i).rev() {
            let Some(ej) = embs[j].as_ref() else { continue };
            // Embeddings are normalised, so this is cosine similarity.
            let csim = dot(ei, ej);
            if csim <= nn_sim {
                continue;
            }
            let fj = &faces[order[j]];
            let threshold = if fj.is_bad {
                BAD_FACE_THRESHOLD
            } else {
                GOOD_FACE_THRESHOLD
            };
            if csim < threshold {
                continue;
            }
            // Never return a face to a cluster the user rejected.
            if let Some(rejected) = rejected
                && let Some(&cj) = cluster_of_face.get(&fj.face_id)
                && rejected.contains(&clusters[cj].id)
            {
                continue;
            }
            // Only faces that are in a cluster can be matched.
            if !cluster_of_face.contains_key(&fj.face_id) {
                continue;
            }
            nn = Some(j);
            nn_sim = csim;
        }
        match nn {
            Some(j) => {
                let ci = cluster_of_face[&faces[order[j]].face_id];
                cluster_of_face.insert(face.face_id.clone(), ci);
                clusters[ci].faces.push(face.face_id.clone());
                modified.insert(clusters[ci].id.clone());
            }
            None => {
                let id = new_id();
                cluster_of_face.insert(face.face_id.clone(), clusters.len());
                modified.insert(id.clone());
                clusters.push(FaceCluster {
                    id,
                    faces: vec![face.face_id.clone()],
                });
            }
        }
    }
}

/// A new cluster id like the apps make (`cluster_` + 22 nanoid characters
/// from Ente's alphabet).
#[must_use]
pub fn new_cluster_id() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut out = String::from("cluster_");
    while out.len() < 8 + 22 {
        for b in crate::crypto::random_bytes(32) {
            // Rejection sampling: no modulo bias.
            if (b as usize) < 248 && out.len() < 30 {
                out.push(ALPHABET[b as usize % 62] as char);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions() {
        let straight = [(0.3, 0.3), (0.7, 0.3), (0.5, 0.5), (0.35, 0.7), (0.65, 0.7)];
        assert_eq!(face_direction(&straight), FaceDirection::Straight);
        let left = [(0.3, 0.3), (0.7, 0.3), (0.1, 0.5), (0.35, 0.7), (0.65, 0.7)];
        assert_eq!(face_direction(&left), FaceDirection::Left);
        let right = [(0.3, 0.3), (0.7, 0.3), (0.9, 0.5), (0.35, 0.7), (0.65, 0.7)];
        assert_eq!(face_direction(&right), FaceDirection::Right);
        assert!(is_bad_face(0.9, 40.0, &straight));
        assert!(is_bad_face(0.84, 150.0, &straight));
        assert!(!is_bad_face(0.9, 150.0, &straight));
        assert!(is_bad_face(0.99, 500.0, &left));
        assert!(!is_clusterable(0.8, 100.0));
        assert!(!is_clusterable(0.9, 10.0));
        assert!(is_clusterable(0.81, 10.5));
    }

    #[test]
    fn ids() {
        crate::crypto::init();
        let a = new_cluster_id();
        assert_eq!(a.len(), 30);
        assert!(a.starts_with("cluster_"));
        assert_ne!(a, new_cluster_id());
    }

    #[test]
    fn dot_matches_naive() {
        let a: Vec<f32> = (0..193).map(|i| (i as f32).sin()).collect();
        let b: Vec<f32> = (0..193).map(|i| (i as f32).cos()).collect();
        let naive: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!((dot(&a, &b) - naive).abs() < 1e-4);
    }
}
