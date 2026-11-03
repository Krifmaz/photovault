//! Trash repository (museum's `TrashRepository` / `TrashController`).
//!
//! * Trashing a file (see [`super::files::trash`]) removes every membership and inserts a
//!   `trash` row with `delete_by = now + 30 days`.
//! * Restoring (`/collections/restore-files`) adds a membership again and marks the row
//!   `is_restored`.
//! * Permanent deletion ([`delete`]) marks the row `is_deleted`, the file's `object_keys`
//!   deleted (queued for removal from the store) and its `file_data` deleted, and gives the
//!   bytes back to the user's usage.
//! * Every write to a trash row sets `updated_at = now`, which is the version used by the
//!   trash diff.

use super::collections::page_by_version;
use super::errors;
use super::files::{TrashItem, bind_ids, lock_files};
use super::model::{File, FileAttributes, MagicMetadata};
use super::objects::{self, TRASH_EMPTY_QUEUE};
use super::sql::{begin, db_err, get_bool, get_i64, get_opt_str, get_str, ids_list};
use super::usage;
use crate::auth::App;
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::state::AppState;
use serde::Serialize;
use sqlx::AnyConnection;
use sqlx::any::AnyRow;
use std::collections::BTreeMap;

/// Days a trashed file is kept before it is deleted automatically.
pub const TRASH_DURATION_DAYS: i64 = 30;
/// Page size of the trash diff, and the most files [`delete`] accepts at once.
pub const TRASH_DIFF_LIMIT: usize = 2500;
/// Batch size for trash inserts and empty-trash deletes.
pub const TRASH_BATCH_SIZE: usize = 1000;
/// Queue for Locker empty-trash requests.
pub const TRASH_EMPTY_LOCKER_QUEUE: &str = "trashEmptyLocker";
/// Separator in empty-trash queue items (`"<userID>::<lastUpdatedAt>"`).
pub const EMPTY_TRASH_SEPARATOR: &str = "::";

const DAY_MICROS: i64 = 24 * 3600 * 1_000_000;

/// `ente.Trash`: one trash diff entry.
#[derive(Debug, Clone, Serialize)]
pub struct TrashEntry {
    /// The file as it was in the collection it was trashed from.
    pub file: File,
    /// Permanently deleted.
    #[serde(rename = "isDeleted")]
    pub is_deleted: bool,
    /// Restored into a collection.
    #[serde(rename = "isRestored")]
    pub is_restored: bool,
    /// When the file will be deleted automatically.
    #[serde(rename = "deleteBy")]
    pub delete_by: i64,
    /// When the file was trashed.
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    /// Version of this entry.
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

/// `TrashRepository.InsertItems`: insert (or revive) trash rows for `items`.
pub async fn insert_items(
    conn: &mut AnyConnection,
    user_id: i64,
    items: &[TrashItem],
    now: i64,
) -> Result<(), sqlx::Error> {
    let delete_by = now + TRASH_DURATION_DAYS * DAY_MICROS;
    for chunk in items.chunks(TRASH_BATCH_SIZE) {
        let values = (0..chunk.len())
            .map(|i| {
                let b = i * 6;
                format!(
                    "(${}, ${}, ${}, ${}, ${}, ${})",
                    b + 1,
                    b + 2,
                    b + 3,
                    b + 4,
                    b + 5,
                    b + 6
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let n = chunk.len() * 6;
        let sql = format!(
            "INSERT INTO trash (file_id, collection_id, user_id, delete_by, created_at, updated_at)
             VALUES {values}
             ON CONFLICT (file_id) DO UPDATE SET is_restored = FALSE, delete_by = ${}, updated_at = ${}
             WHERE trash.is_deleted = FALSE",
            n + 1,
            n + 2
        );
        let mut q = sqlx::query(&sql);
        for it in chunk {
            q = q
                .bind(it.file_id)
                .bind(it.collection_id)
                .bind(user_id)
                .bind(delete_by)
                .bind(now)
                .bind(now);
        }
        q.bind(delete_by).bind(now).execute(&mut *conn).await?;
    }
    Ok(())
}

/// `getFilesInTrashState`: which of `ids` are in the user's trash (not deleted or restored).
pub async fn files_in_trash_state(
    conn: &mut AnyConnection,
    user_id: i64,
    ids: &[i64],
) -> Result<Vec<i64>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT file_id FROM trash WHERE user_id = $1 AND file_id IN ({})
           AND is_deleted = FALSE AND is_restored = FALSE",
        ids_list(2, ids.len())
    );
    bind_ids(sqlx::query(&sql).bind(user_id), ids)
        .fetch_all(conn)
        .await?
        .iter()
        .map(|r| get_i64(r, "file_id"))
        .collect()
}

/// Mark the user's trash rows for `ids` restored (called when files are restored).
pub async fn mark_restored(
    conn: &mut AnyConnection,
    user_id: i64,
    ids: &[i64],
    now: i64,
) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "UPDATE trash SET is_restored = TRUE, updated_at = $1
         WHERE user_id = $2 AND file_id IN ({}) AND is_deleted = FALSE AND is_restored = FALSE",
        ids_list(3, ids.len())
    );
    bind_ids(sqlx::query(&sql).bind(now).bind(user_id), ids)
        .execute(conn)
        .await?;
    Ok(())
}

