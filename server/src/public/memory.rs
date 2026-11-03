//! Memory shares: `memory_shares` / `memory_share_files` (museum `MemoryShareRepository`).

use super::models::MemoryShare;
use crate::db::now_micros;
use crate::state::AppState;
use sqlx::any::AnyRow;
use sqlx::{AnyPool, Row};

/// Memory shares expire 7 days after creation (museum `MemoryShareTTL`, microseconds).
pub const MEMORY_SHARE_TTL: i64 = 7 * 24 * 3600 * 1_000_000;

/// Base URL of the public memories web app: `[accounts] public_memories_url`
/// (museum `apps.public-memories`), defaulting to the server's public URL.
pub fn memories_host(state: &AppState) -> String {
    crate::library::links::app_host(state, &state.config.accounts.public_memories_url)
}

const COLS: &str = "id, user_id, type, memory_hash, metadata_cipher, metadata_nonce, mem_enc_key, \
    mem_key_decryption_nonce, access_token, CAST(CASE WHEN is_deleted THEN 1 ELSE 0 END AS BIGINT), created_at, updated_at";

fn row(state: &AppState, r: &AnyRow) -> sqlx::Result<MemoryShare> {
    let access_token: String = r.try_get(8)?;
    Ok(MemoryShare {
        id: r.try_get(0)?,
        user_id: r.try_get(1)?,
        kind: r.try_get(2)?,
        memory_hash: r.try_get::<Option<String>, _>(3)?.unwrap_or_default(),
        metadata_cipher: r.try_get::<Option<String>, _>(4)?.unwrap_or_default(),
        metadata_nonce: r.try_get::<Option<String>, _>(5)?.unwrap_or_default(),
        encrypted_key: r.try_get(6)?,
        key_decryption_nonce: r.try_get(7)?,
        url: format!("{}/{}", memories_host(state), access_token),
        access_token,
        is_deleted: r.try_get::<i64, _>(9)? != 0,
        created_at: r.try_get(10)?,
        updated_at: r.try_get(11)?,
    })
}

/// One file of a share.
#[derive(Debug, Clone)]
pub struct ShareFile {
    /// File id.
    pub file_id: i64,
    /// File owner.
    pub file_owner_id: i64,
    /// Position in the memory.
    pub position: i64,
    /// File key encrypted with the share key.
    pub encrypted_key: String,
    /// Nonce for `encrypted_key`.
    pub key_decryption_nonce: String,
}

/// Result of [`create`].
#[derive(Debug)]
pub enum CreateError {
    /// The random access token collided; retry.
    TokenInUse,
    /// Database failure.
    Db(sqlx::Error),
}

