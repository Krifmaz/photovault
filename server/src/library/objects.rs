//! Object bookkeeping: staged uploads (`temp_objects`), the work `queue`, and upload URL issuance.
//!
//! Every upload URL the server hands out is recorded in `temp_objects`. Creating a file
//! *consumes* the row ([`consume_file_upload`]); rows that are never consumed expire and
//! the background task ([`super::tasks`]) deletes the orphaned object. Objects that stop
//! being referenced (replaced by an update, permanently deleted) are queued under
//! [`DELETE_OBJECT_QUEUE`] and deleted by the same task once no `object_keys` row points at them.
//!
//! The public-collection workstream can reuse [`issue_put_url`] / [`issue_multipart`] for
//! uploads into shared albums (pass its own `purpose`).

use super::errors;
use super::sql::db_err;
use crate::auth::App;
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::state::AppState;
use crate::storage::MultipartUrls;
use serde::Serialize;
use sqlx::AnyConnection;
use std::time::Duration;

/// Validity of signed upload/download URLs (museum's `PreSignedRequestValidityDuration`).
pub const URL_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// How long a staged upload is kept before the orphaned object is deleted.
pub const TEMP_OBJECT_TTL_MICROS: i64 = 14 * 24 * 3600 * 1_000_000;
/// Purpose of staged file and thumbnail uploads.
pub const PURPOSE_FILE_UPLOAD: &str = "file_upload";
/// Queue of object keys to delete once unreferenced.
pub const DELETE_OBJECT_QUEUE: &str = "deleteObject";
/// Queue of collection ids whose files must be trashed/removed after a delete.
pub const TRASH_COLLECTION_QUEUE: &str = "trashCollectionV3";
/// Queue of `"<userID>::<lastUpdatedAt>"` empty-trash requests.
pub const TRASH_EMPTY_QUEUE: &str = "trashEmpty";
/// museum's `MaxFileSize` (10 GiB).
pub const MAX_FILE_SIZE: i64 = 10 * 1024 * 1024 * 1024;
/// S3's maximum part size (5 GiB).
pub const MAX_PART_SIZE: i64 = 5 * 1024 * 1024 * 1024;
/// S3's minimum part size for all but the last part (5 MiB).
pub const MIN_PART_SIZE: i64 = 5 * 1024 * 1024;
/// S3's maximum number of parts.
pub const MAX_PARTS: i64 = 10_000;

/// museum's `ente.UploadURL`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadUrl {
    /// Object key to report back when creating the file.
    pub object_key: String,
    /// Signed PUT URL.
    pub url: String,
}

/// museum's `ente.MultipartUploadURLs` (note `partURLs` / `completeURL`).
#[derive(Debug, Clone, Serialize)]
pub struct MultipartUploadUrls {
    /// Object key.
    #[serde(rename = "objectKey")]
    pub object_key: String,
    /// One URL per part.
    #[serde(rename = "partURLs")]
    pub part_urls: Vec<String>,
    /// Completion URL.
    #[serde(rename = "completeURL")]
    pub complete_url: String,
}

impl From<MultipartUrls> for MultipartUploadUrls {
    fn from(m: MultipartUrls) -> Self {
        Self {
            object_key: m.object_key,
            part_urls: m.part_urls,
            complete_url: m.complete_url,
        }
    }
}

