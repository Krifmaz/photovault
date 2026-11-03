//! Comments, reactions and anonymous identities on collections ("social").
//!
//! Mirrors museum's `pkg/controller/social`, `pkg/controller/public/{comments,reactions,
//! anon_identity}.go` and `pkg/repo/social`. Everything stored here is ciphertext
//! encrypted by the apps with the collection key.
//!
//! * [`repo`]: SQL over the `comments`, `reactions` and `anon_users` tables (migration 0350).
//! * [`library_sql`]: the only place that reads other workstreams' tables
//!   (collections, shares, collection files, users).
//! * [`anon_token`]: HS256 tokens for anonymous public-link identities.
//! * [`public`]: handlers for the `/public-collection/*` variants.
//!
//! The private HTTP handlers live in `crate::routes::social`.

pub mod anon_token;
pub mod library_sql;
pub mod public;
pub mod repo;

use crate::db::Db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::http::StatusCode;
use serde::Serialize;

/// Max length (base64 chars) of a comment cipher (museum `maxCommentPayloadBytes`).
pub const MAX_COMMENT_CIPHER_LEN: usize = 20 * 1024;
/// Exact length of a reaction cipher: 100 padded bytes + 16 MAC, base64 encoded.
pub const REACTION_CIPHER_LEN: usize = 156;
/// Default page size of the diff endpoints.
pub const DIFF_DEFAULT_LIMIT: i64 = 1000;
/// Max page size of the diff endpoints.
pub const DIFF_MAX_LIMIT: i64 = 2000;

/// A comment as returned to clients (museum `ente/social.Comment`).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    /// `cmt_` + 21-char nanoid.
    pub id: String,
    /// Collection the comment belongs to.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// File within the collection; absent for collection-level comments.
    #[serde(rename = "fileID", skip_serializing_if = "Option::is_none")]
    pub file_id: Option<i64>,
    /// Comment this one replies to.
    #[serde(rename = "parentCommentID", skip_serializing_if = "Option::is_none")]
    pub parent_comment_id: Option<String>,
    /// Author; `-1` for anonymous authors.
    #[serde(rename = "userID")]
    pub user_id: i64,
    /// Anonymous author id.
    #[serde(rename = "anonUserID", skip_serializing_if = "Option::is_none")]
    pub anon_user_id: Option<String>,
    /// Encrypted text (empty once deleted, then omitted).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cipher: String,
    /// Nonce for `cipher` (empty once deleted, then omitted).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub nonce: String,
    /// Soft-deleted tombstone.
    pub is_deleted: bool,
    /// Creation time (µs).
    pub created_at: i64,
    /// Last change (µs); the diff cursor.
    pub updated_at: i64,
}

/// A reaction as returned to clients (museum `ente/social.Reaction`).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reaction {
    /// `rct_` + 21-char nanoid.
    pub id: String,
    /// Collection the reaction belongs to.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// Target file (absent for collection or comment reactions).
    #[serde(rename = "fileID", skip_serializing_if = "Option::is_none")]
    pub file_id: Option<i64>,
    /// Target comment.
    #[serde(rename = "commentID", skip_serializing_if = "Option::is_none")]
    pub comment_id: Option<String>,
    /// Reactor; `-1` for anonymous reactors.
    #[serde(rename = "userID")]
    pub user_id: i64,
    /// Anonymous reactor id.
    #[serde(rename = "anonUserID", skip_serializing_if = "Option::is_none")]
    pub anon_user_id: Option<String>,
    /// Encrypted, padded reaction (omitted once deleted).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cipher: String,
    /// Nonce for `cipher` (omitted once deleted).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub nonce: String,
    /// Soft-deleted tombstone.
    pub is_deleted: bool,
    /// Creation time (µs).
    pub created_at: i64,
    /// Last change (µs); the diff cursor.
    pub updated_at: i64,
}

/// An anonymous public-link identity (museum `ente/social.AnonUser`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnonUser {
    /// `anon_` + 21-char nanoid.
    #[serde(rename = "anonUserID")]
    pub id: String,
    /// The only collection this identity may act in.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// Encrypted display name.
    pub cipher: String,
    /// Nonce for `cipher`.
    pub nonce: String,
    /// Creation time (µs).
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    /// Last change (µs).
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

/// Who is acting: a signed-in user, or an anonymous public-link identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// A real account.
    User(i64),
    /// An `anon_users` id.
    Anon(String),
}

