//! Every query this workstream makes against tables owned by other workstreams.
//!
//! Access checks go through [`crate::library::collections::access`]; the remaining queries
//! read the library's tables (migration `0200_library`) directly, with its portable
//! boolean getters. Emails come from `users` (0001_core).

use super::permission_denied;
use crate::auth::App;
use crate::library::collections;
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

/// museum `access.Controller.GetCollection` ([`collections::access`]): the caller must own
/// the collection or be an active sharee, the collection must belong to the calling app
/// and not be deleted. Missing collection / not shared / deleted → 404, app mismatch → 403.
pub async fn collection_role(
    state: &AppState,
    collection_id: i64,
    user_id: i64,
    app: App,
) -> ApiResult<Role> {
    let a = collections::access(state, collection_id, user_id, app, false, false).await?;
    Ok(Role::parse(a.role))
}

/// museum `validateFileInCollection`: the file must be actively in the collection (present,
/// not deleted, and not pending removal via `action = 'REMOVE'`), otherwise 403.
pub async fn require_file_in_collection(
    state: &AppState,
    collection_id: i64,
    file_id: i64,
) -> ApiResult<()> {
    if collections::is_active_member(state, collection_id, file_id).await? {
        Ok(())
    } else {
        Err(permission_denied())
    }
}

/// Non-deleted collections of `app` that the user owns or is an active sharee of
/// (museum `GetCollectionIDsOwnedByUser` + `GetCollectionIDsSharedWithUser` +
/// `FilterNonDeletedCollectionIDs`).
pub async fn accessible_collection_ids(db: &Db, user_id: i64, app: App) -> ApiResult<Vec<i64>> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT collection_id FROM collections \
         WHERE (owner_id = $1 OR collection_id IN \
               (SELECT collection_id FROM collection_shares WHERE to_user_id = $1 AND is_deleted = FALSE)) \
           AND is_deleted = FALSE AND app = $2 \
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
