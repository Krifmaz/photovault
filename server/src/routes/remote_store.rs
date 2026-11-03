//! Endpoints: /remote-store/* (per-user flags), /remote-store/feature-flags and the public
//! `GET /custom-domain` ownership check.
//!
//! Reference: museum `pkg/api/remotestore.go`, `pkg/controller/remotestore`,
//! `pkg/repo/remotestore`, `ente/remotestore.go`.

use crate::auth::AuthUser;
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::state::AppState;
use crate::users::extract::QueryMap;
use crate::users::{self, family};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

/// Museum `ente.CustomDomain`.
pub const CUSTOM_DOMAIN: &str = "customDomain";

/// Museum `ServerApiFlag` bits (`ente/remotestore.go`).
pub mod api_flag {
    /// Upload v2.
    pub const UPLOAD_V2: i64 = 1 << 0;
    /// Comments.
    pub const COMMENTS: i64 = 1 << 1;
    /// Backup options.
    pub const BACKUP_OPTIONS: i64 = 1 << 2;
    /// Video streaming.
    pub const VIDEO_STREAMING: i64 = 1 << 3;
    /// Cast sessions v2.
    pub const CAST_SESSIONS_V2: i64 = 1 << 5;
    /// Deferred multipart checksums.
    pub const DEFERRED_MULTIPART_CHECKSUMS: i64 = 1 << 6;
    /// Library sharing.
    pub const LIBRARY_SHARING: i64 = 1 << 7;
    /// Preview upload v2 (internal users only).
    pub const PREVIEW_UPLOAD_V2: i64 = 1 << 8;
}

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/remote-store/update", post(update))
        .route("/remote-store/{key}", delete(remove))
        .route("/remote-store", get(get_value))
        .route("/remote-store/feature-flags", get(feature_flags))
        .route("/custom-domain", get(check_domain))
}

/// Flag keys museum accepts (`validFlagKeys`).
fn is_valid_key(k: &str) -> bool {
    matches!(
        k,
        "recoveryKeyVerified"
            | "mapEnabled"
            | "faceSearchEnabled"
            | "passKeyEnabled"
            | "internalUser"
            | "betaUser"
            | CUSTOM_DOMAIN
    )
}

fn user_editable(k: &str) -> bool {
    matches!(
        k,
        "recoveryKeyVerified"
            | "mapEnabled"
            | "faceSearchEnabled"
            | "passKeyEnabled"
            | CUSTOM_DOMAIN
    )
}

/// Museum `FlagKey.IsAdminEditable`.
pub fn admin_editable(k: &str) -> bool {
    !matches!(
        k,
        "recoveryKeyVerified" | "mapEnabled" | "faceSearchEnabled"
    )
}

fn is_bool_key(k: &str) -> bool {
    is_valid_key(k) && k != CUSTOM_DOMAIN
}

/// Museum `CanonicalDomain`: IDNA ASCII form, lower-cased.
pub fn canonical_domain(domain: &str) -> Option<String> {
    match url::Host::parse(domain) {
        Ok(url::Host::Domain(d)) => Some(d.to_lowercase()),
        _ => None,
    }
}

/// Museum `isValidDomainWithoutScheme` (400 with message on failure).
pub fn validate_domain(input: &str) -> ApiResult<()> {
    if input.trim() != input {
        return Err(users::bad_request(
            "domain contains leading or trailing spaces",
        ));
    }
    if input.is_empty() {
        return Err(users::bad_request("domain is empty"));
    }
    if input.contains("://") {
        return Err(users::bad_request(
            "domain should not contain scheme (e.g., http:// or https://)",
        ));
    }
    let ascii = canonical_domain(input)
        .ok_or_else(|| users::bad_request(format!("invalid idn domain format: {input}")))?;
    if !domain_regex(&ascii) {
        return Err(users::bad_request(format!(
            "invalid domain format: {input}"
        )));
    }
    Ok(())
}

/// `^(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}$`
fn domain_regex(s: &str) -> bool {
    let labels: Vec<&str> = s.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let (tld, rest) = labels.split_last().unwrap_or((&"", &[]));
    if tld.len() < 2 || !tld.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    rest.iter().all(|l| {
        let b = l.as_bytes();
        !b.is_empty()
            && b.len() <= 63
            && b[0].is_ascii_alphanumeric()
            && b[b.len() - 1].is_ascii_alphanumeric()
            && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
    })
}