const TRASH_COLUMNS: &str =
    "t.file_id, t.user_id, t.collection_id, cf.encrypted_key, cf.key_decryption_nonce,
    f.file_decryption_header, f.thumbnail_decryption_header, f.metadata_decryption_header,
    f.encrypted_metadata, f.magic_metadata, f.pub_magic_metadata, f.updation_time, f.info,
    t.is_deleted, t.is_restored, t.created_at, t.updated_at, t.delete_by";

fn row_to_entry(r: &AnyRow) -> Result<TrashEntry, sqlx::Error> {
    let is_deleted = get_bool(r, "is_deleted")?;
    let mut file = File {
        id: get_i64(r, "file_id")?,
        owner_id: get_i64(r, "user_id")?,
        collection_id: get_i64(r, "collection_id")?,
        encrypted_key: get_str(r, "encrypted_key")?,
        key_decryption_nonce: get_str(r, "key_decryption_nonce")?,
        file: FileAttributes {
            decryption_header: get_str(r, "file_decryption_header")?,
            ..Default::default()
        },
        thumbnail: FileAttributes {
            decryption_header: get_str(r, "thumbnail_decryption_header")?,
            ..Default::default()
        },
        metadata: FileAttributes {
            decryption_header: get_str(r, "metadata_decryption_header")?,
            encrypted_data: get_str(r, "encrypted_metadata")?,
            ..Default::default()
        },
        updation_time: get_i64(r, "updation_time")?,
        magic_metadata: MagicMetadata::parse(get_opt_str(r, "magic_metadata")?.as_deref()),
        pub_magic_metadata: MagicMetadata::parse(get_opt_str(r, "pub_magic_metadata")?.as_deref()),
        info: get_opt_str(r, "info")?.and_then(|s| serde_json::from_str(&s).ok()),
        ..Default::default()
    };
    if is_deleted {
        // TrashController.GetDiff drops the encrypted data of permanently deleted files.
        file.magic_metadata = None;
        file.pub_magic_metadata = None;
        file.metadata = FileAttributes::default();
        file.info = None;
    }
    Ok(TrashEntry {
        file,
        is_deleted,
        is_restored: get_bool(r, "is_restored")?,
        delete_by: get_i64(r, "delete_by")?,
        created_at: get_i64(r, "created_at")?,
        updated_at: get_i64(r, "updated_at")?,
    })
}

async fn diff_rows(
    state: &AppState,
    user_id: i64,
    since: i64,
    limit: i64,
    app: App,
) -> ApiResult<Vec<TrashEntry>> {
    let rows = sqlx::query(&format!(
        "SELECT {TRASH_COLUMNS} FROM trash t
         JOIN collection_files cf ON t.file_id = cf.file_id AND t.collection_id = cf.collection_id
         JOIN files f ON f.file_id = t.file_id AND t.user_id = $1 AND f.owner_id = $1 AND t.updated_at > $2
         JOIN collections c ON c.collection_id = t.collection_id
         WHERE c.app = $3
         ORDER BY t.updated_at, t.file_id LIMIT $4"
    ))
    .bind(user_id)
    .bind(since)
    .bind(app.as_str())
    .bind(limit)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    rows.iter()
        .map(row_to_entry)
        .collect::<Result<_, _>>()
        .map_err(db_err)
}