/// A random RFC 4122 v4 UUID string.
pub fn uuid_v4() -> String {
    let mut b = crate::crypto::random_bytes(16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// A fresh object key `"<userID>/<uuid>"`.
pub fn new_object_key(user_id: i64) -> String {
    format!("{user_id}/{}", uuid_v4())
}

/// `ente.NormalizeMD5`: base64 of exactly 16 bytes, else 400 `{}`.
pub fn normalize_md5(v: &str) -> ApiResult<String> {
    let t = v.trim();
    if t.is_empty() {
        return Err(errors::bad());
    }
    let d = crate::crypto::unb64(t).map_err(|_| errors::bad())?;
    if d.len() != 16 {
        return Err(errors::bad());
    }
    Ok(crate::crypto::b64(&d))
}

/// `ente.ValidateMD5`: the value must already be canonical base64 of 16 bytes, else
/// 400 `BAD_REQUEST`.
pub fn validate_md5(v: &str) -> ApiResult<()> {
    match normalize_md5(v) {
        Ok(n) if n == v => Ok(()),
        _ => Err(errors::bad_msg(
            "MD5 checksums must be base64-encoded 16-byte digests",
        )),
    }
}

/// `ente.ValidateMultipartPartLength`.
pub fn validate_part_length(content_length: i64, part_length: i64) -> ApiResult<()> {
    if part_length <= 0 || part_length > MAX_PART_SIZE {
        return Err(errors::bad_msg(
            "partLength must be between 1 byte and 5 GiB",
        ));
    }
    if content_length > part_length && part_length < MIN_PART_SIZE {
        return Err(errors::bad_msg(
            "partLength must be at least 5 MiB when more than one part is required",
        ));
    }
    Ok(())
}

/// A staged upload row.
#[derive(Debug, Clone, Default)]
pub struct TempObject {
    /// Object key.
    pub object_key: String,
    /// Multipart upload id, when multipart.
    pub upload_id: Option<String>,
    /// Uploading user (`None` for anonymous public-link uploads).
    pub user_id: Option<i64>,
    /// App.
    pub app: Option<String>,
    /// What the upload is for (`file_upload`, `vid_preview`, …).
    pub purpose: Option<String>,
    /// Declared length.
    pub content_length: Option<i64>,
    /// Declared MD5 (base64).
    pub content_md5: Option<String>,
    /// Client info (`X-Client-Package`/version).
    pub client: Option<String>,
}

/// Record a staged upload (`ObjectCleanupController.AddTempObject`).
pub async fn add_temp_object(state: &AppState, t: &TempObject) -> ApiResult<()> {
    let now = now_micros();
    sqlx::query(
        "INSERT INTO temp_objects (object_key, expiration_time, upload_id, is_multipart, bucket_id,
            user_id, app, purpose, content_length, content_md5, client, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(&t.object_key)
    .bind(now + TEMP_OBJECT_TTL_MICROS)
    .bind(t.upload_id.clone())
    .bind(t.upload_id.is_some())
    .bind(state.store.name())
    .bind(t.user_id)
    .bind(t.app.clone())
    .bind(t.purpose.clone())
    .bind(t.content_length)
    .bind(t.content_md5.clone())
    .bind(t.client.clone())
    .bind(now)
    .execute(&state.db.pool)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// Sign a PUT URL for `key` and stage it.
pub async fn issue_put_url(state: &AppState, temp: TempObject) -> ApiResult<UploadUrl> {
    let url = state
        .store
        .presign_put(
            &temp.object_key,
            URL_TTL,
            temp.content_length,
            temp.content_md5.as_deref(),
        )
        .await
        .map_err(errors::storage)?;
    add_temp_object(state, &temp).await?;
    Ok(UploadUrl {
        object_key: temp.object_key,
        url,
    })
}

/// Start a multipart upload of `parts` parts for `temp.object_key` and stage it.
pub async fn issue_multipart(
    state: &AppState,
    mut temp: TempObject,
    parts: u32,
) -> ApiResult<MultipartUploadUrls> {
    let m = state
        .store
        .create_multipart(&temp.object_key, parts, URL_TTL)
        .await
        .map_err(errors::storage)?;
    temp.upload_id = upload_id_from_url(&m.complete_url).or_else(|| Some(String::new()));
    add_temp_object(state, &temp).await?;
    Ok(m.into())
}

/// The `uploadId` query parameter of a multipart URL.
pub fn upload_id_from_url(u: &str) -> Option<String> {
    let parsed = url::Url::parse(u).ok()?;
    parsed
        .query_pairs()
        .find(|(k, _)| k == "uploadId")
        .map(|(_, v)| v.into_owned())
}

/// `ConsumeFileUpload`: delete the staged row for a key used by a new file. Exactly one row
/// must match, else 400 `{"code":"BAD_REQUEST","message":"staged upload not found"}`.
pub async fn consume_file_upload(
    conn: &mut AnyConnection,
    key: &str,
    user_id: i64,
    app: App,
) -> ApiResult<()> {
    let r = sqlx::query(
        "DELETE FROM temp_objects WHERE object_key = $1
           AND (user_id IS NULL OR user_id = $2)
           AND (app IS NULL OR app = $3)
           AND (purpose IS NULL OR purpose = 'file_upload')",
    )
    .bind(key)
    .bind(user_id)
    .bind(app.as_str())
    .execute(conn)
    .await
    .map_err(db_err)?;
    if r.rows_affected() != 1 {
        return Err(errors::bad_msg("staged upload not found"));
    }
    Ok(())
}

/// Delete staged rows for `key` (any purpose); returns the number removed.
pub async fn remove_temp_object(conn: &mut AnyConnection, key: &str) -> ApiResult<u64> {
    let r = sqlx::query("DELETE FROM temp_objects WHERE object_key = $1")
        .bind(key)
        .execute(conn)
        .await
        .map_err(db_err)?;
    Ok(r.rows_affected())
}

/// Add an item to a queue (duplicates are ignored).
pub async fn enqueue(conn: &mut AnyConnection, queue: &str, item: &str) -> Result<(), sqlx::Error> {
    let now = now_micros();
    sqlx::query(
        "INSERT INTO queue (queue_name, item, created_at, updated_at) VALUES ($1, $2, $3, $3)
         ON CONFLICT (queue_name, item) DO UPDATE SET is_deleted = FALSE, updated_at = $3",
    )
    .bind(queue)
    .bind(item)
    .bind(now)
    .execute(conn)
    .await?;
    Ok(())
}

/// Queue object keys for deletion from the store.
pub async fn queue_object_deletion(
    conn: &mut AnyConnection,
    keys: &[String],
) -> Result<(), sqlx::Error> {
    for k in keys {
        enqueue(&mut *conn, DELETE_OBJECT_QUEUE, k).await?;
    }
    Ok(())
}

/// Client info like museum's `network.GetClientInfo` (package and version).
pub fn client_info(headers: &axum::http::HeaderMap) -> String {
    let pkg = super::http::header(headers, "X-Client-Package");
    let ver = super::http::header(headers, "X-Client-Version");
    if pkg.is_empty() && ver.is_empty() {
        String::new()
    } else {
        format!("{pkg}:{ver}")
    }
}

/// Is `e` a unique-constraint violation?
pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .map(|d| d.is_unique_violation())
        .unwrap_or(false)
}
