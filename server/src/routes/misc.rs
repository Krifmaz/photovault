//! Endpoints: ping, /fire/db-m-ping, /events, /events/user, /paste/*.
//! (`/custom-domain` lives with the remote store in [`super::remote_store`].)
//!
//! Reference: museum `pkg/api/healthcheck.go`, `pkg/api/event.go`, `pkg/api/paste.go`,
//! `pkg/controller/public/paste.go`, `ente/paste.go`.

use crate::auth::AuthUser;
use crate::db::now_micros;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::users::{self, api};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ping", get(ping))
        .route("/fire/db-m-ping", get(db_ping))
        .route("/events", post(create_event))
        .route("/events/user", post(create_user_event))
        .route("/paste/create", post(paste_create))
        .route("/paste/guard", post(paste_guard))
        .route("/paste/consume", post(paste_consume))
}

async fn ping(
    State(state): State<AppState>,
    q: users::extract::QueryMap,
) -> ApiResult<Json<Value>> {
    sqlx::query("SELECT 1").execute(&state.db.pool).await?;
    let mut out = json!({
        "message": "pong",
        "id": std::env::var("GIT_COMMIT").unwrap_or_default(),
    });
    if !q.get("host").is_empty() {
        let host = std::env::var("HOSTNAME").unwrap_or_default();
        out["host"] = json!(host);
    }
    Ok(Json(out))
}

async fn db_ping(State(state): State<AppState>) -> ApiResult<StatusCode> {
    sqlx::query("SELECT 1")
        .execute(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(format!("DB ping failed: {e}")))?;
    Ok(StatusCode::OK)
}

// ---- events (museum `EventHandler`) ----

#[derive(Deserialize)]
struct EventRequest {
    #[serde(default)]
    id: String,
    #[serde(default)]
    event: String,
    #[serde(default)]
    app: String,
    #[serde(default)]
    platform: String,
    data: Option<Map<String, Value>>,
}

/// gin `binding:"required"` + museum `bindEventRequest`.
fn bind_event(body: &[u8]) -> ApiResult<(EventRequest, Map<String, Value>)> {
    let req: EventRequest = serde_json::from_slice(body).map_err(|_| users::bad_request_plain())?;
    if req.id.is_empty() || req.event.is_empty() || req.app.is_empty() || req.platform.is_empty() {
        return Err(users::bad_request_plain());
    }
    if uuid::Uuid::parse_str(&req.id).is_err() {
        return Err(users::bad_request("invalid id"));
    }
    // gin's `required` also rejects a JSON null map.
    let Some(data) = req.data.clone() else {
        return Err(users::bad_request_plain());
    };
    Ok((req, data))
}

fn enrich(headers: &HeaderMap, mut data: Map<String, Value>) -> Map<String, Value> {
    data.remove("app");
    data.remove("platform");
    if let Some(v) = headers
        .get("X-Client-Version")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
    {
        data.insert("app_version".into(), json!(v));
    }
    data
}

async fn insert_event(
    state: &AppState,
    req: &EventRequest,
    data: &Map<String, Value>,
    user_id: Option<i64>,
) -> ApiResult<()> {
    let now = now_micros();
    sqlx::query(
        "INSERT INTO events (id, event, app, platform, data, user_id, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (id, event) DO NOTHING",
    )
    .bind(&req.id)
    .bind(&req.event)
    .bind(&req.app)
    .bind(&req.platform)
    .bind(Value::Object(data.clone()).to_string())
    .bind(user_id)
    .bind(now)
    .bind(now)
    .execute(&state.db.pool)
    .await?;
    Ok(())
}

async fn create_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<StatusCode> {
    let (req, data) = bind_event(&body)?;
    if req.event != "install" {
        return Err(users::bad_request("invalid event"));
    }
    insert_event(&state, &req, &enrich(&headers, data), None).await?;
    Ok(StatusCode::OK)
}