impl Actor {
    /// The `user_id` column value (`-1` for anonymous).
    pub fn user_id(&self) -> i64 {
        match self {
            Actor::User(id) => *id,
            Actor::Anon(_) => -1,
        }
    }
    /// The `anon_user_id` column value.
    pub fn anon_id(&self) -> Option<&str> {
        match self {
            Actor::User(_) => None,
            Actor::Anon(a) => Some(a),
        }
    }
    /// museum's reaction `actor_key`.
    pub fn actor_key(&self) -> String {
        match self {
            Actor::User(id) => format!("U:{id}"),
            Actor::Anon(a) => format!("A:{a}"),
        }
    }
}

// ---- errors (museum's untyped errors serialise as `{}` with a status) ----

pub(crate) fn bad_request() -> ApiError {
    ApiError::status(StatusCode::BAD_REQUEST)
}
pub(crate) fn not_found() -> ApiError {
    ApiError::status(StatusCode::NOT_FOUND)
}
pub(crate) fn permission_denied() -> ApiError {
    ApiError::status(StatusCode::FORBIDDEN)
}
pub(crate) fn auth_required() -> ApiError {
    ApiError::status(StatusCode::UNAUTHORIZED)
}

// ---- ids ----

const NANOID_ALPHABET: &[u8; 64] =
    b"_-0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const NANOID_LEN: usize = 21;

/// A 21-character nanoid (64-symbol URL-safe alphabet, like `gonanoid.New()`).
pub fn nanoid() -> String {
    crate::crypto::random_bytes(NANOID_LEN)
        .into_iter()
        .map(|b| NANOID_ALPHABET[(b & 63) as usize] as char)
        .collect()
}

/// Accept a client-supplied `<prefix><21 nanoid chars>` id, or generate one when empty.
pub fn normalize_id(prefix: &str, id: &str) -> ApiResult<String> {
    if id.is_empty() {
        return Ok(format!("{prefix}{}", nanoid()));
    }
    let Some(rest) = id.strip_prefix(prefix) else {
        return Err(bad_request());
    };
    if rest.len() != NANOID_LEN
        || !rest
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(bad_request());
    }
    Ok(id.to_string())
}

// ---- diff query parsing (museum pkg/api/diff_utils.go) ----

/// Parse `sinceTime` (empty → 0).
pub fn parse_since(raw: Option<&str>) -> ApiResult<i64> {
    match raw {
        None | Some("") => Ok(0),
        Some(s) => s.parse().map_err(|_| bad_request()),
    }
}

/// Parse `limit` (empty → 1000, ≤0 → 400, capped at 2000).
pub fn parse_limit(raw: Option<&str>) -> ApiResult<i64> {
    match raw {
        None | Some("") => Ok(DIFF_DEFAULT_LIMIT),
        Some(s) => {
            let v: i64 = s.parse().map_err(|_| bad_request())?;
            if v <= 0 {
                return Err(bad_request());
            }
            Ok(v.min(DIFF_MAX_LIMIT))
        }
    }
}

/// Parse an optional int64 (empty → None).
pub fn parse_opt_i64(raw: Option<&str>) -> ApiResult<Option<i64>> {
    match raw {
        None | Some("") => Ok(None),
        Some(s) => s.parse().map(Some).map_err(|_| bad_request()),
    }
}

/// Parse a JSON body like gin's `ShouldBindJSON` (any content type; failures are `400 {}`).
/// An empty body is treated as `{}`.
pub fn parse_json<T: serde::de::DeserializeOwned>(body: &[u8]) -> ApiResult<T> {
    let body = if body.iter().all(|b| b.is_ascii_whitespace()) {
        b"{}".as_slice()
    } else {
        body
    };
    serde_json::from_slice(body).map_err(|_| bad_request())
}

// ---- core operations shared by the private and public handlers ----

/// Input for [`create_comment`].
#[derive(Debug, Clone)]
pub struct NewComment {
    /// Client id or empty.
    pub id: String,
    /// Target collection.
    pub collection_id: i64,
    /// Target file.
    pub file_id: Option<i64>,
    /// Reply-to comment.
    pub parent_comment_id: Option<String>,
    /// Ciphertext.
    pub cipher: String,
    /// Nonce.
    pub nonce: String,
}