/// Museum `ParseFamilyCustomDomainPointer`: `Ok(Some((member, domain)))` for `_<id>:<domain>`,
/// `Ok(None)` for a plain domain.
fn parse_pointer(value: &str) -> ApiResult<Option<(i64, String)>> {
    let Some(trimmed) = value.strip_prefix('_') else {
        return Ok(None);
    };
    let Some((id, domain)) = trimmed.split_once(':') else {
        return Err(users::bad_request("invalid family custom domain format"));
    };
    if id.is_empty() || domain.is_empty() {
        return Err(users::bad_request("invalid family custom domain format"));
    }
    let member: i64 = id
        .parse()
        .ok()
        .filter(|m| *m > 0)
        .ok_or_else(|| users::bad_request("invalid family custom domain user id"))?;
    validate_domain(domain)?;
    Ok(Some((member, domain.to_string())))
}

/// Museum `ResolveCustomDomainValue`.
fn resolve_domain(value: &str) -> ApiResult<String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    Ok(match parse_pointer(value)? {
        Some((_, d)) => d,
        None => value.to_string(),
    })
}

fn validate_value(key: &str, value: &str) -> ApiResult<()> {
    if is_bool_key(key) && value != "true" && value != "false" {
        return Err(users::bad_request(format!("value {value} is not allowed")));
    }
    if key == CUSTOM_DOMAIN && !value.is_empty() && parse_pointer(value)?.is_none() {
        validate_domain(value)?;
    }
    Ok(())
}

async fn upsert(
    state: &AppState,
    user_id: i64,
    key: &str,
    value: &str,
    canonical: Option<&str>,
) -> ApiResult<()> {
    let now = now_micros();
    let r = sqlx::query(
        "INSERT INTO remote_store (user_id, key_name, key_value, canonical_value, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (user_id, key_name) \
         DO UPDATE SET key_value = EXCLUDED.key_value, canonical_value = EXCLUDED.canonical_value, updated_at = EXCLUDED.updated_at",
    )
    .bind(user_id)
    .bind(key)
    .bind(value)
    .bind(canonical)
    .bind(now)
    .bind(now)
    .execute(&state.db.pool)
    .await;
    match r {
        Ok(_) => Ok(()),
        Err(e)
            if e.as_database_error()
                .is_some_and(|d| d.is_unique_violation()) =>
        {
            Err(users::conflict(
                "custom domain already exists for another user",
            ))
        }
        Err(e) => Err(e.into()),
    }
}

async fn remove_key(state: &AppState, user_id: i64, key: &str) -> ApiResult<()> {
    sqlx::query("DELETE FROM remote_store WHERE user_id = $1 AND key_name = $2")
        .bind(user_id)
        .bind(key)
        .execute(&state.db.pool)
        .await?;
    Ok(())
}

async fn value_of(state: &AppState, user_id: i64, key: &str) -> ApiResult<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT key_value FROM remote_store WHERE user_id = $1 AND key_name = $2")
            .bind(user_id)
            .bind(key)
            .fetch_optional(&state.db.pool)
            .await?;
    Ok(row.map(|r| r.0))
}

/// Museum `DomainOwner`: the user whose custom domain (or a family pointer) matches.
pub async fn domain_owner(state: &AppState, domain: &str) -> ApiResult<Option<i64>> {
    let Some(canonical) = canonical_domain(domain) else {
        return Ok(None);
    };
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT user_id FROM remote_store WHERE key_name = $1 AND canonical_value = $2",
    )
    .bind(CUSTOM_DOMAIN)
    .bind(&canonical)
    .fetch_optional(&state.db.pool)
    .await?;
    if let Some((id,)) = row {
        return Ok(Some(id));
    }
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT user_id FROM remote_store WHERE key_name = $1 AND LOWER(key_value) = LOWER($2)",
    )
    .bind(CUSTOM_DOMAIN)
    .bind(domain)
    .fetch_optional(&state.db.pool)
    .await?;
    Ok(row.map(|r| r.0))
}

/// Admin of the family `user_id` belongs to.
async fn family_admin_of(state: &AppState, user_id: i64) -> ApiResult<Option<i64>> {
    let row: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT family_admin_id FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&state.db.pool)
            .await?;
    Ok(row.and_then(|r| r.0))
}

