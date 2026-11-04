//! Photovault's own endpoints (not part of museum's API); see `contracts/`.
//!
//! * `GET /photovault/info` (public, CORS-open): what the apps' "Connect to your
//!   server" screen shows, and the addresses they switch between.
//! * `GET /photovault/usage` (auth): the storage numbers, explicitly.

use crate::auth::AuthUser;
use crate::config::StorageConfig;
use crate::error::ApiResult;
use crate::state::AppState;
use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

/// Contract version of these endpoints.
pub const API_VERSION: i64 = 1;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/photovault/info", get(info))
        .route("/photovault/usage", get(usage))
}

/// Short git hash baked in at build time (`PHOTOVAULT_COMMIT`), if any.
pub fn commit() -> Option<&'static str> {
    option_env!("PHOTOVAULT_COMMIT").filter(|c| !c.is_empty())
}

/// The info document.
pub async fn info_value(state: &AppState) -> Value {
    let c = &state.config;
    let public = c.public_url.trim_end_matches('/').to_string();
    let or_default = |v: &str, path: &str| {
        let v = v.trim_end_matches('/');
        if v.is_empty() {
            format!("{public}{path}")
        } else {
            v.to_string()
        }
    };
    let email = match (&c.smtp, c.skip_email_verification) {
        (Some(_), _) => "smtp",
        (None, true) => "skip",
        (None, false) => "log",
    };
    let storage = match c.storage {
        StorageConfig::Local { .. } => "local",
        StorageConfig::S3(_) => "s3",
    };
    let mut features = vec!["admin-cli"];
    if c.web_dir
        .as_ref()
        .is_some_and(|d| d.join("index.html").is_file())
    {
        features.extend(["web", "public-albums", "cast"]);
    }
    if c.mdns {
        features.push("mdns");
    }
    if c.asset_mirror {
        features.extend(["models", "music"]);
    }
    json!({
        "product": "photovault",
        "name": c.server_name,
        "version": env!("CARGO_PKG_VERSION"),
        "commit": commit(),
        "apiVersion": API_VERSION,
        "publicUrl": public,
        "homeUrl": c.home_url,
        // Same base the share links use (`public_albums_url`; `<public>/albums` by default).
        "albumsUrl": or_default(&c.accounts.public_albums_url, "/albums"),
        "castUrl": or_default(&c.accounts.cast_url, "/cast"),
        // The server's own Cast receiver; null = no Chromecast auto-pairing.
        "castAppId": c.cast_app_id,
        // Present when the asset mirror is on (`crate::asset_mirror`).
        "modelsUrl": c.asset_mirror.then(|| format!("{public}/photovault/models")),
        "musicManifestUrl": c.asset_mirror.then(|| format!("{public}/photovault/music/tracks.json")),
        "signupsAllowed": crate::settings::signups_allowed(state).await,
        "emailVerification": email,
        "storage": storage,
        "features": features,
    })
}

async fn info(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(info_value(&state).await),
    )
}

/// `GET /photovault/usage` (contracts/storage-usage.md).
pub async fn usage_value(state: &AppState, user_id: i64) -> ApiResult<Value> {
    let fam: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT family_admin_id FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&state.db.pool)
            .await?;
    let admin = fam.and_then(|f| f.0).unwrap_or(user_id);
    let mut members: Vec<i64> = crate::users::family::active_members(&state.db, admin)
        .await?
        .into_iter()
        .map(|m| m.0)
        .collect();
    if members.is_empty() {
        members.push(admin);
    }
    let used = crate::users::store::combined_usage(&state.db, &members).await?;
    let quota = crate::users::subscription(&state.db, admin).await?.storage;
    let free = crate::capacity::free_bytes(state).await?;
    let disk_limit = free.map(|f| used.saturating_add(f));
    let quota_limit = (quota > 0).then_some(quota);
    let default_quota = quota > 0 && quota == state.config.default_storage_bytes;
    let (limit, source) = match (quota_limit, disk_limit) {
        (Some(q), Some(d)) if q <= d => (
            Some(q),
            if default_quota {
                "default-quota"
            } else {
                "quota"
            },
        ),
        (Some(q), None) => (
            Some(q),
            if default_quota {
                "default-quota"
            } else {
                "quota"
            },
        ),
        (_, Some(d)) => (
            Some(d),
            match &state.config.storage {
                StorageConfig::S3(_) => "storage-limit",
                StorageConfig::Local { .. } if state.config.storage_limit.is_some() => {
                    "storage-limit"
                }
                StorageConfig::Local { .. } => "disk",
            },
        ),
        (None, None) => (None, "unlimited"),
    };
    Ok(json!({
        "usedBytes": used,
        "limitBytes": limit,
        "freeBytes": limit.map(|l| (l - used).max(0)),
        "unlimited": limit.is_none(),
        "limitSource": source,
        "quotaBytes": quota_limit,
    }))
}

async fn usage(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    Ok(Json(usage_value(&state, user.user_id).await?))
}
