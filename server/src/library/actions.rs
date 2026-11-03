//! Collection actions (museum's `CollectionActionsRepository`): pending requests one user
//! leaves for another, such as "remove your file from my album" (`REMOVE`) or "please delete
//! this file" (`DELETE_SUGGESTED`). The owner's apps list them and act on them.

use super::errors;
use super::files::bind_ids;
use super::links::new_id;
use super::model::action;
use super::sql::{db_err, get_bool, get_i64, get_opt_i64, get_opt_str, get_str, ids_list};
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::state::AppState;
use serde::Serialize;
use sqlx::any::AnyRow;

/// Page size of the action listings (`collectionActionsLimit`).
pub const LIST_LIMIT: i64 = 2000;

/// `ente.CollectionAction`.
#[derive(Debug, Clone, Serialize)]
pub struct CollectionAction {
    /// `cact_…` id.
    pub id: String,
    /// User the action is for.
    #[serde(rename = "userID")]
    pub user_id: i64,
    /// User who created it.
    #[serde(rename = "actorUserID")]
    pub actor_user_id: i64,
    /// Collection.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// File, when the action is about one.
    #[serde(rename = "fileID", skip_serializing_if = "Option::is_none")]
    pub file_id: Option<i64>,
    /// `REMOVE` or `DELETE_SUGGESTED`.
    pub action: String,
    /// Still waiting for the user.
    #[serde(rename = "isPending")]
    pub is_pending: bool,
    /// Optional extra data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Map<String, serde_json::Value>>,
    /// Creation time.
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    /// Last change.
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

fn row_to_action(r: &AnyRow) -> Result<CollectionAction, sqlx::Error> {
    let data = get_opt_str(r, "data")?
        .and_then(|s| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&s).ok());
    Ok(CollectionAction {
        id: get_str(r, "id")?,
        user_id: get_i64(r, "user_id")?,
        actor_user_id: get_i64(r, "actor_user_id")?,
        collection_id: get_i64(r, "collection_id")?,
        file_id: get_opt_i64(r, "file_id")?,
        action: get_str(r, "action")?,
        is_pending: get_bool(r, "is_pending")?,
        data,
        created_at: get_i64(r, "created_at")?,
        updated_at: get_i64(r, "updated_at")?,
    })
}

/// `CreateBulk`: one pending `act` for `user_id` per file, created by `actor`.
/// The actor and the target must differ (else 500, like museum).
pub async fn create_bulk(
    state: &AppState,
    user_id: i64,
    actor: i64,
    collection_id: i64,
    file_ids: &[i64],
    act: &str,
) -> ApiResult<()> {
    if file_ids.is_empty() {
        return Ok(());
    }
    if actor == user_id {
        return Err(errors::internal(format!(
            "actor {actor} and target user id {user_id} should be different"
        )));
    }
    let now = now_micros();
    for chunk in file_ids.chunks(500) {
        let values = (0..chunk.len())
            .map(|i| {
                let b = i * 8;
                format!(
                    "(${}, ${}, ${}, ${}, ${}, ${}, ${}, ${})",
                    b + 1,
                    b + 2,
                    b + 3,
                    b + 4,
                    b + 5,
                    b + 6,
                    b + 7,
                    b + 8
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO collection_actions (id, user_id, actor_user_id, collection_id, file_id,
                action, created_at, updated_at) VALUES {values}"
        );
        let mut q = sqlx::query(&sql);
        for fid in chunk {
            q = q
                .bind(new_id("cact"))
                .bind(user_id)
                .bind(actor)
                .bind(collection_id)
                .bind(*fid)
                .bind(act.to_string())
                .bind(now)
                .bind(now);
        }
        q.execute(&state.db.pool).await.map_err(db_err)?;
    }
    Ok(())
}

async fn reset_pending(state: &AppState, ids: &[String]) -> ApiResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "UPDATE collection_actions SET is_pending = FALSE, updated_at = $1 WHERE id IN ({})",
        ids_list(2, ids.len())
    );
    let mut q = sqlx::query(&sql).bind(now_micros());
    for id in ids {
        q = q.bind(id.clone());
    }
    q.execute(&state.db.pool).await.map_err(db_err)?;
    Ok(())
}

