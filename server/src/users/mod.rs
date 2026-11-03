//! Users & accounts: shared helpers for the users, passkeys, family, billing, storage-bonus,
//! remote-store, push, admin and misc route groups, and for other workstreams.
//!
//! Public helpers other workstreams may rely on:
//! * [`email_hash`], [`encrypt_email`], [`decrypt_email`], [`normalize_email`]
//! * [`get_user`], [`user_id_for_email`], [`email_for_user`]
//! * [`usage`] (bytes used, museum-equivalent SQL over `files`/`object_keys`)
//! * [`storage_limit`] (effective quota incl. family + bonuses) for upload checks
//! * [`public_key`] for sharing

pub mod errors;
pub mod extract;
pub mod family;
pub mod lifecycle;
pub mod login;
pub mod passkeys;
pub mod settings;
pub mod srp;
pub mod store;
pub mod totp;
pub mod webauthn;

pub use errors::*;
pub use store::*;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Museum's `NormalizeEmail`: trim + lowercase.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// `users.email_hash`: hex of keyed BLAKE2b-256 (`keys.hashing`) over the normalised
/// (trimmed, lower-cased) email. The single definition every workstream uses.
pub fn email_hash(keys: &crate::state::Keys, email: &str) -> String {
    hex::encode(crate::crypto::generichash(
        normalize_email(email).as_bytes(),
        &keys.hashing,
    ))
}

/// Museum's `crypto.GetHash` for any string (also used for TOTP secret hashes).
pub fn keyed_hash(state: &AppState, data: &str) -> String {
    crate::crypto::b64(&crate::crypto::generichash(
        data.as_bytes(),
        &state.keys.hashing,
    ))
}

/// secretbox(email) with `keys.encryption` → (ciphertext b64, nonce b64).
pub fn encrypt_email(state: &AppState, email: &str) -> ApiResult<(String, String)> {
    encrypt_secret(state, email)
}

/// Museum's `crypto.Encrypt` with the server's encryption key; base64 outputs.
pub fn encrypt_secret(state: &AppState, data: &str) -> ApiResult<(String, String)> {
    let (ct, nonce) = crate::crypto::secretbox(data.as_bytes(), &state.keys.encryption)
        .map_err(ApiError::internal)?;
    Ok((crate::crypto::b64(&ct), crate::crypto::b64(&nonce)))
}

/// Inverse of [`encrypt_email`].
pub fn decrypt_email(state: &AppState, ct_b64: &str, nonce_b64: &str) -> ApiResult<String> {
    decrypt_secret(state, ct_b64, nonce_b64)
}

/// Inverse of [`encrypt_secret`].
pub fn decrypt_secret(state: &AppState, ct_b64: &str, nonce_b64: &str) -> ApiResult<String> {
    let ct = crate::crypto::unb64(ct_b64).map_err(ApiError::internal)?;
    let nonce = crate::crypto::unb64(nonce_b64).map_err(ApiError::internal)?;
    let pt = crate::crypto::secretbox_open(&ct, &nonce, &state.keys.encryption)
        .map_err(ApiError::internal)?;
    String::from_utf8(pt).map_err(ApiError::internal)
}

/// Marker stored in `users.email_hash` once an account is deleted (museum's
/// `DELETED_EMAIL_HASH_FORMAT`, with a neutral domain).
pub fn deleted_email_hash(user_id: i64) -> String {
    format!("deleted+{user_id}@photovault.invalid")
}

/// Random URL-safe string of `n` random bytes (museum `GenerateURLSafeRandomString`).
pub fn random_url_safe(n: usize) -> String {
    crate::crypto::b64url(&crate::crypto::random_bytes(n))
}

/// Museum's `generateAuthToken`: base64url("auth-token" ‖ 32 random bytes).
pub fn generate_auth_token() -> String {
    let mut p = b"auth-token".to_vec();
    p.extend(crate::crypto::random_bytes(32));
    crate::crypto::b64url(&p)
}

/// Museum's `GetEncryptedToken`: seal the *decoded* token bytes to the user's public key.
pub fn seal_token(token: &str, public_key_b64: &str) -> ApiResult<String> {
    let pk = crate::crypto::unb64(public_key_b64).map_err(ApiError::internal)?;
    let raw = crate::crypto::unb64url(token).map_err(ApiError::internal)?;
    let ct = crate::crypto::box_seal(&raw, &pk).map_err(ApiError::internal)?;
    Ok(crate::crypto::b64(&ct))
}

/// Six random digits (museum `GenerateSixDigitOtp`).
pub fn six_digit_code() -> String {
    let b = crate::crypto::random_bytes(4);
    let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) % 1_000_000;
    format!("{n:06}")
}

/// A new random v4 UUID string.
pub fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
