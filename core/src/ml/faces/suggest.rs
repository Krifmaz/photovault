//! "Review suggestions": unnamed clusters that look like a person, ported
//! from the web client's `_suggestionsAndChoicesForPerson`
//! (`web/packages/new/photos/services/ml/people.ts`).
//!
//! For each candidate cluster, up to 50 of its faces are compared with up
//! to 50 of the person's faces; the median cosine similarity ranks it.
//! Clusters with two or more faces are suggested above a median of 0.48,
//! else above 0.46; only when neither finds any, single-face clusters above
//! 0.48. Clusters the user rejected for the person are never suggested. At
//! most 80 suggestions.
//!
//! Two differences, both for speed on large libraries, neither changing
//! the thresholds:
//!
//! - Samples are chosen deterministically (by a hash of the face id)
//!   instead of at random, so results are stable between calls.
//! - The mean of all pairwise similarities equals the dot product of the
//!   two samples' mean embeddings, which is cheap and cached per cluster.
//!   Clusters whose mean similarity is more than [`PREFILTER_MARGIN`]
//!   below the threshold are not scored (their median cannot plausibly
//!   reach it for face embeddings); the rest get Ente's exact median.
//!
//! Suggestions are computed during the rebuild (off the UI thread) for the
//! visible named people; `person_suggestions` reads them.
//!
//! Based on Ente (ente.com).

use super::CLUSTER_PREFIX;
use super::cgroup::{CGroupData, FaceCluster};
use super::cluster::dot;
use super::state::{FacesState, Saved, load_embeddings};
use crate::client::Inner;
use std::collections::{HashMap, HashSet};

/// Faces sampled per side.
const SAMPLE: usize = 50;
/// Median similarity for a suggestion.
const STRICT: f32 = 0.48;
/// Fallback median similarity (multi-face clusters only).
const RELAXED: f32 = 0.46;
/// Most suggestions returned.
const MAX_SUGGESTIONS: usize = 80;
/// See the module docs.
pub(crate) const PREFILTER_MARGIN: f32 = 0.2;
/// Faces whose embeddings are read at once when filling the mean cache.
const LOAD_CHUNK: usize = 4096;

/// Cached mean of a cluster's sample.
#[derive(Debug, Clone)]
pub(crate) struct SampleMean {
    /// Fingerprint of the sampled face ids.
    key: u64,
    mean: Vec<f32>,
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Up to [`SAMPLE`] faces, chosen by hash.
pub(crate) fn sample<'a>(faces: impl Iterator<Item = &'a String>) -> Vec<&'a str> {
    let mut v: Vec<(u64, &str)> = faces.map(|f| (fnv(f), f.as_str())).collect();
    v.sort_unstable();
    v.dedup();
    v.truncate(SAMPLE);
    v.into_iter().map(|(_, f)| f).collect()
}

fn fingerprint(sample: &[&str]) -> u64 {
    sample
        .iter()
        .fold(sample.len() as u64, |h, f| h.rotate_left(5) ^ fnv(f))
}

fn mean(embs: impl Iterator<Item = Vec<f32>>) -> Option<Vec<f32>> {
    let mut sum: Option<Vec<f32>> = None;
    let mut n = 0f32;
    for e in embs {
        match &mut sum {
            Some(s) if s.len() == e.len() => s.iter_mut().zip(&e).for_each(|(a, b)| *a += b),
            Some(_) => continue,
            None => sum = Some(e),
        }
        n += 1.0;
    }
    let mut s = sum?;
    s.iter_mut().for_each(|x| *x /= n);
    Some(s)
}

/// Median pairwise similarity, as Ente takes it (`csims[floor(n / 2)]` of
/// the sorted list).
pub(crate) fn median_similarity(a: &[Vec<f32>], b: &[Vec<f32>]) -> Option<f32> {
    let mut sims: Vec<f32> = Vec::with_capacity(a.len() * b.len());
    for x in b {
        for y in a {
            sims.push(dot(y, x));
        }
    }
    if sims.is_empty() {
        return None;
    }
    sims.sort_by(f32::total_cmp);
    Some(sims[sims.len() / 2])
}