async fn entries_with_version(
    state: &AppState,
    user_id: i64,
    version: i64,
    app: App,
) -> ApiResult<Vec<TrashEntry>> {
    let rows = sqlx::query(&format!(
        "SELECT {TRASH_COLUMNS} FROM trash t
         JOIN collection_files cf ON t.file_id = cf.file_id AND t.collection_id = cf.collection_id
         JOIN files f ON f.file_id = t.file_id AND t.user_id = $1 AND t.updated_at = $2
         JOIN collections c ON c.collection_id = t.collection_id
         WHERE c.app = $3 ORDER BY t.file_id"
    ))
    .bind(user_id)
    .bind(version)
    .bind(app.as_str())
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    rows.iter()
        .map(row_to_entry)
        .collect::<Result<_, _>>()
        .map_err(db_err)
}

/// `TrashController.GetDiff`: trash entries changed after `since`, ordered by `updatedAt`,
/// never splitting one `updatedAt` across pages. Returns `(diff, hasMore)`.
pub async fn diff(
    state: &AppState,
    user_id: i64,
    since: i64,
    app: App,
) -> ApiResult<(Vec<TrashEntry>, bool)> {
    let rows = diff_rows(state, user_id, since, TRASH_DIFF_LIMIT as i64 + 1, app).await?;
    page_by_version(
        rows,
        TRASH_DIFF_LIMIT,
        |e| e.updated_at,
        |v| entries_with_version(state, user_id, v, app),
    )
    .await
}

/// `TrashRepository.Delete`: permanently delete those of `ids` that are in the user's trash.
/// Their objects are queued for deletion and their bytes leave the user's usage.
pub async fn delete(state: &AppState, user_id: i64, ids: &[i64]) -> ApiResult<()> {
    if ids.len() > TRASH_DIFF_LIMIT {
        return Err(errors::internal(format!(
            "can not delete more than {TRASH_DIFF_LIMIT} in one go"
        )));
    }
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    lock_files(&mut tx, state, user_id, ids)
        .await
        .map_err(db_err)?;
    let in_trash = files_in_trash_state(tx.conn(), user_id, ids)
        .await
        .map_err(db_err)?;
    if in_trash.len() != ids.len() {
        tracing::warn!(
            user_id,
            requested = ids.len(),
            in_trash = in_trash.len(),
            "mismatch in input fileIDs and fileIDs present in trash"
        );
    }
    if in_trash.is_empty() {
        tx.commit().await.map_err(db_err)?;
        return Ok(());
    }
    let now = now_micros();
    let list = ids_list(2, in_trash.len());
    bind_ids(
        sqlx::query(&format!(
            "UPDATE trash SET is_deleted = TRUE, updated_at = $1 WHERE file_id IN ({list})"
        ))
        .bind(now),
        &in_trash,
    )
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    schedule_deletion(tx.conn(), user_id, &in_trash, now)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// `FileRepository.scheduleDeletion`: mark the files' objects and file data deleted, queue
/// the objects for removal and subtract their size from the owner's usage.
pub async fn schedule_deletion(
    conn: &mut AnyConnection,
    user_id: i64,
    file_ids: &[i64],
    now: i64,
) -> Result<(), sqlx::Error> {
    if file_ids.is_empty() {
        return Ok(());
    }
    let list = ids_list(1, file_ids.len());
    let rows = bind_ids(
        sqlx::query(&format!(
            "SELECT object_key, size FROM object_keys WHERE file_id IN ({list}) AND is_deleted = FALSE"
        )),
        file_ids,
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut keys = Vec::with_capacity(rows.len());
    let mut total = 0i64;
    for r in &rows {
        keys.push(get_str(r, "object_key")?);
        total += get_i64(r, "size")?;
    }
    bind_ids(
        sqlx::query(&format!(
            "UPDATE object_keys SET is_deleted = TRUE WHERE file_id IN ({list})"
        )),
        file_ids,
    )
    .execute(&mut *conn)
    .await?;
    bind_ids(
        sqlx::query(&format!(
            "UPDATE file_data SET is_deleted = TRUE, pending_sync = TRUE, updated_at = $1
             WHERE file_id IN ({})",
            ids_list(2, file_ids.len())
        ))
        .bind(now),
        file_ids,
    )
    .execute(&mut *conn)
    .await?;
    objects::queue_object_deletion(&mut *conn, &keys).await?;
    if total != 0 {
        usage::adjust(conn, user_id, -total).await?;
    }
    Ok(())
}

/// `TrashController.EmptyTrash`: queue deletion of every entry trashed at or before
/// `last_updated_at`, then process the queue right away (in the background).
pub async fn empty(
    state: &AppState,
    user_id: i64,
    last_updated_at: i64,
    app: App,
) -> ApiResult<()> {
    let queue = if app == App::Locker {
        TRASH_EMPTY_LOCKER_QUEUE
    } else {
        TRASH_EMPTY_QUEUE
    };
    let item = format!("{user_id}{EMPTY_TRASH_SEPARATOR}{last_updated_at}");
    let mut conn = state.db.pool.acquire().await.map_err(db_err)?;
    objects::enqueue(&mut conn, queue, &item)
        .await
        .map_err(db_err)?;
    drop(conn);
    let st = state.clone();
    tokio::spawn(async move {
        process_empty_trash_requests(&st).await;
    });
    Ok(())
}

/// Ids of the entries an empty-trash request covers (`GetFilesIDsForDeletion`).
pub async fn file_ids_for_deletion(
    state: &AppState,
    user_id: i64,
    last_updated_at: i64,
    app: App,
) -> ApiResult<Vec<i64>> {
    sqlx::query(
        "SELECT t.file_id FROM trash t JOIN collections c ON c.collection_id = t.collection_id
         WHERE t.user_id = $1 AND t.updated_at <= $2 AND t.is_deleted = FALSE
           AND t.is_restored = FALSE AND c.app = $3",
    )
    .bind(user_id)
    .bind(last_updated_at)
    .bind(app.as_str())
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?
    .iter()
    .map(|r| get_i64(r, "file_id"))
    .collect::<Result<_, _>>()
    .map_err(db_err)
}

static EMPTY_TRASH_RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Process every pending empty-trash request (Photos and Locker queues). Concurrent calls
/// are skipped while one is running.
pub async fn process_empty_trash_requests(state: &AppState) {
    let Ok(_guard) = EMPTY_TRASH_RUNNING.try_lock() else {
        return;
    };
    for (queue, app) in [
        (TRASH_EMPTY_QUEUE, App::Photos),
        (TRASH_EMPTY_LOCKER_QUEUE, App::Locker),
    ] {
        let items = match pending_queue_items(state, queue, 100).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e.message, queue, "could not read empty-trash queue");
                continue;
            }
        };
        for item in items {
            if let Err(e) = empty_one(state, queue, &item, app).await {
                tracing::error!(error = %e.message, item, "failed to empty trash");
            }
        }
    }
}

