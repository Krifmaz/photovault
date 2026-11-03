//! Authentication for public album links, single-file links and memory shares,
//! mirroring museum's `CollectionLinkMiddleware`, `FileLinkMiddleware` and
//! `MemoryShareMiddleware` (as axum extractors).
//!
//! Not carried over from museum: the link cache, the owner-subscription and
//! custom-domain origin checks (self-hosted has no billing), Cloudflare-worker
//! IP exemptions, and the anonymous social identity (owned by the social routes).

use super::http::{
    client_ip, header_or_query, header_value, json_response, route_path, user_agent,
};
use super::jwt;
use super::memory::{self, MEMORY_SHARE_TTL};
use super::models::MemoryShare;
use crate::db::now_micros;
use crate::library::links;
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::Response;
use serde_json::json;
use sqlx::AnyPool;

/// museum `DeviceLimitThreshold`: the "unlimited-looking" 50 limit really means 500.
const DEVICE_LIMIT_THRESHOLD: i64 = 50;
const DEVICE_LIMIT_THRESHOLD_MULTIPLIER: i64 = 10;

fn internal(e: impl std::fmt::Display) -> Response {
    tracing::error!(error = %e, "public link lookup failed");
    json_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({"error": "something went wrong"}),
    )
}

fn missing_token(context: &str) -> Response {
    json_response(
        StatusCode::UNAUTHORIZED,
        json!({"error": "missing accessToken", "context": context}),
    )
}

fn invalid_token() -> Response {
    json_response(StatusCode::UNAUTHORIZED, json!({"error": "invalid token"}))
}

fn gone(code: &str, msg: &str) -> Response {
    json_response(StatusCode::GONE, json!({"code": code, "error": msg}))
}

fn expired(valid_till: i64) -> bool {
    valid_till > 0 && valid_till < now_micros()
}

fn device_limit_exceeded() -> Response {
    json_response(
        StatusCode::FORBIDDEN,
        json!({"code": "LINK_DEVICE_LIMIT_EXCEEDED", "message": "Public link device limit reached"}),
    )
}

/// Which access-history table a link uses, with the link's key.
#[derive(Clone, Copy)]
enum History<'a> {
    /// `public_collection_access_history.share_id` (BIGINT).
    Collection(i64),
    /// `public_file_tokens_access_history.id` (TEXT).
    File(&'a str),
}

impl History<'_> {
    fn table_and_col(self) -> (&'static str, &'static str) {
        match self {
            History::Collection(_) => ("public_collection_access_history", "share_id"),
            History::File(_) => ("public_file_tokens_access_history", "id"),
        }
    }

    fn bind<'q, O>(
        self,
        q: sqlx::query::QueryScalar<'q, sqlx::Any, O, sqlx::any::AnyArguments<'q>>,
    ) -> sqlx::query::QueryScalar<'q, sqlx::Any, O, sqlx::any::AnyArguments<'q>> {
        match self {
            History::Collection(id) => q.bind(id),
            History::File(id) => q.bind(id.to_string()),
        }
    }
}

/// museum `isDeviceLimitReached`: admits (and records) a new device unless the limit is reached.
async fn device_limit_reached(
    db: &AnyPool,
    history: History<'_>,
    device_limit: i64,
    ip: &str,
    ua: &str,
) -> sqlx::Result<bool> {
    let (table, col) = history.table_and_col();
    let past_q =
        format!("SELECT COUNT(*) FROM {table} WHERE {col} = $1 AND ip = $2 AND user_agent = $3");
    let past: i64 = history
        .bind(sqlx::query_scalar(&past_q))
        .bind(ip)
        .bind(ua)
        .fetch_one(db)
        .await?;
    // A device that opened the link before is always let back in.
    if past > 0 {
        return Ok(false);
    }
    let count_q = format!("SELECT COUNT(*) FROM {table} WHERE {col} = $1");
    let count: i64 = history
        .bind(sqlx::query_scalar(&count_q))
        .fetch_one(db)
        .await?;
    let mut limit = device_limit;
    if limit == DEVICE_LIMIT_THRESHOLD {
        limit = DEVICE_LIMIT_THRESHOLD_MULTIPLIER * DEVICE_LIMIT_THRESHOLD;
    }
    if limit > 0 && count >= limit {
        return Ok(true);
    }
    let ins = format!(
        "INSERT INTO {table} ({col}, ip, user_agent, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING"
    );
    let q = sqlx::query(&ins);
    let q = match history {
        History::Collection(id) => q.bind(id),
        History::File(id) => q.bind(id.to_string()),
    };
    q.bind(ip).bind(ua).bind(now_micros()).execute(db).await?;
    Ok(false)
}

