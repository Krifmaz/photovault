//! Endpoints: /comments/*, /reactions/*, /social/*, /comments-reactions/*.
//!
//! Reference: museum `pkg/api/{comments,reactions,social}.go` and
//! `pkg/controller/social`. All routes are museum `storageAPI` (Photos/Locker apps).
//! The public-link variants are in [`crate::social::public`].

use crate::auth::StorageUser;
use crate::error::ApiResult;
use crate::social::library_sql::{self, Role};
use crate::social::{
    self, Actor, MAX_COMMENT_CIPHER_LEN, NewComment, NewReaction, REACTION_CIPHER_LEN, bad_request,
    not_found, parse_json, parse_limit, parse_opt_i64, parse_since, permission_denied, repo,
};
use crate::state::AppState;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/comments/diff", get(comments_diff))
        .route("/comments", post(create_comment))
        .route(
            "/comments/{commentID}",
            put(update_comment).delete(delete_comment),
        )
        .route("/reactions/diff", get(reactions_diff))
        .route("/reactions", put(upsert_reaction))
        .route(
            "/reactions/{reactionID}",
            axum::routing::delete(delete_reaction),
        )
        .route("/social/diff", get(social_diff))
        .route("/social/anon-profiles", get(anon_profiles))
        .route("/comments-reactions/updated-at", get(latest_updates))
}

/// Query string as a map (first value wins).
pub(crate) fn query_map(raw: Option<String>) -> HashMap<String, String> {
    let mut m = HashMap::new();
    if let Some(q) = raw {
        for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
            m.entry(k.into_owned()).or_insert_with(|| v.into_owned());
        }
    }
    m
}

fn collection_id_param(q: &HashMap<String, String>) -> ApiResult<i64> {
    match q.get("collectionID").and_then(|s| s.parse::<i64>().ok()) {
        Some(id) if id > 0 => Ok(id),
        _ => Err(bad_request()),
    }
}

