//! Request extractors matching museum's binding and JWT middleware behaviour.

use crate::auth::{App, AuthRejection, AuthUser, hash_token, token_from_parts};
use crate::error::ApiError;
use crate::jwt;
use crate::state::AppState;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

/// JSON body parsed like gin's `ShouldBindJSON`: any content type, and failures are 400 `{}`.
#[derive(Debug, Clone)]
pub struct Body<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for Body<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|_| super::bad_request_plain())?;
        serde_json::from_slice(&bytes)
            .map(Body)
            .map_err(|_| super::bad_request_plain())
    }
}

/// Query string as a map (missing/duplicate keys handled by the handler).
#[derive(Debug, Clone, Default)]
pub struct QueryMap(pub std::collections::HashMap<String, String>);

impl<S: Send + Sync> FromRequestParts<S> for QueryMap {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(QueryMap(
            parts
                .uri
                .query()
                .map(|q| {
                    url::form_urlencoded::parse(q.as_bytes())
                        .into_owned()
                        .collect()
                })
                .unwrap_or_default(),
        ))
    }
}

impl QueryMap {
    /// Value or empty string.
    pub fn get(&self, k: &str) -> &str {
        self.0.get(k).map(String::as_str).unwrap_or("")
    }
}

/// Client metadata museum records with sessions.
#[derive(Debug, Clone, Default)]
pub struct ClientInfo {
    /// Calling app.
    pub app: Option<App>,
    /// Client IP (`X-Forwarded-For` first hop, else empty).
    pub ip: String,
    /// `User-Agent`.
    pub user_agent: String,
    /// `X-Client-Package`.
    pub package: String,
}

impl ClientInfo {
    /// Calling app (Photos when unknown).
    pub fn app(&self) -> App {
        self.app.unwrap_or(App::Photos)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientInfo {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let h = |k: &str| {
            parts
                .headers
                .get(k)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string()
        };
        let ip = h("X-Forwarded-For")
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        Ok(ClientInfo {
            app: Some(App::from_parts(parts)),
            ip,
            user_agent: h("User-Agent"),
            package: h("X-Client-Package"),
        })
    }
}

/// Validate a museum web JWT and the session it was minted from (museum
/// `TokenAuthMiddleware(scope)`).
async fn jwt_user(
    state: &AppState,
    token: &str,
    scope: &str,
    app: App,
) -> Result<AuthUser, AuthRejection> {
    let invalid = AuthRejection(StatusCode::UNAUTHORIZED, "invalid token");
    let claim =
        jwt::verify_web(&state.keys.jwt, token, scope).map_err(|_| invalid_clone(&invalid))?;
    let session_hash =
        crate::crypto::unb64(&claim.session_token_hash).map_err(|_| invalid_clone(&invalid))?;
    if session_hash.len() != 32
        || !matches!(claim.session_app.as_str(), "photos" | "auth" | "locker")
    {
        return Err(invalid);
    }
    let token_hash = hex::encode(session_hash);
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT user_id FROM tokens WHERE token_hash = $1 AND app = $2 AND is_deleted = FALSE",
    )
    .bind(&token_hash)
    .bind(&claim.session_app)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(|_| {
        AuthRejection(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to validate token",
        )
    })?;
    match row {
        Some((uid,)) if uid == claim.user_id => Ok(AuthUser {
            user_id: uid,
            app,
            token_hash,
        }),
        _ => Err(invalid),
    }
}

fn invalid_clone(r: &AuthRejection) -> AuthRejection {
    AuthRejection(r.0, r.1)
}

/// `accountsJwtAuthAPI`: token must be an ACCOUNTS-scoped JWT.
#[derive(Debug, Clone)]
pub struct AccountsJwtUser(pub AuthUser);

impl FromRequestParts<AppState> for AccountsJwtUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = token_from_parts(parts)
            .ok_or(AuthRejection(StatusCode::UNAUTHORIZED, "missing token"))?;
        let app = App::from_parts(parts);
        Ok(AccountsJwtUser(
            jwt_user(state, &token, jwt::scope::ACCOUNTS, app).await?,
        ))
    }
}

/// `familyAuthAPI`: a FAMILIES JWT (token contains a dot) or a normal session token.
#[derive(Debug, Clone)]
pub struct FamilyAuthUser(pub AuthUser);

impl FromRequestParts<AppState> for FamilyAuthUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = token_from_parts(parts)
            .ok_or(AuthRejection(StatusCode::UNAUTHORIZED, "missing token"))?;
        if token.contains('.') {
            let app = App::from_parts(parts);
            return Ok(FamilyAuthUser(
                jwt_user(state, &token, jwt::scope::FAMILIES, app).await?,
            ));
        }
        Ok(FamilyAuthUser(
            AuthUser::from_request_parts(parts, state).await?,
        ))
    }
}

/// `paymentJwtAuthAPI`: a PAYMENT JWT.
#[derive(Debug, Clone)]
pub struct PaymentJwtUser(pub AuthUser);

impl FromRequestParts<AppState> for PaymentJwtUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = token_from_parts(parts)
            .ok_or(AuthRejection(StatusCode::UNAUTHORIZED, "missing token"))?;
        let app = App::from_parts(parts);
        Ok(PaymentJwtUser(
            jwt_user(state, &token, jwt::scope::PAYMENT, app).await?,
        ))
    }
}

/// Museum's `AdminAuthMiddleware`: user ids in `config.admins` or marked with
/// `photovault admin mark-admin`; when neither exists, the lowest user id (the first
/// account on the instance) is the admin.
#[derive(Debug, Clone)]
pub struct Admin(pub AuthUser);

impl FromRequestParts<AppState> for Admin {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let u = AuthUser::from_request_parts(parts, state).await?;
        if is_admin(state, u.user_id).await {
            return Ok(Admin(u));
        }
        Err(AuthRejection(
            StatusCode::UNAUTHORIZED,
            "insufficient permissions",
        ))
    }
}

/// Whether `user_id` may use `/admin/*`.
pub async fn is_admin(state: &AppState, user_id: i64) -> bool {
    if state.config.admins.contains(&user_id) {
        return true;
    }
    // Admins marked with `photovault admin mark-admin`.
    let flagged: Vec<(i64,)> =
        sqlx::query_as("SELECT user_id FROM users WHERE is_admin = $1 AND is_deleted = $2")
            .bind(true)
            .bind(false)
            .fetch_all(&state.db.pool)
            .await
            .unwrap_or_default();
    if flagged.iter().any(|f| f.0 == user_id) {
        return true;
    }
    // No implicit admin (e.g. "first account"): on a public server that would hand
    // admin rights to whoever signs up first. Use `config.admins` or `admin mark-admin`.
    false
}

/// SHA-256 hex of the request's session token (matches `tokens.token_hash`).
pub fn current_token_hash(parts: &Parts) -> Option<String> {
    token_from_parts(parts).map(|t| hash_token(&t))
}
