//! The few user-account reads the legacy and contacts features need, done with
//! direct SQL against museum-named tables (`users`, `key_attributes`).
//!
//! Assumptions shared with the users workstream:
//! * `users.encrypted_email` / `users.email_decryption_nonce` hold standard
//!   base64 of a libsodium secretbox under `keys.encryption`.
//! * `users.email_hash` = hex(generichash(lowercase(trim(email)), keys.hashing)).

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashMap;

/// museum's `ente.BasicUser`.
#[derive(Debug, Clone, Serialize)]
pub struct BasicUser {
    /// User id.
    pub id: i64,
    /// Plain email.
    pub email: String,
}

/// museum's `NormalizeEmail`.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Lookup hash for an email (`users.email_hash`).
pub fn email_hash(state: &AppState, email: &str) -> String {
    hex::encode(crate::crypto::generichash(
        normalize_email(email).as_bytes(),
        &state.keys.hashing,
    ))
}

/// Decrypt a stored email.
pub fn decrypt_email(state: &AppState, encrypted: &str, nonce: &str) -> Option<String> {
    let ct = crate::crypto::unb64(encrypted).ok()?;
    let n = crate::crypto::unb64(nonce).ok()?;
    let pt = crate::crypto::secretbox_open(&ct, &n, &state.keys.encryption).ok()?;
    String::from_utf8(pt).ok()
}

/// Id of the (non-deleted) user with this email.
pub async fn user_id_by_email(state: &AppState, email: &str) -> ApiResult<Option<i64>> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT user_id FROM users WHERE email_hash = $1 AND is_deleted = FALSE")
            .bind(email_hash(state, email))
            .fetch_optional(&state.db.pool)
            .await?;
    Ok(row.map(|r| r.0))
}

/// Emails of the active users among `ids` (museum's `GetActiveUsersForIds`).
pub async fn active_emails(state: &AppState, ids: &[i64]) -> ApiResult<HashMap<i64, String>> {
    let mut ids: Vec<i64> = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}")).collect();
    let sql = format!(
        "SELECT user_id, encrypted_email, email_decryption_nonce FROM users WHERE user_id IN ({}) AND is_deleted = FALSE",
        placeholders.join(",")
    );
    let mut q = sqlx::query_as::<_, (i64, Option<String>, Option<String>)>(&sql);
    for id in &ids {
        q = q.bind(*id);
    }
    for (id, enc, nonce) in q.fetch_all(&state.db.pool).await? {
        if let (Some(enc), Some(nonce)) = (enc, nonce) {
            match decrypt_email(state, &enc, &nonce) {
                Some(email) => {
                    out.insert(id, email);
                }
                None => return Err(ApiError::internal("failed to decrypt user email")),
            }
        }
    }
    Ok(out)
}

/// Email of one active user (museum's `UserRepo.Get(..).Email`).
pub async fn email_of(state: &AppState, id: i64) -> ApiResult<Option<String>> {
    Ok(active_emails(state, &[id]).await?.remove(&id))
}

/// museum's `ente.KeyAttributes` (all fields always present, recovery fields `""` when unset).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyAttributes {
    /// Salt for the key-encryption-key derivation.
    pub kek_salt: String,
    /// Master key encrypted with the KEK.
    pub encrypted_key: String,
    /// Nonce for `encrypted_key`.
    pub key_decryption_nonce: String,
    /// X25519 public key.
    pub public_key: String,
    /// Secret key encrypted with the master key.
    pub encrypted_secret_key: String,
    /// Nonce for `encrypted_secret_key`.
    pub secret_key_decryption_nonce: String,
    /// Argon2 memory limit.
    pub mem_limit: i64,
    /// Argon2 ops limit.
    pub ops_limit: i64,
    /// Master key encrypted with the recovery key.
    pub master_key_encrypted_with_recovery_key: String,
    /// Nonce for the above.
    pub master_key_decryption_nonce: String,
    /// Recovery key encrypted with the master key.
    pub recovery_key_encrypted_with_master_key: String,
    /// Nonce for the above.
    pub recovery_key_decryption_nonce: String,
}

type KeyAttrRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Key attributes of a user (museum's `UserRepo.GetKeyAttributes`), `None` when not set up.
pub async fn key_attributes(state: &AppState, user_id: i64) -> ApiResult<Option<KeyAttributes>> {
    let row: Option<KeyAttrRow> = sqlx::query_as(
        "SELECT kek_salt, encrypted_key, key_decryption_nonce, public_key, encrypted_secret_key, \
         secret_key_decryption_nonce, mem_limit, ops_limit, master_key_encrypted_with_recovery_key, \
         master_key_decryption_nonce, recovery_key_encrypted_with_master_key, recovery_key_decryption_nonce \
         FROM key_attributes WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db.pool)
    .await?;
    Ok(row.map(|r| KeyAttributes {
        kek_salt: r.0,
        encrypted_key: r.1,
        key_decryption_nonce: r.2,
        public_key: r.3,
        encrypted_secret_key: r.4,
        secret_key_decryption_nonce: r.5,
        mem_limit: r.6,
        ops_limit: r.7,
        master_key_encrypted_with_recovery_key: r.8.unwrap_or_default(),
        master_key_decryption_nonce: r.9.unwrap_or_default(),
        recovery_key_encrypted_with_master_key: r.10.unwrap_or_default(),
        recovery_key_decryption_nonce: r.11.unwrap_or_default(),
    }))
}
