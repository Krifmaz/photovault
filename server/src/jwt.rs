//! Minimal HS256 JWTs for museum's scoped tokens (payment, families, accounts, delete-account,
//! account restore, public links).
//!
//! Wire format matches `github.com/golang-jwt/jwt/v4` as museum uses it: header
//! `{"alg":"HS256","typ":"JWT"}`, base64url (no padding) segments, HMAC-SHA256 signature.
//! Expiry is museum's custom `expiryTime` claim in **microseconds** (there is no `exp`).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// Museum's `ente/jwt.ClaimScope` values.
pub mod scope {
    /// Billing pages.
    pub const PAYMENT: &str = "PAYMENT";
    /// Family management pages.
    pub const FAMILIES: &str = "FAMILIES";
    /// Accounts app (passkeys).
    pub const ACCOUNTS: &str = "ACCOUNTS";
    /// Self-service account deletion challenge.
    pub const DELETE_ACCOUNT: &str = "DELETE_ACCOUNT";
    /// Link mailed after account deletion to undo it.
    pub const RESTORE_ACCOUNT: &str = "RestoreAccount";
}

/// Why a token was rejected.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum JwtError {
    /// Not three base64url segments, bad JSON, or unexpected algorithm.
    #[error("malformed token")]
    Malformed,
    /// Signature does not verify.
    #[error("bad signature")]
    BadSignature,
    /// `expiryTime` is in the past.
    #[error("token expired")]
    Expired,
    /// Valid token for another scope.
    #[error("unexpected scope")]
    WrongScope,
}

/// Museum's `WebCommonJWTClaim`. Field names and `omitempty` behaviour match Go.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebClaim {
    /// User the token is for.
    #[serde(rename = "userID", default, skip_serializing_if = "is_zero")]
    pub user_id: i64,
    /// Expiry, microseconds since the epoch.
    #[serde(rename = "expiryTime", default, skip_serializing_if = "is_zero")]
    pub expiry_time: i64,
    /// Email (only for account restore links).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// Scope; absent means PAYMENT (museum's `GetScope`).
    #[serde(
        rename = "claimScope",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub claim_scope: Option<String>,
    /// SHA-256 of the session token the JWT was minted from, standard base64 (Go `[]byte`).
    #[serde(
        rename = "sessionTokenHash",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub session_token_hash: String,
    /// App of that session (`photos`, `auth`, `locker`).
    #[serde(
        rename = "sessionApp",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub session_app: String,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

impl WebClaim {
    /// Effective scope (PAYMENT when unset, like museum).
    pub fn scope(&self) -> &str {
        self.claim_scope.as_deref().unwrap_or(scope::PAYMENT)
    }
}

type HmacSha256 = Hmac<Sha256>;

fn mac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

/// Sign any serialisable claims with HS256.
pub fn sign<T: Serialize>(key: &[u8], claims: &T) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap_or_default());
    let signing_input = format!("{header}.{payload}");
    let sig = URL_SAFE_NO_PAD.encode(mac(key, signing_input.as_bytes()));
    format!("{signing_input}.{sig}")
}

/// Verify the HS256 signature and decode the claims (no expiry check).
pub fn decode<T: DeserializeOwned>(key: &[u8], token: &str) -> Result<T, JwtError> {
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(s), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(JwtError::Malformed);
    };
    let header: serde_json::Value = URL_SAFE_NO_PAD
        .decode(h)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or(JwtError::Malformed)?;
    // golang-jwt accepts any HMAC method for an HMAC key; museum only issues HS256.
    if header.get("alg").and_then(|v| v.as_str()) != Some("HS256") {
        return Err(JwtError::Malformed);
    }
    let sig = URL_SAFE_NO_PAD.decode(s).map_err(|_| JwtError::Malformed)?;
    let mut m = HmacSha256::new_from_slice(key).map_err(|_| JwtError::Malformed)?;
    m.update(format!("{h}.{p}").as_bytes());
    m.verify_slice(&sig).map_err(|_| JwtError::BadSignature)?;
    let payload = URL_SAFE_NO_PAD.decode(p).map_err(|_| JwtError::Malformed)?;
    serde_json::from_slice(&payload).map_err(|_| JwtError::Malformed)
}

/// Verify a [`WebClaim`] token: signature, expiry, then scope.
pub fn verify_web(key: &[u8], token: &str, expected_scope: &str) -> Result<WebClaim, JwtError> {
    let claim: WebClaim = decode(key, token)?;
    if claim.expiry_time < crate::db::now_micros() {
        return Err(JwtError::Expired);
    }
    if claim.scope() != expected_scope {
        return Err(JwtError::WrongScope);
    }
    Ok(claim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper() {
        let key = b"secret";
        let claim = WebClaim {
            user_id: 7,
            expiry_time: crate::db::now_micros() + 1_000_000,
            claim_scope: Some(scope::ACCOUNTS.into()),
            ..Default::default()
        };
        let t = sign(key, &claim);
        assert_eq!(verify_web(key, &t, scope::ACCOUNTS).unwrap(), claim);
        assert_eq!(
            verify_web(key, &t, scope::FAMILIES),
            Err(JwtError::WrongScope)
        );
        assert_eq!(
            verify_web(b"other", &t, scope::ACCOUNTS),
            Err(JwtError::BadSignature)
        );
        let expired = sign(
            key,
            &WebClaim {
                expiry_time: 1,
                ..claim.clone()
            },
        );
        assert_eq!(
            verify_web(key, &expired, scope::ACCOUNTS),
            Err(JwtError::Expired)
        );
    }

    #[test]
    fn go_field_names() {
        let c = WebClaim {
            user_id: 1,
            expiry_time: 2,
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"userID":1,"expiryTime":2}"#
        );
    }
}
