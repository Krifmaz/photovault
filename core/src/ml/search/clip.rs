//! Magic search scoring: the files' CLIP image embeddings in memory, scored
//! against a CLIP text embedding.
//!
//! Like Ente's apps the score is the dot product of the two normalised
//! embeddings (cosine similarity), a file matches when it scores at least
//! the threshold (0.175 by default: mobile
//! `SemanticSearchService.kMinimumSimilarityThreshold`, web `clip.ts`), and
//! matches are ranked best first.
//!
//! To keep 100k files within 64 MB the matrix holds each embedding as
//! int8 with a per-row scale (512 bytes a row instead of 2 KB for f32) and
//! the norm of the row's quantisation error. By Cauchy-Schwarz the exact
//! score lies within `error_norm * |text|` of the approximate one, so rows
//! clearly above or below the threshold are decided in memory, and only
//! rows near it, plus the head of the ranking, are re-scored exactly from
//! the stored `mldata` ([`Rescorer`]).
//!
//! Based on Ente (ente.com).

/// Ente's default magic search threshold.
pub(crate) const MIN_SIMILARITY: f32 = 0.175;
/// How many of the best matches are re-scored exactly for the ranking.
pub(crate) const EXACT_HEAD: usize = 100;
/// At most this many rows near the threshold are re-scored exactly; beyond
/// it the approximate score decides (its error is far below CLIP's noise).
pub(crate) const EXACT_BORDER_MAX: usize = 2048;

/// Image embeddings, quantised.
#[derive(Debug, Default)]
pub(crate) struct ClipMatrix {
    dim: usize,
    ids: Vec<i64>,
    scales: Vec<f32>,
    /// Upper bound of the L2 norm of `embedding - scale * q`.
    errors: Vec<f32>,
    q: Vec<i8>,
}

impl ClipMatrix {
    /// An empty matrix with room for `rows` rows of `dim` values (the
    /// exact size avoids a growth spike while building).
    pub(crate) fn with_capacity(rows: usize, dim: usize) -> Self {
        Self {
            dim,
            ids: Vec::with_capacity(rows),
            scales: Vec::with_capacity(rows),
            errors: Vec::with_capacity(rows),
            q: Vec::with_capacity(rows * dim),
        }
    }

    /// Rows.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether there are no rows.
    pub(crate) fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// File ids, row order.
    #[cfg(test)]
    pub(crate) fn ids(&self) -> &[i64] {
        &self.ids
    }

