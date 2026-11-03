//! SQL for `comments`, `reactions` and `anon_users` (museum `pkg/repo/social`).
//!
//! museum's triggers are done here instead: `updated_at` is set explicitly, deleted rows
//! have cipher/nonce nulled, and soft-deleting a comment soft-deletes its reactions in
//! the same transaction.

use super::{Actor, AnonUser, Comment, Reaction};
use crate::db::{Db, now_micros};
use crate::error::ApiResult;
use sqlx::Row;
use sqlx::any::AnyRow;
use std::collections::BTreeMap;

const COMMENT_COLS: &str = "id, collection_id, file_id, parent_comment_id, user_id, anon_user_id, cipher, nonce, CAST(CASE WHEN is_deleted THEN 1 ELSE 0 END AS BIGINT), created_at, updated_at";
const REACTION_COLS: &str = "id, collection_id, file_id, comment_id, user_id, anon_user_id, cipher, nonce, CAST(CASE WHEN is_deleted THEN 1 ELSE 0 END AS BIGINT), created_at, updated_at";

fn comment_from_row(r: &AnyRow) -> Result<Comment, sqlx::Error> {
    Ok(Comment {
        id: r.try_get(0)?,
        collection_id: r.try_get(1)?,
        file_id: r.try_get(2)?,
        parent_comment_id: r.try_get(3)?,
        user_id: r.try_get(4)?,
        anon_user_id: r.try_get(5)?,
        cipher: r.try_get::<Option<String>, _>(6)?.unwrap_or_default(),
        nonce: r.try_get::<Option<String>, _>(7)?.unwrap_or_default(),
        is_deleted: r.try_get::<i64, _>(8)? != 0,
        created_at: r.try_get(9)?,
        updated_at: r.try_get(10)?,
    })
}

fn reaction_from_row(r: &AnyRow) -> Result<Reaction, sqlx::Error> {
    Ok(Reaction {
        id: r.try_get(0)?,
        collection_id: r.try_get(1)?,
        file_id: r.try_get(2)?,
        comment_id: r.try_get(3)?,
        user_id: r.try_get(4)?,
        anon_user_id: r.try_get(5)?,
        cipher: r.try_get::<Option<String>, _>(6)?.unwrap_or_default(),
        nonce: r.try_get::<Option<String>, _>(7)?.unwrap_or_default(),
        is_deleted: r.try_get::<i64, _>(8)? != 0,
        created_at: r.try_get(9)?,
        updated_at: r.try_get(10)?,
    })
}

// ---- comments ----