/// `ListPendingRemoveActions`: pending `REMOVE` actions for `user_id` changed after `since`.
/// An action whose membership is no longer live and REMOVE-marked is stale: it is reset to
/// not pending and left out.
pub async fn list_pending_remove(
    state: &AppState,
    user_id: i64,
    since: i64,
    limit: i64,
) -> ApiResult<Vec<CollectionAction>> {
    let rows = sqlx::query(
        "SELECT ca.id, ca.user_id, ca.actor_user_id, ca.collection_id, ca.file_id, ca.data,
                ca.action, ca.is_pending, ca.created_at, ca.updated_at,
                cf.collection_id AS valid_cid
         FROM collection_actions ca
         LEFT JOIN collection_files cf ON cf.collection_id = ca.collection_id
              AND cf.file_id = ca.file_id AND cf.is_deleted = FALSE AND cf.action = $1
         WHERE ca.user_id = $2 AND ca.action = $1 AND ca.is_pending = TRUE AND ca.updated_at > $3
         ORDER BY ca.updated_at, ca.id LIMIT $4",
    )
    .bind(action::REMOVE)
    .bind(user_id)
    .bind(since)
    .bind(limit)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    let mut out = Vec::new();
    let mut stale = Vec::new();
    for r in &rows {
        let a = row_to_action(r).map_err(db_err)?;
        if get_opt_i64(r, "valid_cid").map_err(db_err)?.is_none() {
            stale.push(a.id);
            continue;
        }
        out.push(a);
    }
    reset_pending(state, &stale).await?;
    Ok(out)
}

/// `ListPendingDeleteSuggestions`: pending `DELETE_SUGGESTED` actions for `user_id` changed
/// after `since`. Actions for missing or trashed files are stale: reset and left out.
pub async fn list_delete_suggestions(
    state: &AppState,
    user_id: i64,
    since: i64,
    limit: i64,
) -> ApiResult<Vec<CollectionAction>> {
    let rows = sqlx::query(
        "SELECT ca.id, ca.user_id, ca.actor_user_id, ca.collection_id, ca.file_id, ca.data,
                ca.action, ca.is_pending, ca.created_at, ca.updated_at,
                f.owner_id AS f_owner_id, t.file_id AS trash_fid
         FROM collection_actions ca
         LEFT JOIN files f ON f.file_id = ca.file_id
         LEFT JOIN trash t ON t.user_id = ca.user_id AND t.file_id = ca.file_id AND t.is_restored = FALSE
         WHERE ca.user_id = $1 AND ca.action = $2 AND ca.is_pending = TRUE AND ca.updated_at > $3
         ORDER BY ca.updated_at, ca.id LIMIT $4",
    )
    .bind(user_id)
    .bind(action::DELETE_SUGGESTED)
    .bind(since)
    .bind(limit)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    let mut out = Vec::new();
    let mut stale = Vec::new();
    for r in &rows {
        let a = row_to_action(r).map_err(db_err)?;
        let owner = get_opt_i64(r, "f_owner_id").map_err(db_err)?;
        let in_trash = get_opt_i64(r, "trash_fid").map_err(db_err)?.is_some();
        match (a.file_id, owner) {
            (Some(_), Some(o)) if o != a.user_id => {
                return Err(errors::internal(format!(
                    "delete suggestion action {} references a file not owned by user {}",
                    a.id, a.user_id
                )));
            }
            (Some(_), Some(_)) if !in_trash => out.push(a),
            _ => stale.push(a.id),
        }
    }
    reset_pending(state, &stale).await?;
    Ok(out)
}

/// `RejectDeleteSuggestions`: clear the user's pending delete suggestions for `file_ids`.
/// Returns the number of actions updated.
pub async fn reject_delete_suggestions(
    state: &AppState,
    user_id: i64,
    file_ids: &[i64],
) -> ApiResult<u64> {
    if file_ids.is_empty() {
        return Ok(0);
    }
    let sql = format!(
        "UPDATE collection_actions SET is_pending = FALSE, updated_at = $1
         WHERE user_id = $2 AND action = $3 AND is_pending = TRUE AND file_id IN ({})",
        ids_list(4, file_ids.len())
    );
    let q = sqlx::query(&sql)
        .bind(now_micros())
        .bind(user_id)
        .bind(action::DELETE_SUGGESTED);
    let r = bind_ids(q, file_ids)
        .execute(&state.db.pool)
        .await
        .map_err(db_err)?;
    Ok(r.rows_affected())
}
