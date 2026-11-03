//! Public-link (`/public-collection/*`) comments, reactions and anonymous identities.
//!
//! Reference: museum `pkg/api/public_comments.go`, `pkg/controller/public/{actor,
//! comments,reactions,anon_identity,anon_user_validation,social_limits}.go`.
//!
//! # Integration contract (for the `/public-collection` middleware owner)
//!
//! 1. Mount: [`public_router`] defines paths **relative to `/public-collection`**
//!    (`/social/diff`, `/comments/diff`, `/comments`, `/comments/{commentID}`,
//!    `/reactions/diff`, `/reactions`, `/reactions/{reactionID}`,
//!    `/participants/masked-emails`, `/anon-profiles`, `/anon-identity`). Merge it into
//!    the router you nest at `/public-collection` **before** applying your access-token
//!    middleware, e.g.
//!    `Router::new().route("/diff", …).merge(crate::social::public::public_router())
//!    .route_layer(from_fn_with_state(state, your_mw))` and then
//!    `.nest("/public-collection", that)`. (Nesting two routers at the same prefix
//!    panics in axum, so merge instead.)
//! 2. Your middleware must, exactly like museum's `CollectionLinkMiddleware`, validate the
//!    link (`X-Auth-Access-Token` / `?accessToken=`, expiry, device limit, password JWT
//!    `X-Auth-Access-Token-JWT`, …) and reject the request itself on failure. On success
//!    it must insert a [`PublicCollectionCtx`] into the request extensions
//!    (`req.extensions_mut().insert(ctx)`). If the extension is missing these handlers
//!    answer `401 {}`.
//! 3. `enable_comment` must be the link's `enable_comment` flag: when false every
//!    handler here answers `405 {"code":"PUBLIC_COMMENT_DISABLED",…}` like museum.
//! 4. You do **not** need to handle `X-Anon-User-Token` (museum's `attachAnonIdentity`):
//!    the anonymous identity tables belong to this module, so these handlers verify that
//!    header themselves.
//!
//! # Who is acting (museum `resolvePublicActor`)
//!
//! In order: a valid `X-Anon-User-Token` header → that anonymous identity; else an
//! `X-Auth-Token` header / `?token=` → that signed-in user (no collection access check,
//! as in museum: having the link is enough); else `X-Anon-User-ID` header or body
//! `anonUserID`, plus `Authorization: Bearer <anonToken>` or body `anonToken`. Anonymous
//! identities must belong to the link's collection (else 403; unknown → 401).

use super::{
    Actor, AnonUser, MAX_COMMENT_CIPHER_LEN, NewComment, NewReaction, REACTION_CIPHER_LEN,
    anon_token, auth_required, bad_request, library_sql, nanoid, not_found, parse_json,
    parse_limit, parse_opt_i64, parse_since, permission_denied, repo,
};
use crate::auth::{App, hash_token, token_from_parts};
use crate::error::{ApiError, ApiResult};
use crate::routes::social::query_map;
use crate::state::AppState;
use axum::extract::{FromRequestParts, Path, RawQuery, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Request extension set by the `/public-collection` middleware after it has validated
/// the public link (museum `ente.PublicAccessContext`, plus the owner id).
#[derive(Debug, Clone)]
pub struct PublicCollectionCtx {
    /// Row id of the public link (`public_collection_tokens.id`).
    pub id: i64,
    /// The shared collection.
    pub collection_id: i64,
    /// The collection owner's user id.
    pub owner_id: i64,
    /// Whether the owner enabled comments/reactions on this link.
    pub enable_comment: bool,
    /// Client IP as seen by the middleware.
    pub ip: String,
    /// Client user agent.
    pub user_agent: String,
}

/// Extractor for [`PublicCollectionCtx`] plus the raw request parts needed to resolve the
/// actor (headers, query).
struct Public {
    ctx: PublicCollectionCtx,
    headers: axum::http::HeaderMap,
    token: Option<String>,
    app: App,
}

impl FromRequestParts<AppState> for Public {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &AppState) -> Result<Self, ApiError> {
        let ctx = parts
            .extensions
            .get::<PublicCollectionCtx>()
            .cloned()
            .ok_or_else(auth_required)?;
        Ok(Public {
            ctx,
            headers: parts.headers.clone(),
            token: token_from_parts(parts),
            app: App::from_parts(parts),
        })
    }
}

