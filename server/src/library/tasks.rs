//! Background work for the library (museum's crons for trash, queues and object cleanup).
//!
//! [`spawn_background`] starts one task that runs [`run_once`] every
//! [`INTERVAL`]. Each pass:
//! 1. trashes the files of deleted collections (`trashCollectionV3` queue);
//! 2. processes empty-trash requests (`trashEmpty`, `trashEmptyLocker` queues);
//! 3. permanently deletes trash entries older than 30 days;
//! 4. deletes objects queued under `deleteObject` once no live `object_keys` row
//!    references them;
//! 5. deletes expired staged uploads (`temp_objects`) and their objects / multipart parts;
//! 6. deletes the objects and rows of deleted file data.
//!
//! Every step logs and swallows its own errors so one failure never stops the others.

use super::objects::{DELETE_OBJECT_QUEUE, TRASH_COLLECTION_QUEUE};
use super::sql::{db_err, get_i64, get_opt_str, get_str};
use super::{collections, filedata, trash};
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::state::AppState;
use std::time::Duration;

/// Pause between background passes.
pub const INTERVAL: Duration = Duration::from_secs(60);

/// Start the library's background task. Call once per process, after the state is built
/// (the HTTP server's `serve` path does this; tests may instead call [`run_once`]).
pub fn spawn_background(state: AppState) {
    tokio::spawn(async move {
        loop {
            run_once(&state).await;
            tokio::time::sleep(INTERVAL).await;
        }
    });
}

static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Run one pass of every background job. Concurrent calls wait for each other.
pub async fn run_once(state: &AppState) {
    let _guard = RUNNING.lock().await;
    if let Err(e) = trash_collections(state).await {
        tracing::error!(error = %e.message, "trash collection queue failed");
    }
    trash::process_empty_trash_requests(state).await;
    if let Err(e) = trash::delete_aged(state).await {
        tracing::error!(error = %e.message, "deleting aged trash failed");
    }
    if let Err(e) = delete_queued_objects(state).await {
        tracing::error!(error = %e.message, "object deletion queue failed");
    }
    if let Err(e) = delete_expired_temp_objects(state).await {
        tracing::error!(error = %e.message, "temp object cleanup failed");
    }
    if let Err(e) = filedata::process_deleted(state, 500).await {
        tracing::error!(error = %e.message, "file data cleanup failed");
    }
}

async fn trash_collections(state: &AppState) -> ApiResult<()> {
    for item in trash::pending_queue_items(state, TRASH_COLLECTION_QUEUE, 100).await? {
        let Ok(cid) = item.parse::<i64>() else {
            trash::delete_queue_item(state, TRASH_COLLECTION_QUEUE, &item).await?;
            continue;
        };
        match collections::process_trashed_collection(state, cid).await {
            Ok(()) => trash::delete_queue_item(state, TRASH_COLLECTION_QUEUE, &item).await?,
            Err(e) => tracing::error!(cid, error = %e.message, "failed to trash collection"),
        }
    }
    Ok(())
}

/// Is `key` still referenced by a live file?
async fn is_live_object(state: &AppState, key: &str) -> ApiResult<bool> {
    let r = sqlx::query(
        "SELECT COUNT(*) AS n FROM object_keys WHERE object_key = $1 AND is_deleted = FALSE",
    )
    .bind(key)
    .fetch_one(&state.db.pool)
    .await
    .map_err(db_err)?;
    Ok(get_i64(&r, "n").map_err(db_err)? > 0)
}

async fn delete_queued_objects(state: &AppState) -> ApiResult<()> {
    for key in trash::pending_queue_items(state, DELETE_OBJECT_QUEUE, 1000).await? {
        if !is_live_object(state, &key).await?
            && let Err(e) = state.store.delete(&key).await
        {
            tracing::error!(key = %key, error = %e, "failed to delete object");
            continue;
        }
        trash::delete_queue_item(state, DELETE_OBJECT_QUEUE, &key).await?;
    }
    Ok(())
}

async fn delete_expired_temp_objects(state: &AppState) -> ApiResult<()> {
    let rows = sqlx::query(
        "SELECT object_key, upload_id FROM temp_objects WHERE expiration_time <= $1 LIMIT 1000",
    )
    .bind(now_micros())
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    for r in &rows {
        let key = get_str(r, "object_key").map_err(db_err)?;
        let upload_id = get_opt_str(r, "upload_id").map_err(db_err)?;
        if !is_live_object(state, &key).await?
            && !is_file_data_object(state, &key).await?
            && let Err(e) = state.store.delete(&key).await
        {
            tracing::error!(key = %key, error = %e, "failed to delete expired upload");
            continue;
        }
        if let Some(id) = upload_id.filter(|u| !u.is_empty())
            && let Err(e) = state.store.abort_multipart(&key, &id).await
        {
            tracing::warn!(key = %key, error = %e, "failed to abort expired multipart upload");
        }
        sqlx::query("DELETE FROM temp_objects WHERE object_key = $1 AND expiration_time <= $2")
            .bind(key)
            .bind(now_micros())
            .execute(&state.db.pool)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}

/// Is `key` the registered object of a live file data row?
async fn is_file_data_object(state: &AppState, key: &str) -> ApiResult<bool> {
    // Keys look like "<owner>/file-data/<fileID>/<type>/<objectID>".
    let parts: Vec<&str> = key.split('/').collect();
    if parts.len() != 5 || parts[1] != "file-data" {
        return Ok(false);
    }
    let Ok(file_id) = parts[2].parse::<i64>() else {
        return Ok(false);
    };
    let r = sqlx::query(
        "SELECT COUNT(*) AS n FROM file_data
         WHERE file_id = $1 AND data_type = $2 AND obj_id = $3 AND is_deleted = FALSE",
    )
    .bind(file_id)
    .bind(parts[3])
    .bind(parts[4])
    .fetch_one(&state.db.pool)
    .await
    .map_err(db_err)?;
    Ok(get_i64(&r, "n").map_err(db_err)? > 0)
}