/// museum `CommentsController.Create` after the access check.
pub async fn create_comment(state: &AppState, actor: &Actor, req: NewComment) -> ApiResult<String> {
    let db = &state.db;
    let id = normalize_id("cmt_", &req.id)?;
    if req.cipher.is_empty() || req.nonce.is_empty() {
        return Err(bad_request());
    }
    if let Some(fid) = req.file_id {
        library_sql::require_file_in_collection(state, req.collection_id, fid).await?;
    }
    if let Some(pid) = &req.parent_comment_id {
        let p = repo::get_comment(db, pid).await?.ok_or_else(not_found)?;
        // Replies stay in the parent's collection and file scope, and only to live comments.
        if p.collection_id != req.collection_id || p.is_deleted || p.file_id != req.file_id {
            return Err(bad_request());
        }
    }
    let now = crate::db::now_micros();
    repo::insert_comment(
        db,
        &Comment {
            id: id.clone(),
            collection_id: req.collection_id,
            file_id: req.file_id,
            parent_comment_id: req.parent_comment_id,
            user_id: actor.user_id(),
            anon_user_id: actor.anon_id().map(str::to_string),
            cipher: req.cipher,
            nonce: req.nonce,
            is_deleted: false,
            created_at: now,
            updated_at: now,
        },
    )
    .await?;
    Ok(id)
}

/// Load a comment and check that `actor` wrote it (museum `UpdatePayload` / `Delete` checks).
pub async fn comment_by_author(db: &Db, actor: &Actor, comment_id: &str) -> ApiResult<Comment> {
    let c = repo::get_comment(db, comment_id)
        .await?
        .ok_or_else(not_found)?;
    let is_author = match actor {
        Actor::User(id) => c.user_id == *id && c.anon_user_id.is_none(),
        Actor::Anon(a) => c.anon_user_id.as_deref() == Some(a.as_str()),
    };
    if !is_author {
        return Err(permission_denied());
    }
    Ok(c)
}

/// Input for [`upsert_reaction`].
#[derive(Debug, Clone)]
pub struct NewReaction {
    /// Client id or empty.
    pub id: String,
    /// Target collection.
    pub collection_id: i64,
    /// Target file (or context of the target comment).
    pub file_id: Option<i64>,
    /// Target comment.
    pub comment_id: Option<String>,
    /// Ciphertext.
    pub cipher: String,
    /// Nonce.
    pub nonce: String,
}

/// museum `ReactionsController.Upsert` after the access check. Returns the id of the
/// (possibly pre-existing) reaction of this actor on this target.
pub async fn upsert_reaction(state: &AppState, actor: &Actor, req: NewReaction) -> ApiResult<String> {
    let db = &state.db;
    let id = normalize_id("rct_", &req.id)?;
    if req.cipher.is_empty() || req.nonce.is_empty() {
        return Err(bad_request());
    }
    if let Some(fid) = req.file_id {
        library_sql::require_file_in_collection(state, req.collection_id, fid).await?;
    }
    let mut file_id = req.file_id;
    if let Some(cid) = &req.comment_id {
        let c = repo::get_comment(db, cid).await?.ok_or_else(not_found)?;
        if c.collection_id != req.collection_id || c.is_deleted || c.file_id != req.file_id {
            return Err(bad_request());
        }
        // Reactions on comments never carry a file id; the request's one only validated context.
        file_id = None;
    }
    let unique_key = match (&req.comment_id, file_id) {
        (Some(c), _) => format!("M:{c}"),
        (None, Some(f)) => format!("F:{}/{f}", req.collection_id),
        (None, None) => format!("C:{}", req.collection_id),
    };
    repo::upsert_reaction(
        db,
        &id,
        req.collection_id,
        file_id,
        req.comment_id.as_deref(),
        actor,
        &unique_key,
        &req.cipher,
        &req.nonce,
    )
    .await
}

/// Load a reaction and check that `actor` made it.
pub async fn reaction_by_actor(db: &Db, actor: &Actor, reaction_id: &str) -> ApiResult<Reaction> {
    let r = repo::get_reaction(db, reaction_id)
        .await?
        .ok_or_else(not_found)?;
    let mine = match actor {
        Actor::User(id) => r.user_id == *id && r.anon_user_id.is_none(),
        Actor::Anon(a) => r.anon_user_id.as_deref() == Some(a.as_str()),
    };
    if !mine {
        return Err(permission_denied());
    }
    Ok(r)
}

/// Soft-delete everything `user_id` wrote in `collection_id` (museum
/// `removeUserSocialActivity`). For the collections workstream to call when a sharee
/// leaves or is removed from a collection.
pub async fn remove_user_social_activity(
    db: &Db,
    collection_id: i64,
    user_id: i64,
) -> ApiResult<()> {
    repo::soft_delete_by_collection_and_user(db, collection_id, user_id).await
}