/// Insert a new comment.
pub async fn insert_comment(db: &Db, c: &Comment) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO comments (id, collection_id, file_id, parent_comment_id, user_id, anon_user_id, cipher, nonce, is_deleted, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, FALSE, $9, $10)",
    )
    .bind(&c.id)
    .bind(c.collection_id)
    .bind(c.file_id)
    .bind(c.parent_comment_id.as_deref())
    .bind(c.user_id)
    .bind(c.anon_user_id.as_deref())
    .bind(&c.cipher)
    .bind(&c.nonce)
    .bind(c.created_at)
    .bind(c.updated_at)
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Fetch a comment by id (including tombstones).
pub async fn get_comment(db: &Db, id: &str) -> ApiResult<Option<Comment>> {
    let row = sqlx::query(&format!(
        "SELECT {COMMENT_COLS} FROM comments WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&db.pool)
    .await?;
    Ok(row.as_ref().map(comment_from_row).transpose()?)
}

/// Replace a live comment's ciphertext. `false` when no live comment matched.
pub async fn update_comment_cipher(
    db: &Db,
    id: &str,
    collection_id: i64,
    cipher: &str,
    nonce: &str,
) -> ApiResult<bool> {
    let res = sqlx::query(
        "UPDATE comments SET cipher = $1, nonce = $2, updated_at = $3 \
         WHERE id = $4 AND collection_id = $5 AND is_deleted = FALSE",
    )
    .bind(cipher)
    .bind(nonce)
    .bind(now_micros())
    .bind(id)
    .bind(collection_id)
    .execute(&db.pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Soft-delete a live comment and its reactions. `false` when no live comment matched.
pub async fn soft_delete_comment(db: &Db, id: &str) -> ApiResult<bool> {
    let now = now_micros();
    let mut tx = db.pool.begin().await?;
    let res = sqlx::query(
        "UPDATE comments SET is_deleted = TRUE, cipher = NULL, nonce = NULL, updated_at = $1 \
         WHERE id = $2 AND is_deleted = FALSE",
    )
    .bind(now)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query(
        "UPDATE reactions SET is_deleted = TRUE, cipher = NULL, nonce = NULL, updated_at = $1 \
         WHERE comment_id = $2 AND is_deleted = FALSE",
    )
    .bind(now)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Soft-delete all of a user's comments and reactions in a collection (plus reactions on
/// those comments).
pub async fn soft_delete_by_collection_and_user(
    db: &Db,
    collection_id: i64,
    user_id: i64,
) -> ApiResult<()> {
    let now = now_micros();
    let mut tx = db.pool.begin().await?;
    sqlx::query(
        "UPDATE reactions SET is_deleted = TRUE, cipher = NULL, nonce = NULL, updated_at = $1 \
         WHERE is_deleted = FALSE AND comment_id IN \
         (SELECT id FROM comments WHERE collection_id = $2 AND user_id = $3 AND is_deleted = FALSE)",
    )
    .bind(now)
    .bind(collection_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    for table in ["comments", "reactions"] {
        sqlx::query(&format!(
            "UPDATE {table} SET is_deleted = TRUE, cipher = NULL, nonce = NULL, updated_at = $1 \
             WHERE collection_id = $2 AND user_id = $3 AND is_deleted = FALSE"
        ))
        .bind(now)
        .bind(collection_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Comments of a collection changed after `since`, oldest first; `(page, has_more)`.
pub async fn comment_diff(
    db: &Db,
    collection_id: i64,
    since: i64,
    limit: i64,
    file_id: Option<i64>,
) -> ApiResult<(Vec<Comment>, bool)> {
    let file_clause = if file_id.is_some() {
        "AND file_id = $3"
    } else {
        ""
    };
    let sql = format!(
        "SELECT {COMMENT_COLS} FROM comments WHERE collection_id = $1 AND updated_at > $2 {file_clause} \
         ORDER BY updated_at ASC, id ASC LIMIT {}",
        limit + 1
    );
    let mut q = sqlx::query(&sql).bind(collection_id).bind(since);
    if let Some(f) = file_id {
        q = q.bind(f);
    }
    let rows = q.fetch_all(&db.pool).await?;
    let mut out = rows
        .iter()
        .map(comment_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let more = out.len() as i64 > limit;
    out.truncate(limit as usize);
    Ok((out, more))
}

/// Distinct signed-in users with live comments in a collection.
pub async fn active_comment_user_ids(db: &Db, collection_id: i64) -> ApiResult<Vec<i64>> {
    active_user_ids(db, "comments", collection_id).await
}

async fn active_user_ids(db: &Db, table: &str, collection_id: i64) -> ApiResult<Vec<i64>> {
    let rows: Vec<(i64,)> = sqlx::query_as(&format!(
        "SELECT DISTINCT user_id FROM {table} WHERE collection_id = $1 AND user_id > 0 AND is_deleted = FALSE"
    ))
    .bind(collection_id)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// `MAX(updated_at)` per collection of `table` (comments, reactions or anon_users).
pub async fn latest_update_by_collection(
    db: &Db,
    table: &str,
    collection_ids: &[i64],
) -> ApiResult<BTreeMap<i64, i64>> {
    let mut out = BTreeMap::new();
    for chunk in collection_ids.chunks(500) {
        let ph = (1..=chunk.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT collection_id, MAX(updated_at) FROM {table} WHERE collection_id IN ({ph}) GROUP BY collection_id"
        );
        let mut q = sqlx::query_as::<_, (i64, i64)>(&sql);
        for id in chunk {
            q = q.bind(*id);
        }
        for (cid, ts) in q.fetch_all(&db.pool).await? {
            out.insert(cid, ts);
        }
    }
    Ok(out)
}

// ---- reactions ----

/// Insert or revive the actor's reaction on a target; returns the stored row's id.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_reaction(
    db: &Db,
    id: &str,
    collection_id: i64,
    file_id: Option<i64>,
    comment_id: Option<&str>,
    actor: &Actor,
    unique_key: &str,
    cipher: &str,
    nonce: &str,
) -> ApiResult<String> {
    let now = now_micros();
    let (rid,): (String,) = sqlx::query_as(
        "INSERT INTO reactions (id, collection_id, file_id, comment_id, user_id, anon_user_id, actor_key, unique_key, cipher, nonce, is_deleted, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, FALSE, $11, $11) \
         ON CONFLICT (actor_key, unique_key) DO UPDATE SET cipher = excluded.cipher, nonce = excluded.nonce, \
         is_deleted = FALSE, updated_at = excluded.updated_at \
         RETURNING id",
    )
    .bind(id)
    .bind(collection_id)
    .bind(file_id)
    .bind(comment_id)
    .bind(actor.user_id())
    .bind(actor.anon_id())
    .bind(actor.actor_key())
    .bind(unique_key)
    .bind(cipher)
    .bind(nonce)
    .bind(now)
    .fetch_one(&db.pool)
    .await?;
    Ok(rid)
}

/// Fetch a reaction by id (including tombstones).
pub async fn get_reaction(db: &Db, id: &str) -> ApiResult<Option<Reaction>> {
    let row = sqlx::query(&format!(
        "SELECT {REACTION_COLS} FROM reactions WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&db.pool)
    .await?;
    Ok(row.as_ref().map(reaction_from_row).transpose()?)
}

/// Soft-delete the actor's reaction. `false` when nothing matched.
pub async fn soft_delete_reaction(db: &Db, id: &str, actor: &Actor) -> ApiResult<bool> {
    let res = sqlx::query(
        "UPDATE reactions SET is_deleted = TRUE, cipher = NULL, nonce = NULL, updated_at = $1 \
         WHERE id = $2 AND actor_key = $3",
    )
    .bind(now_micros())
    .bind(id)
    .bind(actor.actor_key())
    .execute(&db.pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Reactions of a collection changed after `since`. With `file_id`, only reactions on
/// that file or on comments of that file; with `comment_id`, only that comment's.
pub async fn reaction_diff(
    db: &Db,
    collection_id: i64,
    since: i64,
    limit: i64,
    file_id: Option<i64>,
    comment_id: Option<&str>,
) -> ApiResult<(Vec<Reaction>, bool)> {
    let mut clauses = String::new();
    let mut n = 2;
    if file_id.is_some() {
        n += 1;
        clauses.push_str(&format!(
            " AND (file_id = ${n} OR comment_id IN (SELECT id FROM comments WHERE collection_id = $1 AND file_id = ${n}))"
        ));
    }
    if comment_id.is_some() {
        n += 1;
        clauses.push_str(&format!(" AND comment_id = ${n}"));
    }
    let sql = format!(
        "SELECT {REACTION_COLS} FROM reactions WHERE collection_id = $1 AND updated_at > $2{clauses} \
         ORDER BY updated_at ASC, id ASC LIMIT {}",
        limit + 1
    );
    let mut q = sqlx::query(&sql).bind(collection_id).bind(since);
    if let Some(f) = file_id {
        q = q.bind(f);
    }
    if let Some(c) = comment_id {
        q = q.bind(c);
    }
    let rows = q.fetch_all(&db.pool).await?;
    let mut out = rows
        .iter()
        .map(reaction_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let more = out.len() as i64 > limit;
    out.truncate(limit as usize);
    Ok((out, more))
}

/// Distinct signed-in users with live reactions in a collection.
pub async fn active_reaction_user_ids(db: &Db, collection_id: i64) -> ApiResult<Vec<i64>> {
    active_user_ids(db, "reactions", collection_id).await
}

// ---- anon users ----

/// Insert an anonymous identity.
pub async fn insert_anon_user(db: &Db, u: &AnonUser) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO anon_users (id, collection_id, cipher, nonce, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&u.id)
    .bind(u.collection_id)
    .bind(&u.cipher)
    .bind(&u.nonce)
    .bind(u.created_at)
    .bind(u.updated_at)
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// The collection an anonymous identity belongs to.
pub async fn anon_user_collection(db: &Db, id: &str) -> ApiResult<Option<i64>> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT collection_id FROM anon_users WHERE id = $1")
        .bind(id)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row.map(|r| r.0))
}

/// All anonymous identities of a collection, oldest first. `None` when there are none
/// (museum returns `null` there, which the apps accept).
pub async fn list_anon_users(db: &Db, collection_id: i64) -> ApiResult<Option<Vec<AnonUser>>> {
    let rows: Vec<(String, i64, String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, collection_id, cipher, nonce, created_at, updated_at FROM anon_users \
         WHERE collection_id = $1 ORDER BY created_at ASC, id ASC",
    )
    .bind(collection_id)
    .fetch_all(&db.pool)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        rows.into_iter()
            .map(
                |(id, collection_id, cipher, nonce, created_at, updated_at)| AnonUser {
                    id,
                    collection_id,
                    cipher,
                    nonce,
                    created_at,
                    updated_at,
                },
            )
            .collect(),
    ))
}
