//! HS256 tokens for anonymous public-link identities (museum
//! `ente.NewAnonymousIdentityToken` / `ParseAnonymousIdentityToken`).
//!
//! Claims: `{"typ":"anon-identity","sub":<anonUserID>,"iss":…,"iat":…,"exp":…}`, valid
//! for one year, signed with the server's JWT key. The apps treat the token as opaque.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

const TOKEN_TYPE: &str = "anon-identity";
const VALIDITY_SECS: i64 = 365 * 24 * 3600;

#[derive(Serialize, Deserialize)]
struct Claims {
    typ: String,
    #[serde(default)]
    iss: String,
    sub: String,
    #[serde(default)]
    iat: i64,
    exp: i64,
}

fn mac(secret: &[u8]) -> Hmac<Sha256> {
    // HMAC accepts keys of any length.
    <Hmac<Sha256> as Mac>::new_from_slice(secret).unwrap_or_else(|_| unreachable!())
}

fn now_secs() -> i64 {
    crate::db::now_micros() / 1_000_000
}

/// Mint a token for `anon_user_id`; returns `(token, expires_at_micros)`.
pub fn issue(secret: &[u8], anon_user_id: &str) -> (String, i64) {
    let iat = now_secs();
    let exp = iat + VALIDITY_SECS;
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let claims = Claims {
        typ: TOKEN_TYPE.into(),
        iss: "photovault".into(),
        sub: anon_user_id.into(),
        iat,
        exp,
    };
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap_or_default());
    let signing_input = format!("{header}.{payload}");
    let mut m = mac(secret);
    m.update(signing_input.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(m.finalize().into_bytes());
    (format!("{signing_input}.{sig}"), exp * 1_000_000)
}

/// Verify a token and return its subject (the anon user id). `None` if the signature,
/// algorithm, type or expiry is wrong.
pub fn verify(secret: &[u8], token: &str) -> Option<String> {
    let mut parts = token.trim().split('.');
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
    let mut m = mac(secret);
    m.update(h.as_bytes());
    m.update(b".");
    m.update(p.as_bytes());
    m.verify_slice(&sig).ok()?;
    let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(p).ok()?).ok()?;
    if claims.typ != TOKEN_TYPE || claims.sub.is_empty() || claims.exp <= now_secs() {
        return None;
    }
    Some(claims.sub)
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip_and_tamper() {
        let (t, exp) = super::issue(b"k", "anon_x");
        assert!(exp > crate::db::now_micros());
        assert_eq!(super::verify(b"k", &t).as_deref(), Some("anon_x"));
        assert_eq!(super::verify(b"other", &t), None);
        assert_eq!(super::verify(b"k", &format!("{t}x")), None);
        assert_eq!(super::verify(b"k", "a.b"), None);
    }
}