async fn empty_one(state: &AppState, queue: &str, item: &str, app: App) -> ApiResult<()> {
    let mut parts = item.split(EMPTY_TRASH_SEPARATOR);
    let user_id: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let last: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ids = file_ids_for_deletion(state, user_id, last, app).await?;
    for batch in ids.chunks(TRASH_BATCH_SIZE) {
        delete(state, user_id, batch).await?;
    }
    delete_queue_item(state, queue, item).await
}

/// Undeleted items of `queue`, oldest first.
pub async fn pending_queue_items(
    state: &AppState,
    queue: &str,
    limit: i64,
) -> ApiResult<Vec<String>> {
    sqlx::query(
        "SELECT item FROM queue WHERE queue_name = $1 AND is_deleted = FALSE
         ORDER BY created_at, queue_id LIMIT $2",
    )
    .bind(queue)
    .bind(limit)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?
    .iter()
    .map(|r| get_str(r, "item"))
    .collect::<Result<_, _>>()
    .map_err(db_err)
}

/// Mark a queue item done.
pub async fn delete_queue_item(state: &AppState, queue: &str, item: &str) -> ApiResult<()> {
    sqlx::query(
        "UPDATE queue SET is_deleted = TRUE, updated_at = $1 WHERE queue_name = $2 AND item = $3",
    )
    .bind(now_micros())
    .bind(queue)
    .bind(item)
    .execute(&state.db.pool)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// `DeleteAgedTrashedFiles`: permanently delete entries whose `delete_by` has passed.
pub async fn delete_aged(state: &AppState) -> ApiResult<()> {
    let rows = sqlx::query(
        "SELECT user_id, file_id FROM trash
         WHERE delete_by <= $1 AND is_deleted = FALSE AND is_restored = FALSE LIMIT $2",
    )
    .bind(now_micros())
    .bind(TRASH_DIFF_LIMIT as i64)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    let mut by_user: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for r in &rows {
        by_user
            .entry(get_i64(r, "user_id").map_err(db_err)?)
            .or_default()
            .push(get_i64(r, "file_id").map_err(db_err)?);
    }
    for (uid, ids) in by_user {
        if let Err(e) = delete(state, uid, &ids).await {
            tracing::error!(user_id = uid, error = %e.message, "failed to delete aged trash");
        }
    }
    Ok(())
}
