//! Search: magic search (MobileCLIP text embedding vs the files' image
//! embeddings, Ente's thresholds) and the metadata search of Ente's apps
//! (file names, captions, albums, dates, people, file types, places).
//!
//! STUB (contract v1.2): owned by the `core-search` track.
//!
//! Internal API for `memories`: [`invalidate`], [`clip_matches`].

use super::{SearchKind, SearchSuggestion};
use crate::client::Inner;
use crate::{Core, CoreError, TimelineItem};

/// Search caches (CLIP matrix, text index), rebuilt lazily after
/// [`invalidate`].
#[derive(Debug, Default)]
pub(crate) struct SearchCache {}

/// Drop the caches (library or `mldata` changed).
pub(crate) fn invalidate(inner: &Inner) {
    let _ = inner;
}

/// Files whose CLIP image embedding scores at least `threshold` against a
/// text embedding, best first: `(file_id, score)`.
pub(crate) fn clip_matches(
    inner: &Inner,
    text_embedding: &[f32],
    threshold: f32,
) -> Vec<(i64, f32)> {
    let _ = (inner, text_embedding, threshold);
    Vec::new()
}

#[uniffi::export]
impl Core {
    /// Suggestions for what the user typed, best first (fast, in memory).
    pub fn search_suggestions(&self, query: String, limit: u32) -> Vec<SearchSuggestion> {
        let _ = (query, limit);
        Vec::new()
    }

    /// What the search screen shows before typing: people, places, file
    /// types, recent albums and magic-search prompts.
    pub fn search_discovery(&self) -> Vec<SearchSuggestion> {
        Vec::new()
    }

    /// The files of a suggestion, newest first (magic: best match first).
    pub async fn search_results(
        &self,
        kind: SearchKind,
        key: String,
    ) -> Result<Vec<TimelineItem>, CoreError> {
        let _ = (kind, key);
        Ok(Vec::new())
    }

    /// Magic search: files matching a description, best first. Empty when
    /// the text model is not available.
    pub async fn magic_search(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<TimelineItem>, CoreError> {
        let _ = (query, limit);
        Ok(Vec::new())
    }
}