/// museum `checkDeviceLimit`: `Ok(Some(token))` when a (new or refreshed) device token
/// should be returned, `Ok(None)` when the presented one is still good, `Err(true)` when
/// the limit is reached.
#[allow(clippy::too_many_arguments)]
async fn check_device_limit(
    state: &AppState,
    parts: &Parts,
    history: History<'_>,
    scope: &str,
    link_id: &str,
    access_token: &str,
    device_limit: i64,
    valid_till: i64,
    ip: &str,
    ua: &str,
) -> Result<Option<String>, Response> {
    let secret = &state.keys.jwt;
    if let Some(t) = header_value(parts, "X-Auth-Link-Device-Token")
        && let Some(c) = jwt::validate_device_token(secret, &t, scope, link_id, access_token)
    {
        if c.exp - now_micros() < jwt::DEVICE_TOKEN_REFRESH_BEFORE {
            return Ok(Some(jwt::new_device_token(
                secret,
                scope,
                link_id,
                access_token,
                valid_till,
            )));
        }
        return Ok(None);
    }
    match device_limit_reached(&state.db.pool, history, device_limit, ip, ua).await {
        Err(e) => Err(internal(e)),
        Ok(true) => Err(device_limit_exceeded()),
        Ok(false) => Ok(Some(jwt::new_device_token(
            secret,
            scope,
            link_id,
            access_token,
            valid_till,
        ))),
    }
}

/// An authenticated public album link (museum `PublicAccessContext`).
#[derive(Debug, Clone)]
pub struct CollectionLink {
    /// Link id (`public_collection_tokens.id`).
    pub id: i64,
    /// Collection the link shares.
    pub collection_id: i64,
    /// Comments enabled on the link.
    pub enable_comment: bool,
    /// Viewer IP.
    pub ip: String,
    /// Viewer user agent.
    pub user_agent: String,
    /// Device token to hand back in `X-Link-Device-Token`.
    pub device_token: Option<String>,
}

const COLLECTION_PASSWORD_WHITELIST: [&str; 2] = [
    "/public-collection/info",
    "/public-collection/verify-password",
];

impl FromRequestParts<AppState> for CollectionLink {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = header_or_query(parts, "X-Auth-Access-Token", "accessToken")
            .ok_or_else(|| missing_token("album_link"))?;
        let ip = client_ip(parts);
        let ua = user_agent(parts);
        let path = route_path(parts);
        let link = links::collection_link_by_token(state, &token)
            .await
            .map_err(|e| {
                tracing::warn!(error = ?e, "album link lookup failed");
                invalid_token()
            })?
            .ok_or_else(invalid_token)?;
        if link.is_disabled {
            return Err(gone("LINK_DISABLED", "disabled token"));
        }
        if expired(link.valid_till) {
            return Err(gone("LINK_EXPIRED", "expired token"));
        }
        if let Some(hash) = link.pass_hash.as_deref().filter(|h| !h.is_empty())
            && !COLLECTION_PASSWORD_WHITELIST.contains(&path.as_str())
        {
            // museum serialises a plain Go error here, which renders as `{}`.
            let jwt_token = header_or_query(parts, "X-Auth-Access-Token-JWT", "accessTokenJWT")
                .ok_or_else(|| json_response(StatusCode::UNAUTHORIZED, json!({"error": {}})))?;
            if !jwt::password_token_valid(&state.keys.jwt, &jwt_token, hash) {
                return Err(json_response(
                    StatusCode::UNAUTHORIZED,
                    json!({"error": {}}),
                ));
            }
        }
        // Device admission is tied to the two entry/refresh endpoints, like museum.
        let mut device_token = None;
        if path == "/public-collection/info" || path == "/public-collection/diff" {
            device_token = check_device_limit(
                state,
                parts,
                History::Collection(link.id),
                jwt::SCOPE_COLLECTION,
                &link.id.to_string(),
                &token,
                link.device_limit,
                link.valid_till,
                &ip,
                &ua,
            )
            .await?;
        }
        Ok(CollectionLink {
            id: link.id,
            collection_id: link.collection_id,
            enable_comment: link.enable_comment,
            ip,
            user_agent: ua,
            device_token,
        })
    }
}

