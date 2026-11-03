//! Every query this workstream makes against tables owned by other workstreams.
//!
//! Kept in one module so the integrator can replace these bodies with the real
//! collections/files helpers. Tables and columns relied on (museum's names):
//!
//! * `collections(collection_id BIGINT, owner_id BIGINT, app TEXT NULL, is_deleted BOOLEAN NULL)`
//! * `collection_shares(collection_id BIGINT, to_user_id BIGINT, role_type TEXT, is_deleted BOOLEAN NULL)`
//!   with `role_type` in `VIEWER | COLLABORATOR | ADMIN` (museum `CollectionParticipantRole`).
//! * `collection_files(collection_id BIGINT, file_id BIGINT, is_deleted BOOLEAN NULL)`.
//!   museum also treats rows whose `action = 'REMOVE'` as not active; that column is not
//!   assumed here (see [`require_file_in_collection`]).
//! * `users(user_id, encrypted_email TEXT, email_decryption_nonce TEXT)` (0001_core):
//!   both base64 (standard), secretbox under `keys.encryption`.

use super::{not_found, permission_denied};
use crate::auth::App;
use crate::db::Db;
use crate::error::ApiResult;
use crate::state::AppState;

/// The caller's role on a collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Read-only sharee.
    Viewer,
    /// Sharee who can add files.
    Collaborator,
    /// Sharee with moderation rights.
    Admin,
    /// The collection owner.
    Owner,
}

impl Role {
    fn parse(s: &str) -> Self {
        match s {
            "ADMIN" => Role::Admin,
            "COLLABORATOR" => Role::Collaborator,
            "OWNER" => Role::Owner,
            _ => Role::Viewer,
        }
    }
}

/// museum `access.Controller.GetCollection`: the caller must own the collection or be an
/// active sharee, the collection must belong to the calling app and not be deleted.
/// Missing collection / not shared / deleted → 404, app mismatch → 403.
pub async fn collection_role(
    db: &Db,
    collection_id: i64,
    user_id: i64,
    app: App,
) -> ApiResult<Role> {
    let row: Option<(i64, Option<String>, i64)> = sqlx::query_as(
        "SELECT owner_id, app, CAST(CASE WHEN COALESCE(is_deleted, FALSE) THEN 1 ELSE 0 END AS BIGINT) \
         FROM collections WHERE collection_id = $1",
    )
    .bind(collection_id)
    .fetch_optional(&db.pool)
    .await?;
    let (owner_id, coll_app, is_deleted) = row.ok_or_else(not_found)?;
    let role = if owner_id == user_id {
        Role::Owner
    } else {
        let share: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT role_type FROM collection_shares \
             WHERE collection_id = $1 AND to_user_id = $2 AND COALESCE(is_deleted, FALSE) = FALSE",
        )
        .bind(collection_id)
        .bind(user_id)
        .fetch_optional(&db.pool)
        .await?;
        let (role_type,) = share.ok_or_else(not_found)?;
        Role::parse(role_type.as_deref().unwrap_or("VIEWER"))
    };
    let coll_app = coll_app.filter(|a| !a.is_empty());
    if coll_app.as_deref().unwrap_or("photos") != app.as_str() {
        return Err(permission_denied());
    }
    if is_deleted != 0 {
        return Err(not_found());
    }
    Ok(role)
}

/// museum `validateFileInCollection`: the file must be actively in the collection,
/// otherwise 403.
pub async fn require_file_in_collection(
    db: &Db,
    collection_id: i64,
    file_id: i64,
) -> ApiResult<()> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT CAST(CASE WHEN COALESCE(is_deleted, FALSE) THEN 1 ELSE 0 END AS BIGINT) \
         FROM collection_files WHERE collection_id = $1 AND file_id = $2",
    )
    .bind(collection_id)
    .bind(file_id)
    .fetch_optional(&db.pool)
    .await?;
    match row {
        Some((0,)) => Ok(()),
        _ => Err(permission_denied()),
    }
}

/// Non-deleted collections of `app` that the user owns or is an active sharee of
/// (museum `GetCollectionIDsOwnedByUser` + `GetCollectionIDsSharedWithUser` +
/// `FilterNonDeletedCollectionIDs`).
pub async fn accessible_collection_ids(db: &Db, user_id: i64, app: App) -> ApiResult<Vec<i64>> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT collection_id FROM collections \
         WHERE (owner_id = $1 OR collection_id IN \
               (SELECT collection_id FROM collection_shares WHERE to_user_id = $1 AND COALESCE(is_deleted, FALSE) = FALSE)) \
           AND COALESCE(is_deleted, FALSE) = FALSE \
           AND COALESCE(NULLIF(app, ''), 'photos') = $2 \
         ORDER BY collection_id",
    )
    .bind(user_id)
    .bind(app.as_str())
    .fetch_all(&db.pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// The user's email address, decrypted, or `None` if the user is missing or deleted.
pub async fn user_email(state: &AppState, user_id: i64) -> ApiResult<Option<String>> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT encrypted_email, email_decryption_nonce FROM users WHERE user_id = $1 AND is_deleted = FALSE",
    )
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await?;
    let Some((ct, nonce)) = row else {
        return Ok(None);
    };
    let (Ok(ct), Ok(nonce)) = (crate::crypto::unb64(&ct), crate::crypto::unb64(&nonce)) else {
        return Ok(None);
    };
    Ok(
        crate::crypto::secretbox_open(&ct, &nonce, &state.keys.encryption)
            .ok()
            .and_then(|b| String::from_utf8(b).ok()),
    )
}