/// Insert a share and its files atomically (museum `CreateWithFiles`).
pub async fn create(
    state: &AppState,
    mut share: MemoryShare,
    files: &[ShareFile],
) -> Result<MemoryShare, CreateError> {
    let db = &state.db.pool;
    let now = now_micros();
    let opt = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let mut tx = db.begin().await.map_err(CreateError::Db)?;
    let res: Result<(i64,), sqlx::Error> = sqlx::query_as(
        "INSERT INTO memory_shares (user_id, type, memory_hash, metadata_cipher, metadata_nonce, mem_enc_key, \
         mem_key_decryption_nonce, access_token, is_deleted, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING id",
    )
    .bind(share.user_id)
    .bind(&share.kind)
    .bind(opt(&share.memory_hash))
    .bind(&share.metadata_cipher)
    .bind(&share.metadata_nonce)
    .bind(&share.encrypted_key)
    .bind(&share.key_decryption_nonce)
    .bind(&share.access_token)
    .bind(false)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await;
    let id = match res {
        Ok((id,)) => id,
        Err(sqlx::Error::Database(d)) if d.is_unique_violation() => {
            return Err(CreateError::TokenInUse);
        }
        Err(e) => return Err(CreateError::Db(e)),
    };
    for f in files {
        sqlx::query(
            "INSERT INTO memory_share_files (memory_share_id, file_id, file_owner_id, position, file_enc_key, \
             file_key_decryption_nonce, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(f.file_id)
        .bind(f.file_owner_id)
        .bind(f.position)
        .bind(&f.encrypted_key)
        .bind(&f.key_decryption_nonce)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(CreateError::Db)?;
    }
    tx.commit().await.map_err(CreateError::Db)?;
    share.id = id;
    share.created_at = now;
    share.updated_at = now;
    share.url = format!("{}/{}", memories_host(state), share.access_token);
    Ok(share)
}

/// Share by id.
pub async fn by_id(state: &AppState, id: i64) -> sqlx::Result<Option<MemoryShare>> {
    let q = format!("SELECT {COLS} FROM memory_shares WHERE id = $1");
    match sqlx::query(&q)
        .bind(id)
        .fetch_optional(&state.db.pool)
        .await?
    {
        Some(r) => Ok(Some(row(state, &r)?)),
        None => Ok(None),
    }
}

/// Share by access token.
pub async fn by_access_token(state: &AppState, token: &str) -> sqlx::Result<Option<MemoryShare>> {
    let q = format!("SELECT {COLS} FROM memory_shares WHERE access_token = $1");
    match sqlx::query(&q)
        .bind(token)
        .fetch_optional(&state.db.pool)
        .await?
    {
        Some(r) => Ok(Some(row(state, &r)?)),
        None => Ok(None),
    }
}

/// The user's live (not deleted, not expired) shares, newest first.
pub async fn for_user(state: &AppState, user_id: i64) -> sqlx::Result<Vec<MemoryShare>> {
    let q = format!(
        "SELECT {COLS} FROM memory_shares WHERE user_id = $1 AND is_deleted = FALSE AND created_at > $2 ORDER BY created_at DESC"
    );
    sqlx::query(&q)
        .bind(user_id)
        .bind(now_micros() - MEMORY_SHARE_TTL)
        .fetch_all(&state.db.pool)
        .await?
        .iter()
        .map(|r| row(state, r))
        .collect()
}

/// Soft-delete a share the user owns; false when nothing was deleted.
pub async fn delete(db: &AnyPool, id: i64, user_id: i64) -> sqlx::Result<bool> {
    let r = sqlx::query(
        "UPDATE memory_shares SET is_deleted = TRUE, updated_at = $1 WHERE id = $2 AND user_id = $3 AND is_deleted = FALSE",
    )
    .bind(now_micros())
    .bind(id)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Soft-delete all of a user's shares (museum `DeleteForUser`, for account deletion).
pub async fn delete_for_user(db: &AnyPool, user_id: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE memory_shares SET is_deleted = TRUE, updated_at = $1 WHERE user_id = $2 AND is_deleted = FALSE",
    )
    .bind(now_micros())
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

/// The share's files ordered by position.
pub async fn files(db: &AnyPool, share_id: i64) -> sqlx::Result<Vec<ShareFile>> {
    let rows: Vec<(i64, i64, i64, String, String)> = sqlx::query_as(
        "SELECT file_id, file_owner_id, position, file_enc_key, file_key_decryption_nonce \
         FROM memory_share_files WHERE memory_share_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(share_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(file_id, file_owner_id, position, encrypted_key, key_decryption_nonce)| ShareFile {
                file_id,
                file_owner_id,
                position,
                encrypted_key,
                key_decryption_nonce,
            },
        )
        .collect())
}

/// Owner of `file_id` when it is part of the share.
pub async fn file_owner_in_share(
    db: &AnyPool,
    share_id: i64,
    file_id: i64,
) -> sqlx::Result<Option<i64>> {
    let r: Option<(i64,)> = sqlx::query_as(
        "SELECT file_owner_id FROM memory_share_files WHERE memory_share_id = $1 AND file_id = $2 LIMIT 1",
    )
    .bind(share_id)
    .bind(file_id)
    .fetch_optional(db)
    .await?;
    Ok(r.map(|r| r.0))
}