/// Rewrite (or with `None`, clear) the family pointers of an admin's members.
async fn sync_family_pointers(
    state: &AppState,
    user_id: i64,
    domain: Option<&str>,
) -> ApiResult<()> {
    if family_admin_of(state, user_id).await? != Some(user_id) {
        return Ok(());
    }
    for (member, _) in family::active_members(&state.db, user_id).await? {
        if member == user_id {
            continue;
        }
        let Some(current) = value_of(state, member, CUSTOM_DOMAIN).await? else {
            continue;
        };
        match parse_pointer(&current) {
            Ok(Some(_)) => {}
            Ok(None) => continue,
            Err(_) => {
                tracing::warn!(member, "family custom domain parse failed");
                continue;
            }
        }
        match domain {
            Some(d) => {
                upsert(
                    state,
                    member,
                    CUSTOM_DOMAIN,
                    &format!("_{member}:{d}"),
                    None,
                )
                .await?
            }
            None => remove_key(state, member, CUSTOM_DOMAIN).await?,
        }
    }
    Ok(())
}

fn is_reserved(state: &AppState, value: &str) -> bool {
    let Some(v) = canonical_domain(value) else {
        return false;
    };
    let cname = &state.config.accounts.custom_domain_cname;
    [cname.as_str(), "my.ente.com", "my.ente.io"]
        .iter()
        .filter(|d| !d.is_empty())
        .any(|d| canonical_domain(d).as_deref() == Some(v.as_str()))
}

/// Museum `insertOrUpdateCustomDomain`.
async fn set_custom_domain(state: &AppState, user_id: i64, value: &str) -> ApiResult<()> {
    if value.is_empty() {
        remove_key(state, user_id, CUSTOM_DOMAIN).await?;
        return sync_family_pointers(state, user_id, None).await;
    }
    if value.starts_with('_') {
        return Err(users::bad_request("invalid custom domain"));
    }
    if is_reserved(state, value) {
        return Err(users::bad_request("custom domain is reserved"));
    }
    let canonical = canonical_domain(value);
    match domain_owner(state, value).await? {
        None => {}
        Some(owner) if owner == user_id => {}
        Some(owner) => {
            if family_admin_of(state, user_id).await? == Some(owner) {
                let admin_domain = value_of(state, owner, CUSTOM_DOMAIN).await?;
                let Some(admin_domain) = admin_domain.filter(|d| !d.is_empty()) else {
                    return Err(users::bad_request("family admin has no custom domain"));
                };
                let pointer = format!("_{user_id}:{admin_domain}");
                return upsert(state, user_id, CUSTOM_DOMAIN, &pointer, None).await;
            }
            return Err(users::conflict(
                "custom domain already exists for another user",
            ));
        }
    }
    upsert(state, user_id, CUSTOM_DOMAIN, value, canonical.as_deref()).await?;
    sync_family_pointers(state, user_id, Some(value)).await
}

/// Museum `_validateRequest` + `InsertOrUpdate` / `AdminInsertOrUpdate`.
pub async fn insert_or_update(
    state: &AppState,
    user_id: i64,
    key: &str,
    value: Option<&str>,
    by_admin: bool,
) -> ApiResult<()> {
    if !is_valid_key(key) {
        return Err(users::bad_request(format!("key {key} is not allowed")));
    }
    let Some(value) = value else {
        return Err(users::bad_request("value is missing"));
    };
    validate_value(key, value)?;
    if !by_admin && !user_editable(key) {
        return Err(users::bad_request(format!(
            "key {key} is not user editable"
        )));
    }
    if by_admin && !admin_editable(key) {
        return Err(users::bad_request(format!(
            "key {key} is not admin editable"
        )));
    }
    if key == CUSTOM_DOMAIN {
        return set_custom_domain(state, user_id, value).await;
    }
    upsert(state, user_id, key, value, None).await
}

#[derive(Deserialize)]
struct UpdateRequest {
    key: String,
    value: Option<String>,
}

async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    body: axum::body::Bytes,
) -> ApiResult<StatusCode> {
    // gin's `binding:"required"` rejects a missing key or a null value with 400 `{}`.
    let req: UpdateRequest =
        serde_json::from_slice(&body).map_err(|_| users::bad_request_plain())?;
    if req.key.is_empty() || req.value.is_none() {
        return Err(users::bad_request_plain());
    }
    insert_or_update(&state, user.user_id, &req.key, req.value.as_deref(), false).await?;
    Ok(StatusCode::OK)
}