async fn create_user_event(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<StatusCode> {
    let (req, extra) = bind_event(&body)?;
    if req.event != "sign_up" && req.event != "log_in" {
        return Err(users::bad_request("invalid event"));
    }
    let install: Option<(String,)> =
        sqlx::query_as("SELECT data FROM events WHERE id = $1 AND event = $2")
            .bind(&req.id)
            .bind("install")
            .fetch_optional(&state.db.pool)
            .await?;
    let mut data: Map<String, Value> = install
        .and_then(|(d,)| serde_json::from_str(&d).ok())
        .unwrap_or_default();
    data.extend(extra);
    insert_event(&state, &req, &enrich(&headers, data), Some(user.user_id)).await?;
    Ok(StatusCode::OK)
}

// ---- paste (museum `PasteController`) ----

const PASTE_TOKEN_LEN: usize = 10;
const PASTE_MAX_CIPHERTEXT: usize = 32256;
const PASTE_TTL_US: i64 = 24 * 3600 * 1_000_000;
const GUARD_TTL_US: i64 = 2 * 60 * 1_000_000;
const GUARD_COOKIE: &str = "paste_guard";

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct CreatePaste {
    encrypted_data: String,
    decryption_header: String,
    encrypted_paste_key: String,
    encrypted_paste_key_nonce: String,
    kdf_nonce: String,
    kdf_mem_limit: i64,
    kdf_ops_limit: i64,
}

impl CreatePaste {
    /// Museum `CreatePasteRequest.Validate`.
    fn validate(&self) -> ApiResult<()> {
        let blank = |s: &str| s.trim().is_empty();
        if blank(&self.encrypted_data) || blank(&self.decryption_header) {
            return Err(users::bad_request("invalid encrypted payload"));
        }
        if self.encrypted_data.len() > PASTE_MAX_CIPHERTEXT || self.decryption_header.len() > 128 {
            return Err(users::bad_request("encrypted payload too large"));
        }
        if blank(&self.encrypted_paste_key)
            || blank(&self.encrypted_paste_key_nonce)
            || blank(&self.kdf_nonce)
        {
            return Err(users::bad_request("invalid key material"));
        }
        if self.encrypted_paste_key.len() > 256 || self.encrypted_paste_key_nonce.len() > 128 {
            return Err(users::bad_request("key material too large"));
        }
        let kdf_err = || users::bad_request("invalid key derivation parameters");
        if self.kdf_nonce.len() > 64 {
            return Err(kdf_err());
        }
        use base64::Engine;
        let salt = base64::engine::general_purpose::STANDARD
            .decode(&self.kdf_nonce)
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&self.kdf_nonce));
        if salt.map(|s| s.len()).ok() != Some(16) {
            return Err(kdf_err());
        }
        let ok = matches!(
            (self.kdf_mem_limit, self.kdf_ops_limit),
            (67_108_864, 2) | (268_435_456, 3)
        );
        if !ok {
            return Err(kdf_err());
        }
        Ok(())
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct PasteToken {
    access_token: String,
}

/// Museum `PasteTokenRequest.Validate`; returns the trimmed token.
fn validate_token(body: &[u8]) -> ApiResult<String> {
    let req: PasteToken = serde_json::from_slice(body).map_err(|_| users::bad_request_plain())?;
    if req.access_token.is_empty() {
        return Err(users::bad_request_plain());
    }
    let t = req.access_token.trim();
    if t.is_empty() {
        return Err(users::bad_request("access token required"));
    }
    let n = t.chars().count();
    if !(6..=32).contains(&n) || !t.chars().all(char::is_alphanumeric) {
        return Err(users::bad_request("invalid access token"));
    }
    Ok(t.to_string())
}

fn unavailable() -> ApiError {
    api(StatusCode::GONE, "NOT_FOUND", "Paste is unavailable")
}

fn header_str<'a>(h: &'a HeaderMap, k: &str) -> &'a str {
    h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("").trim()
}

fn is_preview(h: &HeaderMap) -> bool {
    let ua = header_str(h, "User-Agent").to_lowercase();
    let purpose = header_str(h, "Purpose").to_lowercase();
    let sec = header_str(h, "Sec-Purpose").to_lowercase();
    if purpose.contains("prefetch")
        || purpose.contains("preview")
        || sec.contains("prefetch")
        || sec.contains("preview")
        || sec.contains("prerender")
    {
        return true;
    }
    [
        "bot",
        "crawler",
        "spider",
        "preview",
        "slackbot",
        "discordbot",
        "twitterbot",
        "facebookexternalhit",
        "whatsappbot",
        "telegrambot",
        "linkedinbot",
        "skypeuripreview",
        "googlebot",
    ]
    .iter()
    .any(|t| ua.contains(t))
}

fn ua_hash(h: &HeaderMap) -> String {
    hex::encode(Sha256::digest(header_str(h, "User-Agent").as_bytes()))
}

fn secure_cookie(h: &HeaderMap) -> bool {
    header_str(h, "Origin").starts_with("https://")
        || header_str(h, "X-Forwarded-Proto").eq_ignore_ascii_case("https")
}

fn cookie(h: &HeaderMap, name: &str, value: &str, max_age: i64) -> String {
    let mut c = format!("{name}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict");
    if secure_cookie(h) {
        c.push_str("; Secure");
    }
    c
}

fn read_cookie(h: &HeaderMap, name: &str) -> Option<String> {
    h.get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

/// Museum `PasteGuardClaim`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GuardClaim {
    access_token: String,
    user_agent_hash: String,
    expiry_time: i64,
}

fn paste_access_token() -> String {
    const SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut out = String::with_capacity(PASTE_TOKEN_LEN);
    while out.len() < PASTE_TOKEN_LEN {
        let b = crate::crypto::random_bytes(1)[0] as usize;
        if b < 256 - (256 % SET.len()) {
            out.push(SET[b % SET.len()] as char);
        }
    }
    out
}