impl Public {
    fn ensure_enabled(&self) -> ApiResult<()> {
        if self.ctx.enable_comment {
            Ok(())
        } else {
            Err(ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "PUBLIC_COMMENT_DISABLED",
                "User has not enabled public comments for this url",
            ))
        }
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// The signed-in user behind `X-Auth-Token` / `?token=`, if a token was sent.
    async fn token_user(&self, st: &AppState) -> ApiResult<Option<i64>> {
        let Some(token) = self.token.as_deref().filter(|t| !t.is_empty()) else {
            return Ok(None);
        };
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT user_id FROM tokens WHERE token_hash = $1 AND app = $2 AND is_deleted = FALSE",
        )
        .bind(hash_token(token))
        .bind(self.app.as_str())
        .fetch_optional(&st.db.pool)
        .await?;
        row.map(|r| Some(r.0)).ok_or_else(auth_required)
    }

    /// museum `resolvePublicActor(…, requireIdentity = true)` + `ensureAnonUserForCollection`.
    async fn actor(&self, st: &AppState, anon: &AnonFields) -> ApiResult<Actor> {
        let actor = if let Some(t) = self.header("X-Anon-User-Token") {
            Actor::Anon(anon_token::verify(&st.keys.jwt, &t).ok_or_else(auth_required)?)
        } else if let Some(uid) = self.token_user(st).await? {
            Actor::User(uid)
        } else {
            let anon_id = self
                .header("X-Anon-User-ID")
                .or_else(|| anon.anon_user_id.clone().filter(|s| !s.is_empty()));
            let token = self
                .header("Authorization")
                .and_then(|h| {
                    let (scheme, rest) = h.split_once(' ')?;
                    scheme
                        .eq_ignore_ascii_case("bearer")
                        .then(|| rest.trim().to_string())
                })
                .filter(|s| !s.is_empty())
                .or_else(|| Some(anon.anon_token.trim().to_string()).filter(|s| !s.is_empty()));
            let (Some(anon_id), Some(token)) = (anon_id, token) else {
                return Err(auth_required());
            };
            let sub = anon_token::verify(&st.keys.jwt, &token).ok_or_else(auth_required)?;
            if sub != anon_id {
                return Err(bad_request());
            }
            Actor::Anon(anon_id)
        };
        if let Actor::Anon(a) = &actor {
            match repo::anon_user_collection(&st.db, a).await? {
                None => return Err(auth_required()),
                Some(c) if c != self.ctx.collection_id => return Err(permission_denied()),
                Some(_) => {}
            }
        }
        Ok(actor)
    }
}

/// Routes, relative to `/public-collection` (see the module docs for how to mount them).
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/social/diff", get(social_diff))
        .route("/comments/diff", get(comments_diff))
        .route("/comments", post(create_comment))
        .route(
            "/comments/{commentID}",
            axum::routing::put(update_comment).delete(delete_comment),
        )
        .route("/reactions/diff", get(reactions_diff))
        .route("/reactions", post(create_reaction))
        .route(
            "/reactions/{reactionID}",
            axum::routing::delete(delete_reaction),
        )
        .route("/participants/masked-emails", get(participants))
        .route("/anon-profiles", get(anon_profiles))
        .route("/anon-identity", post(create_anon_identity))
}

// ---- payloads ----

