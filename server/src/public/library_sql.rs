//! The two reads of library tables (migration `0200_library`) that the public endpoints
//! need and [`crate::library`] has no helper for. Everything else goes through the
//! library's repositories (`collections`, `files`, `links`, `filedata`, `objects`).

use super::models::{File, FileAttributes, MagicMetadata};
use crate::error::ApiResult;
use crate::library::sql::{db_err, get_i64, get_opt_str, get_str};
use crate::state::AppState;

/// museum `FileRepo.GetFileAttributes`: the file's encrypted attributes without any
/// collection context (`None` when the file does not exist).
pub async fn file_attributes(state: &AppState, file_id: i64) -> ApiResult<Option<File>> {
    let r = sqlx::query(
        "SELECT file_id, owner_id, file_decryption_header, thumbnail_decryption_header,
            metadata_decryption_header, encrypted_metadata, pub_magic_metadata, info
         FROM files WHERE file_id = $1",
    )
    .bind(file_id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(db_err)?;
    let Some(r) = r else { return Ok(None) };
    let file = (|| -> Result<File, sqlx::Error> {
        Ok(File {
            id: get_i64(&r, "file_id")?,
            owner_id: get_i64(&r, "owner_id")?,
            file: FileAttributes {
                decryption_header: get_str(&r, "file_decryption_header")?,
                ..Default::default()
            },
            thumbnail: FileAttributes {
                decryption_header: get_str(&r, "thumbnail_decryption_header")?,
                ..Default::default()
            },
            metadata: FileAttributes {
                decryption_header: get_str(&r, "metadata_decryption_header")?,
                encrypted_data: get_str(&r, "encrypted_metadata")?,
                ..Default::default()
            },
            pub_magic_metadata: MagicMetadata::parse(
                get_opt_str(&r, "pub_magic_metadata")?.as_deref(),
            ),
            info: get_opt_str(&r, "info")?.and_then(|s| serde_json::from_str(&s).ok()),
            ..Default::default()
        })
    })()
    .map_err(db_err)?;
    Ok(Some(file))
}

/// museum `CollectionRepo.GetCollectionCount`: in how many collections the file is a
/// (non-deleted) member.
pub async fn active_collection_count(state: &AppState, file_id: i64) -> ApiResult<i64> {
    let r = sqlx::query(
        "SELECT COUNT(*) AS n FROM collection_files WHERE file_id = $1 AND is_deleted = FALSE",
    )
    .bind(file_id)
    .fetch_one(&state.db.pool)
    .await
    .map_err(db_err)?;
    get_i64(&r, "n").map_err(db_err)
}
