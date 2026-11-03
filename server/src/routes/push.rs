//! Endpoints: /push/token. Tokens are stored per session; this server sends no pushes
//! itself (no FCM credentials), matching museum without `fcm` configuration.
//!
//! Reference: museum `pkg/api/push.go`, `pkg/controller/push.go`, `pkg/repo/push.go`.

use crate::auth::AuthUser;
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::state::AppState;
use crate::users;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

/// Length of an ML-KEM-768+X25519 HPKE public key (museum validates it with
/// `hpke.MLKEM768X25519().NewPublicKey`).
const NOTIFICATION_KEY_LEN: usize = 1184 + 32;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new().route("/push/token", post(add_token))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushTokenRequest {
    #[serde(default)]
    fcm_token: String,
    #[serde(default)]
    apns_token: String,
    #[serde(default = "default_platform")]
    platform: Option<String>,
    /// Go `[]byte`: standard base64.
    notification_key: Option<String>,
}

fn default_platform() -> Option<String> {
    Some("ios".into())
}

async fn add_token(
    State(state): State<AppState>,
    user: AuthUser,
    body: axum::body::Bytes,
) -> ApiResult<Json<Value>> {
    let req: PushTokenRequest =
        serde_json::from_slice(&body).map_err(|_| users::bad_request_plain())?;
    // binding: fcmToken required, platform required and one of ios/android.
    let platform = req.platform.unwrap_or_default();
    if req.fcm_token.is_empty() || !matches!(platform.as_str(), "ios" | "android") {
        return Err(users::bad_request_plain());
    }
    let key = match req.notification_key.as_deref() {
        None => None,
        Some(k) => {
            let raw = crate::crypto::unb64(k).map_err(|_| users::bad_request_plain())?;
            if raw.len() != NOTIFICATION_KEY_LEN {
                return Err(users::bad_request("invalid notification public key"));
            }
            Some(k.to_string())
        }
    };
    let now = now_micros();
    // Museum inserts only while the calling session is still live (else 401).
    let r = sqlx::query(
        "INSERT INTO push_tokens (fcm_token, user_id, apns_token, platform, session_token_hash, notification_public_key, created_at, updated_at, last_notified_at) \
         SELECT $1, user_id, $2, $3, token_hash, $4, $5, $6, $7 FROM tokens WHERE user_id = $8 AND token_hash = $9 AND is_deleted = $10 \
         ON CONFLICT (fcm_token) DO UPDATE SET user_id = EXCLUDED.user_id, apns_token = EXCLUDED.apns_token, \
         session_token_hash = EXCLUDED.session_token_hash, platform = EXCLUDED.platform, \
         notification_public_key = EXCLUDED.notification_public_key, updated_at = EXCLUDED.updated_at",
    )
    .bind(&req.fcm_token)
    .bind(&req.apns_token)
    .bind(&platform)
    .bind(key)
    .bind(now)
    .bind(now)
    .bind(0_i64)
    .bind(user.user_id)
    .bind(&user.token_hash)
    .bind(false)
    .execute(&state.db.pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(users::unauthorized_plain());
    }
    Ok(Json(json!({})))
}