#[derive(Deserialize, Default)]
#[serde(default)]
struct AnonFields {
    #[serde(rename = "anonUserID")]
    anon_user_id: Option<String>,
    #[serde(rename = "anonToken")]
    anon_token: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CommentBody {
    id: String,
    #[serde(rename = "fileID")]
    file_id: Option<i64>,
    #[serde(rename = "parentCommentID")]
    parent_comment_id: Option<String>,
    cipher: String,
    nonce: String,
    #[serde(flatten)]
    anon: AnonFields,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CommentEditBody {
    cipher: String,
    nonce: String,
    #[serde(flatten)]
    anon: AnonFields,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ReactionBody {
    id: String,
    #[serde(rename = "fileID")]
    file_id: Option<i64>,
    #[serde(rename = "commentID")]
    comment_id: Option<String>,
    cipher: String,
    nonce: String,
    #[serde(flatten)]
    anon: AnonFields,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AnonIdentityBody {
    cipher: String,
    nonce: String,
}

/// museum `social_limits.go`: secretbox MAC overhead.
const SECRETBOX_OVERHEAD: usize = 16;
/// Max plaintext bytes of a public comment.
const MAX_COMMENT_BYTES: usize = 280;
/// Max plaintext bytes of an anonymous display name.
const MAX_ANON_NAME_BYTES: usize = 50;
/// Max base64 length of a public reaction cipher accepted by the handler.
const MAX_PUBLIC_REACTION_CIPHER_LEN: usize = 2 * 1024;

/// museum `validateEncryptedPayloadLength`.
fn check_plaintext_len(cipher: &str, max: usize, code: &'static str, msg: &str) -> ApiResult<()> {
    let data = crate::crypto::unb64(cipher).map_err(|_| bad_request())?;
    if data.len() < SECRETBOX_OVERHEAD {
        return Err(bad_request());
    }
    if data.len() - SECRETBOX_OVERHEAD > max {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, code, msg));
    }
    Ok(())
}

fn check_comment_cipher(cipher: &str, nonce: &str) -> ApiResult<()> {
    if cipher.is_empty() || cipher.len() > MAX_COMMENT_CIPHER_LEN || nonce.is_empty() {
        return Err(bad_request());
    }
    check_plaintext_len(
        cipher,
        MAX_COMMENT_BYTES,
        "PUBLIC_COMMENT_TOO_LONG",
        "Comments are limited to 280 characters",
    )
}

// ---- handlers ----

async fn create_comment(
    State(st): State<AppState>,
    p: Public,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let b: CommentBody = parse_json(&body)?;
    p.ensure_enabled()?;
    check_comment_cipher(&b.cipher, &b.nonce)?;
    let actor = p.actor(&st, &b.anon).await?;
    let id = super::create_comment(
        &st,
        &actor,
        NewComment {
            id: b.id,
            collection_id: p.ctx.collection_id,
            file_id: b.file_id,
            parent_comment_id: b.parent_comment_id,
            cipher: b.cipher,
            nonce: b.nonce,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

async fn update_comment(
    State(st): State<AppState>,
    p: Public,
    Path(comment_id): Path<String>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let b: CommentEditBody = parse_json(&body)?;
    p.ensure_enabled()?;
    check_comment_cipher(&b.cipher, &b.nonce)?;
    let actor = p.actor(&st, &b.anon).await?;
    let c = super::comment_by_author(&st.db, &actor, &comment_id).await?;
    if c.collection_id != p.ctx.collection_id {
        return Err(permission_denied());
    }
    if !repo::update_comment_cipher(&st.db, &comment_id, c.collection_id, &b.cipher, &b.nonce)
        .await?
    {
        return Err(not_found());
    }
    Ok(Json(json!({})))
}

async fn delete_comment(
    State(st): State<AppState>,
    p: Public,
    Path(comment_id): Path<String>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let b: AnonFields = parse_json(&body)?;
    p.ensure_enabled()?;
    let c = if let Some(uid) = p.token_user(&st).await? {
        // A signed-in user: their own comment, or moderation as owner/admin.
        let c = repo::get_comment(&st.db, &comment_id)
            .await?
            .ok_or_else(not_found)?;
        if c.user_id != uid {
            let role = library_sql::collection_role(&st, c.collection_id, uid, p.app).await?;
            if role < library_sql::Role::Admin {
                return Err(permission_denied());
            }
        }
        c
    } else {
        let actor = p.actor(&st, &b).await?;
        super::comment_by_author(&st.db, &actor, &comment_id).await?
    };
    if c.collection_id != p.ctx.collection_id {
        return Err(permission_denied());
    }
    if !repo::soft_delete_comment(&st.db, &comment_id).await? {
        return Err(not_found());
    }
    Ok(Json(json!({})))
}

struct DiffParams {
    since: i64,
    limit: i64,
    file_id: Option<i64>,
    comment_id: Option<String>,
}

fn diff_params(raw: Option<String>) -> ApiResult<DiffParams> {
    let q = query_map(raw);
    let get = |k: &str| q.get(k).map(String::as_str);
    Ok(DiffParams {
        since: parse_since(get("sinceTime"))?,
        limit: parse_limit(get("limit"))?,
        file_id: parse_opt_i64(get("fileID"))?,
        comment_id: get("commentID")
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    })
}

async fn comments_diff(
    State(st): State<AppState>,
    p: Public,
    RawQuery(raw): RawQuery,
) -> ApiResult<impl IntoResponse> {
    p.ensure_enabled()?;
    let d = diff_params(raw)?;
    let (comments, has_more) =
        repo::comment_diff(&st.db, p.ctx.collection_id, d.since, d.limit, d.file_id).await?;
    Ok(Json(json!({ "comments": comments, "hasMore": has_more })))
}

async fn reactions_diff(
    State(st): State<AppState>,
    p: Public,
    RawQuery(raw): RawQuery,
) -> ApiResult<impl IntoResponse> {
    p.ensure_enabled()?;
    let d = diff_params(raw)?;
    let (reactions, has_more) = repo::reaction_diff(
        &st.db,
        p.ctx.collection_id,
        d.since,
        d.limit,
        d.file_id,
        d.comment_id.as_deref(),
    )
    .await?;
    Ok(Json(json!({ "reactions": reactions, "hasMore": has_more })))
}

async fn social_diff(
    State(st): State<AppState>,
    p: Public,
    RawQuery(raw): RawQuery,
) -> ApiResult<impl IntoResponse> {
    p.ensure_enabled()?;
    let d = diff_params(raw)?;
    let cid = p.ctx.collection_id;
    let (comments, more_c) = repo::comment_diff(&st.db, cid, d.since, d.limit, d.file_id).await?;
    let (reactions, more_r) =
        repo::reaction_diff(&st.db, cid, d.since, d.limit, d.file_id, None).await?;
    Ok(Json(json!({
        "comments": comments,
        "reactions": reactions,
        "hasMoreComments": more_c,
        "hasMoreReactions": more_r,
    })))
}

async fn create_reaction(
    State(st): State<AppState>,
    p: Public,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let b: ReactionBody = parse_json(&body)?;
    p.ensure_enabled()?;
    if b.cipher.is_empty() || b.cipher.len() > MAX_PUBLIC_REACTION_CIPHER_LEN || b.nonce.is_empty()
    {
        return Err(bad_request());
    }
    if b.cipher.len() != REACTION_CIPHER_LEN {
        return Err(bad_request());
    }
    let actor = p.actor(&st, &b.anon).await?;
    let id = super::upsert_reaction(
        &st,
        &actor,
        NewReaction {
            id: b.id,
            collection_id: p.ctx.collection_id,
            file_id: b.file_id,
            comment_id: b.comment_id,
            cipher: b.cipher,
            nonce: b.nonce,
        },
    )
    .await?;
    Ok(Json(json!({ "id": id })))
}

async fn delete_reaction(
    State(st): State<AppState>,
    p: Public,
    Path(reaction_id): Path<String>,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let b: AnonFields = parse_json(&body)?;
    p.ensure_enabled()?;
    let actor = p.actor(&st, &b).await?;
    let r = super::reaction_by_actor(&st.db, &actor, &reaction_id).await?;
    if r.collection_id != p.ctx.collection_id {
        return Err(permission_denied());
    }
    if !repo::soft_delete_reaction(&st.db, &reaction_id, &actor).await? {
        return Err(not_found());
    }
    Ok(Json(json!({})))
}

#[derive(Serialize)]
struct Participant {
    #[serde(rename = "userID")]
    user_id: i64,
    #[serde(rename = "emailMasked")]
    email_masked: String,
}

/// museum `GetMaskedEmailForPublic`: `john@example.io` → `jo**@e********o`.
pub fn mask_email_for_public(email: &str) -> String {
    let email = email.trim();
    let Some(at) = email.rfind('@') else {
        return "[invalid_email]".into();
    };
    if at == 0 || at == email.len() - 1 {
        return "[invalid_email]".into();
    }
    let user: Vec<char> = email[..at].chars().collect();
    let domain: Vec<char> = email[at + 1..].chars().collect();
    let mu: String = if user.len() <= 2 {
        user.iter().collect()
    } else {
        user[..2].iter().collect::<String>() + &"*".repeat(user.len() - 2)
    };
    let md: String = if domain.len() <= 2 {
        domain.iter().collect()
    } else {
        format!(
            "{}{}{}",
            domain[0],
            "*".repeat(domain.len() - 2),
            domain[domain.len() - 1]
        )
    };
    format!("{mu}@{md}")
}

async fn participants(State(st): State<AppState>, p: Public) -> ApiResult<impl IntoResponse> {
    p.ensure_enabled()?;
    let cid = p.ctx.collection_id;
    let mut ids = repo::active_comment_user_ids(&st.db, cid).await?;
    ids.extend(repo::active_reaction_user_ids(&st.db, cid).await?);
    ids.sort_unstable();
    ids.dedup();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(email) = library_sql::user_email(&st, id).await? {
            out.push(Participant {
                user_id: id,
                email_masked: mask_email_for_public(&email),
            });
        }
    }
    Ok(Json(json!({ "participants": out })))
}

async fn anon_profiles(State(st): State<AppState>, p: Public) -> ApiResult<impl IntoResponse> {
    p.ensure_enabled()?;
    let profiles = repo::list_anon_users(&st.db, p.ctx.collection_id).await?;
    Ok(Json(json!({ "profiles": profiles })))
}

async fn create_anon_identity(
    State(st): State<AppState>,
    p: Public,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    let b: AnonIdentityBody = parse_json(&body)?;
    p.ensure_enabled()?;
    if b.cipher.trim().is_empty() || b.nonce.trim().is_empty() {
        return Err(bad_request());
    }
    check_plaintext_len(
        &b.cipher,
        MAX_ANON_NAME_BYTES,
        "ANON_NAME_TOO_LONG",
        "Anonymous names are limited to 50 characters",
    )?;
    let now = crate::db::now_micros();
    let user = AnonUser {
        id: format!("anon_{}", nanoid()),
        collection_id: p.ctx.collection_id,
        cipher: b.cipher,
        nonce: b.nonce,
        created_at: now,
        updated_at: now,
    };
    repo::insert_anon_user(&st.db, &user).await?;
    let (token, expires_at) = anon_token::issue(&st.keys.jwt, &user.id);
    Ok((
        StatusCode::CREATED,
        Json(json!({ "anonUserID": user.id, "token": token, "expiresAt": expires_at })),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn masks_like_museum() {
        assert_eq!(
            super::mask_email_for_public("john@example.io"),
            "jo**@e********o"
        );
        assert_eq!(super::mask_email_for_public("ab@cd"), "ab@cd");
        assert_eq!(super::mask_email_for_public("x@"), "[invalid_email]");
    }
}
