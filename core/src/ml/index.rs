//! ML data sync and on-device indexing.
//!
//! STUB (contract v1.2): owned by the `core-index` track.
//!
//! - Pull: `/files/data/status-diff` + `/files/data/fetch` (type `mldata`),
//!   decrypt with the file key ([`super::mldata::open`]), store
//!   ([`crate::store::Store::put_ml_data`]).
//! - Models: `GET <server>/photovault/models/<file>` (the server's mirror of
//!   Ente's model CDN), verified against the size and SHA-256 Ente pins, in
//!   [`super::MlState::models_dir`].
//! - Index (feature `ml`): Ente's `ente_ml::indexing::analyze_image`,
//!   merge into the existing object ([`super::mldata::merge`]), upload with
//!   `PUT /files/data`.
//! - After data changes: [`super::faces::rebuild`],
//!   [`super::search::invalidate`], [`super::memories::invalidate`].

use super::{MlListener, MlStatus};
use crate::{Core, CoreError};
use std::sync::Arc;

/// Indexer state kept in memory.
#[derive(Debug, Default)]
pub(crate) struct IndexerState {}

#[uniffi::export]
impl Core {
    /// Current ML state (fast; from the store and memory).
    pub fn ml_status(&self) -> MlStatus {
        MlStatus::default()
    }

    /// Turn ML on or off for this account (Ente's remote flag
    /// `faceSearchEnabled`, shared with the Ente apps).
    pub async fn set_ml_enabled(&self, enabled: bool) -> Result<MlStatus, CoreError> {
        let _ = enabled;
        Ok(MlStatus::default())
    }

    /// Pull the `mldata` and people other devices produced, then recluster.
    /// Cheap when nothing changed; call after every library sync.
    pub async fn sync_ml(
        &self,
        listener: Option<Arc<dyn MlListener>>,
    ) -> Result<MlStatus, CoreError> {
        let _ = listener;
        Ok(MlStatus::default())
    }

    /// Download the models if needed and index up to `max_files` files on
    /// this device, uploading each result. Cancel by cancelling the
    /// coroutine. Returns the final status.
    pub async fn run_ml_indexing(
        &self,
        max_files: u32,
        listener: Option<Arc<dyn MlListener>>,
    ) -> Result<MlStatus, CoreError> {
        let _ = (max_files, listener);
        Ok(MlStatus::default())
    }
}
