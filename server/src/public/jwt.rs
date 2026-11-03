//! HS256 tokens used by public links, mirroring museum's `LinkPasswordClaim`
//! (`ente/jwt`) and `LinkDeviceClaim` (`pkg/controller/public/link_device_token.go`).
//!
//! Both claims carry their own microsecond `expiryTime`/`exp` and are validated
//! the way museum's custom `Valid()` methods do.

use crate::db::now_micros;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const MICROS_PER_DAY: i64 = 24 * 60 * 60 * 1_000_000;

/// Sign `claims` as a compact HS256 JWT.
pub fn sign<T: Serialize>(secret: &[u8], claims: &T) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap_or_default());
    let signing_input = format!("{header}.{payload}");
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(signing_input.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{signing_input}.{sig}")
}

/// Verify an HS256 JWT's signature and decode its claims. Expiry is checked by the caller.
pub fn verify<T: DeserializeOwned>(secret: &[u8], token: &str) -> Option<T> {
    let mut parts = token.split('.');
    let (h, p, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let header: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(h).ok()?).ok()?;
    if header.get("alg")?.as_str()? != "HS256" {
        return None;
    }
    let sig = URL_SAFE_NO_PAD.decode(s).ok()?;
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(format!("{h}.{p}").as_bytes());
    mac.verify_slice(&sig).ok()?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(p).ok()?).ok()
}

/// museum's `LinkPasswordClaim`: proof that the viewer knew a link's password.
#[derive(Debug, Serialize, Deserialize)]
pub struct LinkPasswordClaim {
    /// The link's password hash (as derived and sent by the client).
    #[serde(rename = "passKey")]
    pub pass_hash: String,
    /// Expiry, microseconds.
    #[serde(rename = "expiryTime")]
    pub expiry_time: i64,
}

/// Issue a 30-day password JWT for `pass_hash` (museum `verifyPassword`).
pub fn new_password_token(secret: &[u8], pass_hash: &str) -> String {
    sign(
        secret,
        &LinkPasswordClaim {
            pass_hash: pass_hash.to_string(),
            expiry_time: now_micros() + 30 * MICROS_PER_DAY,
        },
    )
}

/// True when `token` is a valid, unexpired password JWT for `pass_hash` (museum `validateJWTToken`).
pub fn password_token_valid(secret: &[u8], token: &str, pass_hash: &str) -> bool {
    match verify::<LinkPasswordClaim>(secret, token) {
        Some(c) => c.expiry_time >= now_micros() && c.pass_hash == pass_hash,
        None => false,
    }
}

/// Device-token scope for album links.
pub const SCOPE_COLLECTION: &str = "collection";
/// Device-token scope for single-file links.
pub const SCOPE_FILE: &str = "file";
const DEVICE_TYP: &str = "link-device";
const DEVICE_ISS: &str = "museum";
const DEVICE_AUD: &str = "public-link-device";
/// Refresh a device token when it has less than this long left (30 days, microseconds).
pub const DEVICE_TOKEN_REFRESH_BEFORE: i64 = 30 * MICROS_PER_DAY;

/// museum's `LinkDeviceClaim`: an already-admitted device for a link with a device limit.
#[derive(Debug, Serialize, Deserialize)]
pub struct LinkDeviceClaim {
    /// Always `link-device`.
    pub typ: String,
    /// `collection` or `file`.
    pub scope: String,
    /// Link id (stringified for collections).
    #[serde(rename = "linkID")]
    pub link_id: String,
    /// HMAC of the link's access token, so the token dies with the link.
    pub ath: String,
    /// Issued at, microseconds.
    pub iat: i64,
    /// Expiry, microseconds.
    pub exp: i64,
    /// Issuer.
    pub iss: String,
    /// Audience.
    pub aud: String,
}

fn access_token_hmac(secret: &[u8], access_token: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(b"ente:public-link-device-token:access-token:v1");
    mac.update(&[0]);
    mac.update(access_token.as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// Issue a device token valid for a year, or until the link's `valid_till` if sooner.
pub fn new_device_token(
    secret: &[u8],
    scope: &str,
    link_id: &str,
    access_token: &str,
    valid_till: i64,
) -> String {
    let now = now_micros();
    let mut exp = now + 365 * MICROS_PER_DAY;
    if valid_till > 0 && valid_till < exp {
        exp = valid_till;
    }
    sign(
        secret,
        &LinkDeviceClaim {
            typ: DEVICE_TYP.into(),
            scope: scope.into(),
            link_id: link_id.into(),
            ath: access_token_hmac(secret, access_token),
            iat: now,
            exp,
            iss: DEVICE_ISS.into(),
            aud: DEVICE_AUD.into(),
        },
    )
}

/// Validate a device token for this link; returns its claims when valid.
pub fn validate_device_token(
    secret: &[u8],
    token: &str,
    scope: &str,
    link_id: &str,
    access_token: &str,
) -> Option<LinkDeviceClaim> {
    let c: LinkDeviceClaim = verify(secret, token)?;
    let ok = c.exp >= now_micros()
        && c.typ == DEVICE_TYP
        && c.scope == scope
        && c.link_id == link_id
        && c.iss == DEVICE_ISS
        && c.aud == DEVICE_AUD;
    let expected = access_token_hmac(secret, access_token);
    let ath_ok: bool = subtle::ConstantTimeEq::ct_eq(c.ath.as_bytes(), expected.as_bytes()).into();
    (ok && ath_ok).then_some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let k = b"secret";
        let t = new_password_token(k, "hash");
        assert!(password_token_valid(k, &t, "hash"));
        assert!(!password_token_valid(k, &t, "other"));
        assert!(!password_token_valid(b"other", &t, "hash"));
        assert!(!password_token_valid(k, &format!("{t}x"), "hash"));
    }

    #[test]
    fn device_roundtrip() {
        let k = b"secret";
        let t = new_device_token(k, SCOPE_COLLECTION, "5", "TOKEN", 0);
        assert!(validate_device_token(k, &t, SCOPE_COLLECTION, "5", "TOKEN").is_some());
        assert!(validate_device_token(k, &t, SCOPE_FILE, "5", "TOKEN").is_none());
        assert!(validate_device_token(k, &t, SCOPE_COLLECTION, "6", "TOKEN").is_none());
        assert!(validate_device_token(k, &t, SCOPE_COLLECTION, "5", "OTHER").is_none());
    }
}