    /// Heap bytes held.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.ids.capacity() * 8
            + self.scales.capacity() * 4
            + self.errors.capacity() * 4
            + self.q.capacity()
    }

    /// Heap bytes `rows` rows of `dim` values take.
    #[cfg(test)]
    pub(crate) const fn bytes_for(rows: usize, dim: usize) -> usize {
        rows * (8 + 4 + 4 + dim)
    }

    /// Add a file's embedding. Rows of another dimension than the first
    /// one, empty or non-finite ones are skipped (returns `false`).
    pub(crate) fn push(&mut self, file_id: i64, embedding: &[f64]) -> bool {
        if embedding.is_empty() || embedding.iter().any(|v| !v.is_finite()) {
            return false;
        }
        if self.dim == 0 {
            self.dim = embedding.len();
        }
        if embedding.len() != self.dim {
            return false;
        }
        let max = embedding.iter().fold(0f64, |m, v| m.max(v.abs()));
        let scale = (max / 127.0) as f32;
        let mut err2 = 0f64;
        for &v in embedding {
            let q = if scale > 0.0 {
                (v / f64::from(scale)).round().clamp(-127.0, 127.0)
            } else {
                0.0
            };
            let d = v - q * f64::from(scale);
            err2 += d * d;
            self.q.push(q as i8);
        }
        self.ids.push(file_id);
        self.scales.push(scale);
        // Round up so the bound holds after the cast to f32.
        self.errors.push((err2.sqrt() * (1.0 + 1e-6) + 1e-9) as f32);
        true
    }

    /// Approximate score of every row and its error bound for a text
    /// embedding `t` (of [`Self::dim`] values): `(row, score, bound)`.
    fn scan(&self, t: &[f32]) -> impl Iterator<Item = (usize, f32, f32)> + '_ {
        let t: Vec<f64> = t.iter().map(|v| f64::from(*v)).collect();
        let t_norm = t.iter().map(|v| v * v).sum::<f64>().sqrt();
        // f64 accumulation: rounding (and the final cast to f32) stays far
        // below this slack.
        let slack = 1e-6 * (1.0 + t_norm);
        self.q
            .chunks_exact(self.dim.max(1))
            .enumerate()
            .map(move |(i, row)| {
                let mut acc = 0f64;
                for (q, x) in row.iter().zip(&t) {
                    acc += f64::from(*q) * x;
                }
                let score = acc * f64::from(self.scales[i]);
                let bound = f64::from(self.errors[i]) * t_norm + slack;
                (i, score as f32, bound as f32)
            })
    }

    /// Files scoring at least `threshold` against `t`, best first.
    /// `exact` re-scores the given file ids from their stored embeddings
    /// (missing ids keep the approximate score).
    pub(crate) fn matches(
        &self,
        t: &[f32],
        threshold: f32,
        exact: &mut dyn Rescorer,
    ) -> Vec<(i64, f32)> {
        if self.is_empty() || t.len() != self.dim {
            return Vec::new();
        }
        let mut sure: Vec<(usize, f32)> = Vec::new();
        let mut border: Vec<(usize, f32)> = Vec::new();
        for (i, s, b) in self.scan(t) {
            if s - b >= threshold {
                sure.push((i, s));
            } else if s + b >= threshold {
                border.push((i, s));
            }
        }
        let by_score = |a: &(usize, f32), b: &(usize, f32)| b.1.total_cmp(&a.1);
        sure.sort_by(by_score);
        border.sort_by(by_score);
        // Rows to re-score: the head of the ranking and those near the
        // threshold (closest first).
        let head: Vec<usize> = sure.iter().take(EXACT_HEAD).map(|r| r.0).collect();
        let mut near: Vec<(usize, f32)> = border.clone();
        near.sort_by(|a, b| (a.1 - threshold).abs().total_cmp(&(b.1 - threshold).abs()));
        near.truncate(EXACT_BORDER_MAX);
        let want: Vec<i64> = head
            .iter()
            .copied()
            .chain(near.iter().map(|r| r.0))
            .map(|i| self.ids[i])
            .collect();
        let exact_scores = exact.rescore(&want, t);
        let score_of = |i: usize, approx: f32| -> (f32, bool) {
            exact_scores
                .get(&self.ids[i])
                .map_or((approx, false), |s| (*s, true))
        };
        let mut out: Vec<(i64, f32)> = Vec::with_capacity(sure.len() + border.len());
        for (i, s) in sure {
            out.push((self.ids[i], score_of(i, s).0));
        }
        for (i, s) in border {
            let (score, _) = score_of(i, s);
            if score >= threshold {
                out.push((self.ids[i], score));
            }
        }
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }
}

/// Exact scores from the stored embeddings.
pub(crate) trait Rescorer {
    /// Exact dot products of the files' stored embeddings with `t`; ids
    /// without a stored embedding are left out.
    fn rescore(&mut self, file_ids: &[i64], t: &[f32]) -> std::collections::HashMap<i64, f32>;
}

/// Exact dot product (f64 accumulation, like the apps' double math).
pub(crate) fn dot(a: &[f64], t: &[f32]) -> f32 {
    a.iter().zip(t).map(|(x, y)| x * f64::from(*y)).sum::<f64>() as f32
}

// ---------------------------------------------------------------- prompts

/// A discovery prompt (mobile `assets/discover.json`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Prompt {
    /// The CLIP query.
    pub query: &'static str,
    /// The label.
    pub title: &'static str,
    /// Threshold.
    pub min_score: f32,
    /// Newest first instead of best first.
    pub recent_first: bool,
    /// Include videos.
    pub show_video: bool,
}

const fn prompt(
    query: &'static str,
    title: &'static str,
    min_score: f32,
    recent_first: bool,
    show_video: bool,
) -> Prompt {
    Prompt {
        query,
        title,
        min_score,
        recent_first,
        show_video,
    }
}