/// An authenticated single-file link (museum `FileLinkAccessContext`).
#[derive(Debug, Clone)]
pub struct FileLink {
    /// Link id.
    pub link_id: String,
    /// Shared file.
    pub file_id: i64,
    /// File owner.
    pub owner_id: i64,
    /// Viewer IP.
    pub ip: String,
    /// Viewer user agent.
    pub user_agent: String,
    /// Device token to hand back in `X-Link-Device-Token`.
    pub device_token: Option<String>,
}

const FILE_PASSWORD_WHITELIST: [&str; 2] = ["/file-link/pass-info", "/file-link/verify-password"];

impl FromRequestParts<AppState> for FileLink {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = header_or_query(parts, "X-Auth-Access-Token", "accessToken")
            .ok_or_else(|| missing_token("file_link"))?;
        let ip = client_ip(parts);
        let ua = user_agent(parts);
        let path = route_path(parts);
        let link = links::file_link_by_token(state, &token)
            .await
            .map_err(|e| {
                tracing::info!(error = ?e, "file link lookup failed");
                invalid_token()
            })?
            .ok_or_else(invalid_token)?;
        if link.is_disabled {
            return Err(gone("LINK_DISABLED", "disabled token"));
        }
        if expired(link.valid_till) {
            return Err(gone("LINK_EXPIRED", "expired token"));
        }
        if let Some(hash) = link.pw_hash.as_deref().filter(|h| !h.is_empty()) {
            match header_or_query(parts, "X-Auth-Access-Token-JWT", "accessTokenJWT") {
                None if FILE_PASSWORD_WHITELIST.contains(&path.as_str()) => {}
                None => {
                    return Err(json_response(
                        StatusCode::UNAUTHORIZED,
                        json!({"error": {"code": "PASS_PROTECTED_RESOURCE", "message": "This resource is password protected"}}),
                    ));
                }
                Some(t) => {
                    if !jwt::password_token_valid(&state.keys.jwt, &t, hash) {
                        return Err(json_response(
                            StatusCode::UNAUTHORIZED,
                            json!({"error": {}}),
                        ));
                    }
                }
            }
        }
        let mut device_token = None;
        if path == "/file-link/info" {
            device_token = check_device_limit(
                state,
                parts,
                History::File(&link.link_id),
                jwt::SCOPE_FILE,
                &link.link_id,
                &token,
                link.device_limit,
                link.valid_till,
                &ip,
                &ua,
            )
            .await?;
        }
        Ok(FileLink {
            link_id: link.link_id,
            file_id: link.file_id,
            owner_id: link.owner_id,
            ip,
            user_agent: ua,
            device_token,
        })
    }
}

/// An authenticated public memory link (museum `MemoryShareAccessContext`).
#[derive(Debug, Clone)]
pub struct MemoryShareLink {
    /// The share.
    pub share: MemoryShare,
    /// Viewer IP.
    pub ip: String,
    /// Viewer user agent.
    pub user_agent: String,
}

impl FromRequestParts<AppState> for MemoryShareLink {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = header_or_query(parts, "X-Auth-Access-Token", "accessToken")
            .ok_or_else(|| missing_token("memory_share"))?;
        let share = memory::by_access_token(state, &token)
            .await
            .map_err(|_| invalid_token())?
            .ok_or_else(invalid_token)?;
        if share.is_deleted {
            return Err(gone("LINK_DISABLED", "memory share is deleted"));
        }
        if share.created_at + MEMORY_SHARE_TTL <= now_micros() {
            return Err(gone("LINK_EXPIRED", "expired token"));
        }
        Ok(MemoryShareLink {
            share,
            ip: client_ip(parts),
            user_agent: user_agent(parts),
        })
    }
}