async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
) -> ApiResult<StatusCode> {
    if !is_valid_key(&key) {
        return Err(users::bad_request(format!("key {key} is not allowed")));
    }
    if key != CUSTOM_DOMAIN {
        return Err(users::bad_request(format!("key {key} is not removable")));
    }
    remove_key(&state, user.user_id, &key).await?;
    sync_family_pointers(&state, user.user_id, None).await?;
    Ok(StatusCode::OK)
}

async fn get_value(
    State(state): State<AppState>,
    user: AuthUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let key = q.get("key");
    if key.is_empty() {
        return Err(users::bad_request_plain());
    }
    let value = match value_of(&state, user.user_id, key).await? {
        Some(v) if key == CUSTOM_DOMAIN => resolve_domain(&v)?,
        Some(v) => v,
        None => match q.0.get("defaultValue") {
            Some(d) => d.clone(),
            None => return Err(users::not_found_plain()),
        },
    };
    Ok(Json(json!({ "value": value })))
}

/// Museum `GetFeatureFlags` for one user.
pub async fn feature_flags_for(state: &AppState, user_id: i64) -> ApiResult<Value> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT key_name, key_value FROM remote_store WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&state.db.pool)
            .await?;
    let s = &state.config.accounts;
    let mut flags = json!({
        "enableStripe": true,
        "disableCFWorker": false,
        "mapEnabled": false,
        "faceSearchEnabled": false,
        "passKeyEnabled": false,
        "recoveryKeyVerified": false,
        "internalUser": false,
        "betaUser": false,
        "enableMobMultiPart": true,
        "serverApiFlag": api_flag::UPLOAD_V2
            | api_flag::COMMENTS
            | api_flag::BACKUP_OPTIONS
            | api_flag::CAST_SESSIONS_V2
            | api_flag::DEFERRED_MULTIPART_CHECKSUMS
            | api_flag::VIDEO_STREAMING
            | api_flag::LIBRARY_SHARING,
        "castUrl": s.cast_url,
        "embedUrl": s.embed_albums_url,
    });
    if !s.custom_domain_cname.is_empty() {
        flags["customDomainCNAME"] = json!(s.custom_domain_cname);
    }
    let mut internal = false;
    for (k, v) in rows {
        match k.as_str() {
            CUSTOM_DOMAIN => {
                let resolved = resolve_domain(&v)?;
                if !resolved.is_empty() {
                    flags["customDomain"] = json!(resolved);
                }
            }
            "internalUser" => {
                internal = v == "true";
                flags["internalUser"] = json!(internal);
            }
            k if is_bool_key(k) => flags[k] = json!(v == "true"),
            _ => {}
        }
    }
    if internal {
        flags["serverApiFlag"] =
            json!(flags["serverApiFlag"].as_i64().unwrap_or(0) | api_flag::PREVIEW_UPLOAD_V2);
    }
    Ok(flags)
}

async fn feature_flags(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    Ok(Json(feature_flags_for(&state, user.user_id).await?))
}

async fn check_domain(State(state): State<AppState>, q: QueryMap) -> ApiResult<Json<Value>> {
    let domain = q.get("domain");
    if domain.is_empty() {
        return Err(users::bad_request("domain is missing"));
    }
    if validate_domain(domain).is_err() {
        // Museum answers with a bare 400 and no body here.
        return Err(crate::error::ApiError::status(StatusCode::BAD_REQUEST));
    }
    match domain_owner(&state, domain).await? {
        Some(_) => Ok(Json(json!({}))),
        None => Err(users::not_found_typed("")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains() {
        assert!(validate_domain("photos.example.com").is_ok());
        assert!(validate_domain("münchen.de").is_ok());
        assert!(validate_domain("https://a.com").is_err());
        assert!(validate_domain("localhost").is_err());
        assert!(validate_domain("-a.com").is_err());
        assert!(validate_domain(" a.com").is_err());
        assert_eq!(
            canonical_domain("Photos.Example.COM").as_deref(),
            Some("photos.example.com")
        );
        assert_eq!(
            parse_pointer("_12:a.example.com").ok().flatten(),
            Some((12, "a.example.com".to_string()))
        );
        assert!(parse_pointer("_x:a.example.com").is_err());
    }
}