/// Ente's discovery prompts, in its order.
pub(crate) const PROMPTS: &[Prompt] = &[
    prompt(
        "identity documents and cards",
        "Identity",
        0.20,
        false,
        false,
    ),
    prompt("screenshots", "Screenshots", 0.18, true, false),
    prompt(
        "image containing a black and white QR code",
        "QR Codes",
        0.20,
        true,
        false,
    ),
    prompt("receipts, bills and invoices", "Receipts", 0.20, true, true),
    prompt(
        "screenshots of digital notes, photos of notebook and whiteboards",
        "Notes",
        0.20,
        true,
        true,
    ),
    prompt("memes", "Memes", 0.18, true, true),
    prompt("business card", "Visiting Cards", 0.20, true, false),
    prompt("photo of toddler", "Babies", 0.20, true, true),
    prompt("photos of pets", "Pets", 0.20, true, true),
    prompt("selfie", "Selfies", 0.18, true, true),
    prompt(
        "high resolution hd wallpaper",
        "Wallpapers",
        0.18,
        false,
        false,
    ),
    prompt("photo of food", "Food", 0.20, true, true),
    prompt("celebration with friends", "Celebrations", 0.20, true, true),
    prompt("photo of setting sun", "Sunset", 0.20, false, true),
    prompt("photo of mountains or hills", "Hills", 0.20, false, true),
    prompt(
        "outdoor photo of trees, leaves, flowers",
        "Greenery",
        0.20,
        false,
        true,
    ),
];

/// The prompt whose query is `q`, if any.
pub(crate) fn prompt_for(q: &str) -> Option<&'static Prompt> {
    PROMPTS.iter().find(|p| p.query == q)
}

/// Ente's debug syntax `0.25 query`: a leading threshold overrides the
/// default (`SemanticSearchService.getMatchingFiles`).
pub(crate) fn split_threshold(query: &str) -> (Option<f32>, &str) {
    if let Some((first, rest)) = query.split_once(' ')
        && first.starts_with("0.")
        && first.len() > 2
        && first[2..].starts_with(|c: char| c.is_ascii_digit())
        && let Ok(t) = first.parse::<f32>()
    {
        return (Some(t), rest);
    }
    (None, query)
}

// ---------------------------------------------------------------- text model

/// The CLIP text model and its vocabulary, by the exact file names Ente's
/// catalog pins (`rust/crates/ml/src/assets.rs`, `Model::ClipText`), with
/// their sizes. They live in `MlState::models_dir`.
pub(crate) const TEXT_MODEL: (&str, u64) = ("mobileclip_s2_text_opset18_quant.onnx", 67_144_712);
/// See [`TEXT_MODEL`].
pub(crate) const TEXT_VOCAB: (&str, u64) = ("bpe_simple_vocab_16e6.txt", 3_194_984);

