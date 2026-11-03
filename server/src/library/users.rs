//! What the library needs from the users workstream's tables.
//!
//! Reads `users` (0001_core) and `key_attributes` (museum column names, owned by the
//! users workstream). Email lookups go through a small per-requester limiter like
//! museum's `UserLookupController`, so sharing cannot be used to enumerate accounts.

use super::errors;
use super::sql::{db_err, get_opt_str};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// museum's `NormalizeEmail`: trimmed and lower-cased.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// `users.email_hash` for an email (delegates to [`crate::users::email_hash`], the single definition).
pub fn email_hash(state: &AppState, email: &str) -> String {
    crate::users::email_hash(&state.keys, email)
}

/// Look up a user id by email without limits (`GetUserIDWithEmailUnrestricted`).
pub async fn user_id_by_email(state: &AppState, email: &str) -> ApiResult<Option<i64>> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM users WHERE email_hash = $1")
        .bind(email_hash(state, email))
        .fetch_optional(&state.db.pool)
        .await
        .map_err(db_err)?;
    Ok(row.map(|r| r.0))
}

struct Window {
    started: Instant,
    targets: HashSet<String>,
    not_found: HashSet<String>,
}

impl Window {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            targets: HashSet::new(),
            not_found: HashSet::new(),
        }
    }
    fn reset_if_expired(&mut self, d: Duration) {
        if self.started.elapsed() >= d {
            *self = Self::new();
        }
    }
}

static LOOKUPS: Mutex<Option<HashMap<i64, (Window, Window)>>> = Mutex::new(None);

const MINUTE_TOTAL: usize = 50;
const DAY_TOTAL: usize = 50;
const MINUTE_NOT_FOUND: usize = 5;
const DAY_NOT_FOUND: usize = 10;

fn limit_start(requester: i64, target: &str) -> ApiResult<()> {
    let mut g = LOOKUPS.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    if map.len() > 100_000 {
        map.retain(|_, (_, d)| d.started.elapsed() < Duration::from_secs(86400));
    }
    let (m, d) = map
        .entry(requester)
        .or_insert_with(|| (Window::new(), Window::new()));
    m.reset_if_expired(Duration::from_secs(60));
    d.reset_if_expired(Duration::from_secs(86400));
    let m_new = !m.targets.contains(target);
    let d_new = !d.targets.contains(target);
    let too_many = ApiError::status(axum::http::StatusCode::TOO_MANY_REQUESTS);
    if (m_new && m.targets.len() >= MINUTE_TOTAL)
        || (d_new && d.targets.len() >= DAY_TOTAL)
        || (m_new && m.not_found.len() >= MINUTE_NOT_FOUND)
        || (d_new && d.not_found.len() >= DAY_NOT_FOUND)
    {
        return Err(too_many);
    }
    m.targets.insert(target.to_string());
    d.targets.insert(target.to_string());
    Ok(())
}

fn limit_not_found(requester: i64, target: &str) {
    let mut g = LOOKUPS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((m, d)) = g.get_or_insert_with(HashMap::new).get_mut(&requester) {
        m.not_found.insert(target.to_string());
        d.not_found.insert(target.to_string());
    }
}

/// `UserLookup.LookupUserID`: rate-limited email → user id (`None` when no such user).
pub async fn lookup_user_id(
    state: &AppState,
    requester: i64,
    email: &str,
) -> ApiResult<Option<i64>> {
    let target = email_hash(state, email);
    limit_start(requester, &target)?;
    let id = user_id_by_email(state, email).await?;
    if id.is_none() {
        limit_not_found(requester, &target);
    }
    Ok(id)
}

/// `UserLookup.VerifyUserID`: the email must belong to `expected`, else `RECIPIENT_IDENTITY_MISMATCH`.
pub async fn verify_user_id(
    state: &AppState,
    requester: i64,
    email: &str,
    expected: i64,
) -> ApiResult<()> {
    if expected <= 0 {
        return Err(errors::bad());
    }
    let target = format!("email-user-id\0{}\0{expected}", normalize_email(email));
    let target = crate::crypto::b64(&crate::crypto::generichash(
        target.as_bytes(),
        &state.keys.hashing,
    ));
    limit_start(requester, &target)?;
    match user_id_by_email(state, email).await? {
        Some(id) if id == expected => Ok(()),
        _ => {
            limit_not_found(requester, &target);
            Err(errors::recipient_identity_mismatch())
        }
    }
}

/// Decrypt a user's email (`""` when the user has none, e.g. deleted accounts).
pub async fn email_of(state: &AppState, user_id: i64) -> ApiResult<String> {
    let row =
        sqlx::query("SELECT encrypted_email, email_decryption_nonce FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&state.db.pool)
            .await
            .map_err(db_err)?;
    match row {
        None => Ok(String::new()),
        Some(r) => decrypt_email(
            state,
            get_opt_str(&r, "encrypted_email").map_err(db_err)?,
            get_opt_str(&r, "email_decryption_nonce").map_err(db_err)?,
        ),
    }
}

/// Decrypt stored email columns (base64 ciphertext and nonce, secretbox with the encryption key).
pub fn decrypt_email(
    state: &AppState,
    enc: Option<String>,
    nonce: Option<String>,
) -> ApiResult<String> {
    match (enc, nonce) {
        (Some(e), Some(n)) if !e.is_empty() && !n.is_empty() => {
            let ct = crate::crypto::unb64(&e).map_err(errors::internal)?;
            let nonce = crate::crypto::unb64(&n).map_err(errors::internal)?;
            let pt = crate::crypto::secretbox_open(&ct, &nonce, &state.keys.encryption)
                .map_err(errors::internal)?;
            String::from_utf8(pt).map_err(errors::internal)
        }
        _ => Ok(String::new()),
    }
}

/// museum's `GetKeyAttributes` existence check: no row → 404 `{}` (`sql.ErrNoRows`).
pub async fn require_key_attributes(state: &AppState, user_id: i64) -> ApiResult<()> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT user_id FROM key_attributes WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&state.db.pool)
            .await
            .map_err(db_err)?;
    row.map(|_| ()).ok_or_else(errors::not_found)
}

/// Best-effort cleanup in tables owned by other workstreams (cast tokens, comments,
/// reactions). A missing table or column is logged and ignored, so this crate keeps
/// working whichever of those workstreams are merged.
pub async fn best_effort(state: &AppState, what: &str, sql: &str, args: &[i64]) {
    let mut q = sqlx::query(sql);
    for a in args {
        q = q.bind(*a);
    }
    if let Err(e) = q.execute(&state.db.pool).await {
        tracing::debug!(error = %e, what, "best-effort cleanup skipped");
    }
}