async fn paste_create(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> ApiResult<Json<Value>> {
    let req: CreatePaste = serde_json::from_slice(&body).map_err(|_| users::bad_request_plain())?;
    // `binding:"required"` on every field.
    if req.encrypted_data.is_empty()
        || req.decryption_header.is_empty()
        || req.encrypted_paste_key.is_empty()
        || req.encrypted_paste_key_nonce.is_empty()
        || req.kdf_nonce.is_empty()
        || req.kdf_mem_limit == 0
        || req.kdf_ops_limit == 0
    {
        return Err(users::bad_request_plain());
    }
    req.validate()?;
    let now = now_micros();
    let expires_at = now + PASTE_TTL_US;
    // Expired pastes are never served again; drop them here instead of in a cron.
    sqlx::query("DELETE FROM public_paste_tokens WHERE expires_at <= $1")
        .bind(now)
        .execute(&state.db.pool)
        .await?;
    for _ in 0..5 {
        let token = paste_access_token();
        let id = format!("ppt_{}", users::random_url_safe(16));
        let r = sqlx::query(
            "INSERT INTO public_paste_tokens (id, access_token, encrypted_data, decryption_header, encrypted_paste_key, \
             encrypted_paste_key_nonce, kdf_nonce, kdf_mem_limit, kdf_ops_limit, expires_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&id)
        .bind(&token)
        .bind(&req.encrypted_data)
        .bind(&req.decryption_header)
        .bind(&req.encrypted_paste_key)
        .bind(&req.encrypted_paste_key_nonce)
        .bind(&req.kdf_nonce)
        .bind(req.kdf_mem_limit)
        .bind(req.kdf_ops_limit)
        .bind(expires_at)
        .bind(now)
        .execute(&state.db.pool)
        .await;
        match r {
            Ok(_) => {
                return Ok(Json(
                    json!({ "accessToken": token, "expiresAt": expires_at }),
                ));
            }
            Err(e)
                if e.as_database_error()
                    .is_some_and(|d| d.is_unique_violation()) =>
            {
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(api(
        StatusCode::CONFLICT,
        "ACCESS_TOKEN_IN_USE",
        "access token already in use",
    ))
}

async fn paste_guard(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    let token = validate_token(&body)?;
    if is_preview(&headers) {
        return Err(unavailable());
    }
    let exists: Option<(i64,)> = sqlx::query_as(
        "SELECT kdf_ops_limit FROM public_paste_tokens WHERE access_token = $1 AND expires_at > $2",
    )
    .bind(&token)
    .bind(now_micros())
    .fetch_optional(&state.db.pool)
    .await?;
    if exists.is_none() {
        return Err(unavailable());
    }
    let jwt = crate::jwt::sign(
        &state.keys.jwt,
        &GuardClaim {
            access_token: token,
            user_agent_hash: ua_hash(&headers),
            expiry_time: now_micros() + GUARD_TTL_US,
        },
    );
    let c = cookie(&headers, GUARD_COOKIE, &jwt, GUARD_TTL_US / 1_000_000);
    Ok(([(header::SET_COOKIE, c)], Json(json!({}))).into_response())
}

async fn paste_consume(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    let token = validate_token(&body)?;
    if header_str(&headers, "X-Paste-Consume") != "1" {
        return Err(users::bad_request("missing consume confirmation"));
    }
    if is_preview(&headers) {
        return Err(unavailable());
    }
    let origin = &state.config.accounts.paste_url;
    let req_origin = header_str(&headers, "Origin");
    if !origin.is_empty() && !req_origin.is_empty() && req_origin != origin.trim_end_matches('/') {
        return Err(unavailable());
    }
    let guard = read_cookie(&headers, GUARD_COOKIE).ok_or_else(unavailable)?;
    let claim: GuardClaim =
        crate::jwt::decode(&state.keys.jwt, &guard).map_err(|_| unavailable())?;
    if claim.expiry_time < now_micros()
        || claim.access_token != token
        || claim.user_agent_hash != ua_hash(&headers)
    {
        return Err(unavailable());
    }
    type Row = (String, String, String, String, String, i64, i64);
    let row: Option<Row> = sqlx::query_as(
        "DELETE FROM public_paste_tokens WHERE access_token = $1 AND expires_at > $2 \
         RETURNING encrypted_data, decryption_header, encrypted_paste_key, encrypted_paste_key_nonce, kdf_nonce, kdf_mem_limit, kdf_ops_limit",
    )
    .bind(&token)
    .bind(now_micros())
    .fetch_optional(&state.db.pool)
    .await?;
    let (ed, dh, epk, epkn, kn, mem, ops) = row.ok_or_else(unavailable)?;
    let c = cookie(&headers, GUARD_COOKIE, "", -1);
    Ok((
        [(header::SET_COOKIE, c)],
        Json(json!({
            "encryptedData": ed,
            "decryptionHeader": dh,
            "encryptedPasteKey": epk,
            "encryptedPasteKeyNonce": epkn,
            "kdfNonce": kn,
            "kdfMemLimit": mem,
            "kdfOpsLimit": ops,
        })),
    )
        .into_response())
}