/// Paths of the text model and vocabulary when both are fully downloaded.
pub(crate) fn text_model_paths(
    models_dir: &std::path::Path,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let ok = |(name, size): (&str, u64)| {
        let p = models_dir.join(name);
        std::fs::metadata(&p)
            .ok()
            .filter(|m| m.is_file() && m.len() == size)
            .map(|_| p)
    };
    Some((ok(TEXT_MODEL)?, ok(TEXT_VOCAB)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Exact scores from a map of embeddings.
    struct Exact(HashMap<i64, Vec<f64>>, usize);
    impl Rescorer for Exact {
        fn rescore(&mut self, ids: &[i64], t: &[f32]) -> HashMap<i64, f32> {
            self.1 += ids.len();
            ids.iter()
                .filter_map(|id| self.0.get(id).map(|e| (*id, dot(e, t))))
                .collect()
        }
    }

    /// A deterministic pseudo-random unit vector.
    fn unit(seed: u64, dim: usize) -> Vec<f64> {
        let mut s = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let mut v: Vec<f64> = (0..dim)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                ((s >> 33) as f64 / (1u64 << 31) as f64) - 0.5
            })
            .collect();
        let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        v.iter_mut().for_each(|x| *x /= n);
        v
    }

    /// `base` mixed with noise to reach about `cos` similarity to it.
    fn near(base: &[f64], cos: f64, seed: u64) -> Vec<f64> {
        let n = unit(seed, base.len());
        // Remove the component along base, then mix.
        let d: f64 = n.iter().zip(base).map(|(a, b)| a * b).sum();
        let mut o: Vec<f64> = n.iter().zip(base).map(|(a, b)| a - d * b).collect();
        let on = o.iter().map(|x| x * x).sum::<f64>().sqrt();
        o.iter_mut().for_each(|x| *x /= on);
        let s = (1.0 - cos * cos).sqrt();
        base.iter().zip(&o).map(|(b, x)| cos * b + s * x).collect()
    }

    #[test]
    fn threshold_and_order_are_exact() {
        let dim = 512;
        let text = unit(1, dim);
        let t: Vec<f32> = text.iter().map(|v| *v as f32).collect();
        let mut m = ClipMatrix::default();
        let mut exact = HashMap::new();
        // Scores spread around the threshold, some within 1e-4 of it.
        let wanted = [
            0.9, 0.5, 0.3, 0.2, 0.176, 0.17505, 0.17495, 0.174, 0.1, 0.0, -0.2,
        ];
        for (i, c) in wanted.iter().enumerate() {
            let e = near(&text, *c, 100 + i as u64);
            assert!(m.push(i as i64, &e));
            exact.insert(i as i64, e);
        }
        // Plus many unrelated files.
        for i in 100..2100 {
            let e = unit(i, dim);
            m.push(i as i64, &e);
            exact.insert(i as i64, e);
        }
        let mut r = Exact(exact.clone(), 0);
        let got = m.matches(&t, MIN_SIMILARITY, &mut r);
        let mut want: Vec<(i64, f32)> = exact
            .iter()
            .map(|(id, e)| (*id, dot(e, &t)))
            .filter(|(_, s)| *s >= MIN_SIMILARITY)
            .collect();
        want.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        assert_eq!(
            got.iter().map(|g| g.0).collect::<Vec<_>>(),
            want.iter().map(|g| g.0).collect::<Vec<_>>()
        );
        assert_eq!(
            &got.iter().map(|g| g.0).take(6).collect::<Vec<_>>(),
            &[0, 1, 2, 3, 4, 5]
        );
        for (g, w) in got.iter().zip(&want) {
            assert!((g.1 - w.1).abs() < 1e-5, "{g:?} vs {w:?}");
        }
        // Only the head and the rows near the threshold were re-scored.
        assert!(r.1 < 200, "re-scored {}", r.1);
        // A higher threshold, a wrong dimension.
        let got = m.matches(&t, 0.4, &mut Exact(exact, 0));
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), vec![0, 1]);
        assert!(
            m.matches(&t[..10], 0.1, &mut Exact(HashMap::new(), 0))
                .is_empty()
        );
    }

    #[test]
    fn bound_holds_without_rescoring() {
        let dim = 512;
        let text = unit(7, dim);
        let t: Vec<f32> = text.iter().map(|v| *v as f32).collect();
        let mut m = ClipMatrix::default();
        let mut es = Vec::new();
        for i in 0..500 {
            let e = near(&text, (i as f64) / 500.0 - 0.2, 1000 + i);
            m.push(i as i64, &e);
            es.push(e);
        }
        for (i, s, b) in m.scan(&t) {
            let x = dot(&es[i], &t);
            assert!((s - x).abs() <= b, "row {i}: {s} vs {x}, bound {b}");
            assert!(b < 0.02);
        }
    }

    #[test]
    fn rows_are_validated() {
        let mut m = ClipMatrix::default();
        assert!(!m.push(1, &[]));
        assert!(!m.push(1, &[f64::NAN, 1.0]));
        assert!(m.push(1, &[0.6, 0.8]));
        assert!(!m.push(2, &[1.0]));
        assert!(m.push(3, &[0.0, 0.0]));
        assert_eq!(m.len(), 2);
        assert_eq!(m.ids(), &[1, 3]);
    }

    #[test]
    fn a_100k_library_fits_in_64_mb() {
        let (rows, dim) = (100_000, 512);
        assert!(ClipMatrix::bytes_for(rows, dim) <= 64 * 1024 * 1024);
        let mut m = ClipMatrix::with_capacity(rows, dim);
        let e = unit(3, dim);
        for i in 0..rows {
            m.push(i as i64, &e);
        }
        assert_eq!(m.len(), rows);
        assert_eq!(m.bytes(), ClipMatrix::bytes_for(rows, dim));
        assert!(m.bytes() <= 64 * 1024 * 1024, "{} bytes", m.bytes());
    }

    #[test]
    fn prompts_and_threshold_syntax() {
        assert_eq!(PROMPTS.len(), 16);
        assert_eq!(prompt_for("photo of food").map(|p| p.title), Some("Food"));
        assert_eq!(split_threshold("0.25 beach"), (Some(0.25), "beach"));
        assert_eq!(split_threshold("0. beach"), (None, "0. beach"));
        assert_eq!(split_threshold("beach at night"), (None, "beach at night"));
        assert_eq!(split_threshold("0.3"), (None, "0.3"));
    }
}