/// Means of the samples of `clusters`, from the cache or computed.
fn cluster_means<'a>(inner: &Inner, clusters: &[&'a FaceCluster]) -> HashMap<&'a str, Vec<f32>> {
    let cache = &inner.ml.faces;
    let samples: Vec<(&FaceCluster, Vec<&str>, u64)> = clusters
        .iter()
        .map(|c| {
            let s = sample(c.faces.iter());
            let k = fingerprint(&s);
            (*c, s, k)
        })
        .collect();
    let mut out = HashMap::new();
    let mut missing: Vec<(&FaceCluster, Vec<&str>, u64)> = Vec::new();
    {
        let means = cache.means.lock().unwrap_or_else(|e| e.into_inner());
        for (c, s, k) in samples {
            match means.get(&c.id) {
                Some(m) if m.key == k => {
                    out.insert(c.id.as_str(), m.mean.clone());
                }
                _ => missing.push((c, s, k)),
            }
        }
    }
    // Fill the cache in chunks of faces (bounded memory).
    let mut start = 0;
    while start < missing.len() {
        let mut end = start;
        let mut n = 0;
        while end < missing.len() && (n == 0 || n + missing[end].1.len() <= LOAD_CHUNK) {
            n += missing[end].1.len();
            end += 1;
        }
        let ids: Vec<&str> = missing[start..end]
            .iter()
            .flat_map(|m| m.1.iter().copied())
            .collect();
        let mut embs = load_embeddings(inner, &ids).into_iter();
        let mut computed = Vec::new();
        for (c, s, k) in &missing[start..end] {
            let e: Vec<Vec<f32>> = embs.by_ref().take(s.len()).flatten().collect();
            if let Some(m) = mean(e.into_iter()) {
                computed.push((*c, *k, m));
            }
        }
        let mut means = cache.means.lock().unwrap_or_else(|e| e.into_inner());
        for (c, key, m) in computed {
            out.insert(c.id.as_str(), m.clone());
            means.insert(c.id.clone(), SampleMean { key, mean: m });
        }
        start = end;
    }
    out
}

/// Suggestions for every visible named person.
pub(crate) fn compute(
    inner: &Inner,
    st: &FacesState,
    cgroups: &[(String, CGroupData)],
    saved: &Saved,
) -> HashMap<String, Vec<String>> {
    let targets: Vec<&(String, CGroupData)> = cgroups
        .iter()
        .filter(|(id, d)| {
            st.person(id)
                .is_some_and(|p| !p.person.is_hidden && d.name().is_some())
        })
        .collect();
    let mut out = HashMap::new();
    if targets.is_empty() || saved.clusters.is_empty() {
        return out;
    }
    // Forget means of clusters that are gone.
    {
        let live: HashSet<&str> = saved.clusters.iter().map(|c| c.id.as_str()).collect();
        let mut means = inner
            .ml
            .faces
            .means
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        means.retain(|k, _| live.contains(k.as_str()));
    }
    let all: Vec<&FaceCluster> = saved.clusters.iter().collect();
    let means = cluster_means(inner, &all);
    for (id, d) in targets {
        let clusters = d.effective_clusters();
        let person_sample = sample(clusters.iter().flat_map(|c| c.faces.iter()));
        let person: Vec<Vec<f32>> = load_embeddings(inner, &person_sample)
            .into_iter()
            .flatten()
            .collect();
        let Some(pmean) = mean(person.iter().cloned()) else {
            continue;
        };
        let rejected: HashSet<&str> = saved
            .rejected
            .get(id)
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
        let candidates = |multi: bool, threshold: f32| -> Vec<&FaceCluster> {
            let mut scored: Vec<(&FaceCluster, f32)> = Vec::new();
            for c in &saved.clusters {
                if rejected.contains(c.id.as_str()) || (c.faces.len() >= 2) != multi {
                    continue;
                }
                let Some(m) = means.get(c.id.as_str()) else {
                    continue;
                };
                if dot(&pmean, m) < threshold - PREFILTER_MARGIN {
                    continue;
                }
                let s = sample(c.faces.iter());
                let embs: Vec<Vec<f32>> =
                    load_embeddings(inner, &s).into_iter().flatten().collect();
                if let Some(med) = median_similarity(&person, &embs)
                    && med > threshold
                {
                    scored.push((c, med));
                }
            }
            scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
            scored.into_iter().map(|(c, _)| c).collect()
        };
        let mut found = candidates(true, STRICT);
        if found.is_empty() {
            found = candidates(true, RELAXED);
        }
        if found.is_empty() {
            found = candidates(false, STRICT);
        }
        let ids: Vec<String> = found
            .into_iter()
            .map(|c| format!("{CLUSTER_PREFIX}{}", c.id))
            .filter(|pid| st.person(pid).is_some())
            .take(MAX_SUGGESTIONS)
            .collect();
        if !ids.is_empty() {
            out.insert(id.clone(), ids);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_and_sampling() {
        let a = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let b = vec![vec![1.0, 0.0]];
        // sims: 1, 0 → sorted [0, 1], median index 1 → 1.
        assert_eq!(median_similarity(&a, &b), Some(1.0));
        assert_eq!(median_similarity(&a, &[]), None);
        let faces: Vec<String> = (0..120).map(|i| format!("{i}_x")).collect();
        let s1 = sample(faces.iter());
        let s2 = sample(faces.iter().rev());
        assert_eq!(s1.len(), SAMPLE);
        assert_eq!(s1, s2);
        assert_eq!(
            mean([vec![1.0, 3.0], vec![3.0, 1.0]].into_iter()),
            Some(vec![2.0, 2.0])
        );
    }
}