fn opt(q: &HashMap<String, String>, k: &str) -> Option<String> {
    q.get(k).cloned()
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
struct CreateCommentBody {
    id: String,
    #[serde(rename = "collectionID")]
    collection_id: i64,
    #[serde(rename = "fileID")]
    file_id: Option<i64>,
    #[serde(rename = "parentCommentID")]
    parent_comment_id: Option<String>,
    cipher: String,
    nonce: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CipherBody {
    cipher: String,
    nonce: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ReactionBody {
    id: String,
    #[serde(rename = "collectionID")]
    collection_id: i64,
    #[serde(rename = "fileID")]
    file_id: Option<i64>,
    #[serde(rename = "commentID")]
    comment_id: Option<String>,
    cipher: String,
    nonce: String,
}

fn comment_cipher_ok(cipher: &str, nonce: &str) -> bool {
    !cipher.is_empty() && cipher.len() <= MAX_COMMENT_CIPHER_LEN && !nonce.is_empty()
}

async fn create_comment(
    State(st): State<AppState>,
    StorageUser(u): StorageUser,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let b: CreateCommentBody = parse_json(&body)?;
    if b.collection_id == 0 || !comment_cipher_ok(&b.cipher, &b.nonce) {
        return Err(bad_request());
    }
    library_sql::collection_role(&st.db, b.collection_id, u.user_id, u.app).await?;
    let id = social::create_comment(
        &st.db,
        &Actor::User(u.user_id),
        NewComment {
            id: b.id,
            collection_id: b.collection_id,
            file_id: b.file_id,
            parent_comment_id: b.parent_comment_id,
            cipher: b.cipher,
            nonce: b.nonce,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

async fn comments_diff(
    State(st): State<AppState>,
    StorageUser(u): StorageUser,
    RawQuery(raw): RawQuery,
) -> ApiResult<impl IntoResponse> {
    let q = query_map(raw);
    let cid = collection_id_param(&q)?;
    let since = parse_since(opt(&q, "sinceTime").as_deref())?;
    let limit = parse_limit(opt(&q, "limit").as_deref())?;
    let file_id = parse_opt_i64(opt(&q, "fileID").as_deref())?;
    library_sql::collection_role(&st.db, cid, u.user_id, u.app).await?;
    let (comments, has_more) = repo::comment_diff(&st.db, cid, since, limit, file_id).await?;
    Ok(Json(json!({ "comments": comments, "hasMore": has_more })))
}

async fn update_comment(
    State(st): State<AppState>,
    StorageUser(u): StorageUser,
    Path(comment_id): Path<String>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let b: CipherBody = parse_json(&body)?;
    if !comment_cipher_ok(&b.cipher, &b.nonce) {
        return Err(bad_request());
    }
    let c = social::comment_by_author(&st.db, &Actor::User(u.user_id), &comment_id).await?;
    library_sql::collection_role(&st.db, c.collection_id, u.user_id, u.app).await?;
    if !repo::update_comment_cipher(&st.db, &comment_id, c.collection_id, &b.cipher, &b.nonce)
        .await?
    {
        return Err(not_found());
    }
    Ok(Json(json!({})))
}

async fn delete_comment(
    State(st): State<AppState>,
    StorageUser(u): StorageUser,
    Path(comment_id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let c = repo::get_comment(&st.db, &comment_id)
        .await?
        .ok_or_else(not_found)?;
    if c.user_id != u.user_id || c.anon_user_id.is_some() {
        // Moderation: the owner or an admin of the collection may delete anyone's comment.
        let role = library_sql::collection_role(&st.db, c.collection_id, u.user_id, u.app).await?;
        if role < Role::Admin {
            return Err(permission_denied());
        }
    }
    if !repo::soft_delete_comment(&st.db, &comment_id).await? {
        return Err(not_found());
    }
    Ok(Json(json!({})))
}

async fn upsert_reaction(
    State(st): State<AppState>,
    StorageUser(u): StorageUser,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let b: ReactionBody = parse_json(&body)?;
    if b.collection_id == 0 || b.cipher.len() != REACTION_CIPHER_LEN || b.nonce.is_empty() {
        return Err(bad_request());
    }
    library_sql::collection_role(&st.db, b.collection_id, u.user_id, u.app).await?;
    let id = social::upsert_reaction(
        &st.db,
        &Actor::User(u.user_id),
        NewReaction {
            id: b.id,
            collection_id: b.collection_id,
            file_id: b.file_id,
            comment_id: b.comment_id,
            cipher: b.cipher,
            nonce: b.nonce,
        },
    )
    .await?;
    Ok(Json(json!({ "id": id })))
}

async fn reactions_diff(
    State(st): State<AppState>,
    StorageUser(u): StorageUser,
    RawQuery(raw): RawQuery,
) -> ApiResult<impl IntoResponse> {
    let q = query_map(raw);
    let cid = collection_id_param(&q)?;
    let since = parse_since(opt(&q, "sinceTime").as_deref())?;
    let limit = parse_limit(opt(&q, "limit").as_deref())?;
    let file_id = parse_opt_i64(opt(&q, "fileID").as_deref())?;
    let comment_id = opt(&q, "commentID").filter(|s| !s.is_empty());
    library_sql::collection_role(&st.db, cid, u.user_id, u.app).await?;
    let (reactions, has_more) =
        repo::reaction_diff(&st.db, cid, since, limit, file_id, comment_id.as_deref()).await?;
    Ok(Json(json!({ "reactions": reactions, "hasMore": has_more })))
}

async fn delete_reaction(
    State(st): State<AppState>,
    StorageUser(u): StorageUser,
    Path(reaction_id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let actor = Actor::User(u.user_id);
    let r = social::reaction_by_actor(&st.db, &actor, &reaction_id).await?;
    library_sql::collection_role(&st.db, r.collection_id, u.user_id, u.app).await?;
    if !repo::soft_delete_reaction(&st.db, &reaction_id, &actor).await? {
        return Err(not_found());
    }
    Ok(Json(json!({})))
}

async fn social_diff(
    State(st): State<AppState>,
    StorageUser(u): StorageUser,
    RawQuery(raw): RawQuery,
) -> ApiResult<impl IntoResponse> {
    let q = query_map(raw);
    let cid = collection_id_param(&q)?;
    let base = parse_since(opt(&q, "sinceTime").as_deref())?;
    let since_or_base = |k: &str| -> ApiResult<i64> {
        match q.get(k).map(String::as_str) {
            None | Some("") => Ok(base),
            Some(s) => parse_since(Some(s)),
        }
    };
    let comments_since = since_or_base("commentsSinceTime")?;
    let reactions_since = since_or_base("reactionsSinceTime")?;
    let limit = parse_limit(opt(&q, "limit").as_deref())?;
    let file_id = parse_opt_i64(opt(&q, "fileID").as_deref())?;
    library_sql::collection_role(&st.db, cid, u.user_id, u.app).await?;
    let (comments, more_c) =
        repo::comment_diff(&st.db, cid, comments_since, limit, file_id).await?;
    let (reactions, more_r) =
        repo::reaction_diff(&st.db, cid, reactions_since, limit, file_id, None).await?;
    Ok(Json(json!({
        "comments": comments,
        "reactions": reactions,
        "hasMoreComments": more_c,
        "hasMoreReactions": more_r,
    })))
}

async fn anon_profiles(
    State(st): State<AppState>,
    StorageUser(u): StorageUser,
    RawQuery(raw): RawQuery,
) -> ApiResult<impl IntoResponse> {
    let q = query_map(raw);
    let cid = collection_id_param(&q)?;
    library_sql::collection_role(&st.db, cid, u.user_id, u.app).await?;
    let profiles = repo::list_anon_users(&st.db, cid).await?;
    Ok(Json(json!({ "profiles": profiles })))
}

/// museum `CollectionLatestUpdate`.
#[derive(Serialize)]
struct LatestUpdate {
    #[serde(rename = "collectionID")]
    collection_id: i64,
    #[serde(rename = "commentsUpdatedAt", skip_serializing_if = "Option::is_none")]
    comments_updated_at: Option<i64>,
    #[serde(rename = "reactionsUpdatedAt", skip_serializing_if = "Option::is_none")]
    reactions_updated_at: Option<i64>,
    #[serde(
        rename = "anonProfilesUpdatedAt",
        skip_serializing_if = "Option::is_none"
    )]
    anon_profiles_updated_at: Option<i64>,
}

async fn latest_updates(
    State(st): State<AppState>,
    StorageUser(u): StorageUser,
) -> ApiResult<impl IntoResponse> {
    let ids = library_sql::accessible_collection_ids(&st.db, u.user_id, u.app).await?;
    let comments = repo::latest_update_by_collection(&st.db, "comments", &ids).await?;
    let reactions = repo::latest_update_by_collection(&st.db, "reactions", &ids).await?;
    let anon = repo::latest_update_by_collection(&st.db, "anon_users", &ids).await?;
    let mut all: std::collections::BTreeMap<i64, LatestUpdate> = Default::default();
    for (id, ts) in comments {
        all.entry(id)
            .or_insert_with(|| empty_update(id))
            .comments_updated_at = Some(ts);
    }
    for (id, ts) in reactions {
        all.entry(id)
            .or_insert_with(|| empty_update(id))
            .reactions_updated_at = Some(ts);
    }
    for (id, ts) in anon {
        all.entry(id)
            .or_insert_with(|| empty_update(id))
            .anon_profiles_updated_at = Some(ts);
    }
    // museum returns `null` (not `[]`) when there is no activity; the apps accept both.
    let updates: Option<Vec<LatestUpdate>> = if all.is_empty() {
        None
    } else {
        Some(all.into_values().collect())
    };
    Ok(Json(json!({ "updates": updates })))
}

fn empty_update(id: i64) -> LatestUpdate {
    LatestUpdate {
        collection_id: id,
        comments_updated_at: None,
        reactions_updated_at: None,
        anon_profiles_updated_at: None,
    }
}
