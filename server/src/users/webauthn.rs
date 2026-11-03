//! Minimal WebAuthn relying party in pure Rust (museum uses go-webauthn).
//!
//! Supported: ES256 (P-256 ECDSA, COSE alg -7) and EdDSA (Ed25519, COSE alg -8) credentials;
//! attestation formats `none` and `packed` (the attestation statement itself is not verified,
//! matching go-webauthn's default `attestation: "none"` conveyance, which never asks for one).
//! Not supported: RS256 and other algorithms, attestation trust chains, token binding.
//!
//! Checks performed, as go-webauthn does: client data `type`, `challenge` and `origin`;
//! RP id hash; user-present flag; signature over `authenticatorData ‖ SHA-256(clientDataJSON)`;
//! credential belongs to the user; sign counter not going backwards (when non-zero).

use ciborium::value::Value;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// COSE algorithm ids we accept.
pub const ALG_ES256: i64 = -7;
/// EdDSA.
pub const ALG_EDDSA: i64 = -8;

/// Verification failure (the reason is only logged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebauthnError(pub String);

impl std::fmt::Display for WebauthnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(m: impl Into<String>) -> Result<T, WebauthnError> {
    Err(WebauthnError(m.into()))
}

/// Base64url without padding (what WebAuthn JSON uses); also accepts padded/standard input.
pub fn decode_b64url(s: &str) -> Result<Vec<u8>, WebauthnError> {
    let t = s.trim_end_matches('=').replace('+', "-").replace('/', "_");
    crate::crypto::unb64url(&t).map_err(|_| WebauthnError("bad base64url".into()))
}

/// Encode base64url without padding.
pub fn encode_b64url(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// `CollectedClientData`.
#[derive(Debug, Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    typ: String,
    challenge: String,
    origin: String,
}

fn check_client_data(
    client_data_json: &[u8],
    typ: &str,
    challenge: &str,
    origins: &[String],
) -> Result<(), WebauthnError> {
    let cd: ClientData =
        serde_json::from_slice(client_data_json).map_err(|_| WebauthnError("clientData".into()))?;
    if cd.typ != typ {
        return err("client data type mismatch");
    }
    let got = decode_b64url(&cd.challenge)?;
    if got != decode_b64url(challenge)? {
        return err("challenge mismatch");
    }
    let origin = cd.origin.trim_end_matches('/');
    if !origins.iter().any(|o| o.trim_end_matches('/') == origin) {
        return err(format!("origin {origin} not allowed"));
    }
    Ok(())
}

/// Parsed authenticator data.
#[derive(Debug, Clone)]
pub struct AuthData {
    /// SHA-256 of the RP id.
    pub rp_id_hash: [u8; 32],
    /// Flags byte.
    pub flags: u8,
    /// Signature counter.
    pub sign_count: u32,
    /// Attested credential id (registration only).
    pub credential_id: Option<Vec<u8>>,
    /// COSE public key bytes (registration only).
    pub cose_key: Option<Vec<u8>>,
}

const FLAG_UP: u8 = 0x01;
const FLAG_UV: u8 = 0x04;
const FLAG_BE: u8 = 0x08;
const FLAG_BS: u8 = 0x10;
const FLAG_AT: u8 = 0x40;

/// Parse authenticator data.
pub fn parse_auth_data(d: &[u8]) -> Result<AuthData, WebauthnError> {
    if d.len() < 37 {
        return err("authData too short");
    }
    let mut rp_id_hash = [0u8; 32];
    rp_id_hash.copy_from_slice(&d[..32]);
    let flags = d[32];
    let sign_count = u32::from_be_bytes([d[33], d[34], d[35], d[36]]);
    let (mut credential_id, mut cose_key) = (None, None);
    if flags & FLAG_AT != 0 {
        if d.len() < 55 {
            return err("attested data too short");
        }
        let len = u16::from_be_bytes([d[53], d[54]]) as usize;
        let start = 55;
        if d.len() < start + len {
            return err("credential id truncated");
        }
        credential_id = Some(d[start..start + len].to_vec());
        let rest = &d[start + len..];
        let mut cur = std::io::Cursor::new(rest);
        let _: Value =
            ciborium::de::from_reader(&mut cur).map_err(|_| WebauthnError("COSE key".into()))?;
        let used = cur.position() as usize;
        cose_key = Some(rest[..used].to_vec());
    }
    Ok(AuthData {
        rp_id_hash,
        flags,
        sign_count,
        credential_id,
        cose_key,
    })
}

fn map_get<'a>(m: &'a [(Value, Value)], k: i64) -> Option<&'a Value> {
    m.iter().find_map(|(key, v)| match key {
        Value::Integer(i) if i128::from(*i) == k as i128 => Some(v),
        _ => None,
    })
}

fn int(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Integer(i)) => i64::try_from(i128::from(*i)).ok(),
        _ => None,
    }
}

fn bytes(v: Option<&Value>) -> Option<&[u8]> {
    match v {
        Some(Value::Bytes(b)) => Some(b.as_slice()),
        _ => None,
    }
}

/// A credential public key.
#[derive(Debug, Clone)]
pub enum PublicKey {
    /// P-256 SEC1 uncompressed point.
    Es256(Vec<u8>),
    /// Ed25519 raw key.
    EdDsa([u8; 32]),
}

/// Parse a COSE_Key (EC2/P-256/ES256 or OKP/Ed25519/EdDSA).
pub fn parse_cose_key(b: &[u8]) -> Result<PublicKey, WebauthnError> {
    let v: Value = ciborium::de::from_reader(b).map_err(|_| WebauthnError("COSE".into()))?;
    let Value::Map(m) = v else {
        return err("COSE key not a map");
    };
    let kty = int(map_get(&m, 1));
    let alg = int(map_get(&m, 3));
    let crv = int(map_get(&m, -1));
    match (kty, alg, crv) {
        (Some(2), Some(ALG_ES256) | None, Some(1)) => {
            let x = bytes(map_get(&m, -2)).ok_or(WebauthnError("x".into()))?;
            let y = bytes(map_get(&m, -3)).ok_or(WebauthnError("y".into()))?;
            if x.len() != 32 || y.len() != 32 {
                return err("bad EC2 coordinates");
            }
            let mut p = vec![4u8];
            p.extend_from_slice(x);
            p.extend_from_slice(y);
            // Validate the point now so a bad key is rejected at registration.
            p256::ecdsa::VerifyingKey::from_sec1_bytes(&p)
                .map_err(|_| WebauthnError("invalid P-256 point".into()))?;
            Ok(PublicKey::Es256(p))
        }
        (Some(1), Some(ALG_EDDSA) | None, Some(6)) => {
            let x = bytes(map_get(&m, -2)).ok_or(WebauthnError("x".into()))?;
            let k: [u8; 32] = x
                .try_into()
                .map_err(|_| WebauthnError("ed25519 len".into()))?;
            Ok(PublicKey::EdDsa(k))
        }
        _ => err(format!(
            "unsupported COSE key kty={kty:?} alg={alg:?} crv={crv:?}"
        )),
    }
}

fn verify_sig(key: &PublicKey, msg: &[u8], sig: &[u8]) -> Result<(), WebauthnError> {
    match key {
        PublicKey::Es256(p) => {
            use p256::ecdsa::signature::Verifier;
            let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(p)
                .map_err(|_| WebauthnError("key".into()))?;
            let s = p256::ecdsa::Signature::from_der(sig)
                .or_else(|_| p256::ecdsa::Signature::from_slice(sig))
                .map_err(|_| WebauthnError("signature encoding".into()))?;
            vk.verify(msg, &s)
                .map_err(|_| WebauthnError("bad signature".into()))
        }
        PublicKey::EdDsa(k) => {
            let vk = ed25519_dalek::VerifyingKey::from_bytes(k)
                .map_err(|_| WebauthnError("key".into()))?;
            let s = ed25519_dalek::Signature::from_slice(sig)
                .map_err(|_| WebauthnError("signature encoding".into()))?;
            vk.verify_strict(msg, &s)
                .map_err(|_| WebauthnError("bad signature".into()))
        }
    }
}

/// `PublicKeyCredential` JSON as the accounts app posts it.
#[derive(Debug, Deserialize)]
pub struct CredentialJson {
    /// Credential id (base64url).
    pub id: String,
    /// Raw id (base64url).
    #[serde(rename = "rawId", default)]
    pub raw_id: String,
    /// `public-key`.
    #[serde(rename = "type", default)]
    pub typ: String,
    /// Authenticator response.
    pub response: ResponseJson,
}

/// Attestation or assertion response fields.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ResponseJson {
    /// Client data (base64url).
    #[serde(rename = "clientDataJSON")]
    pub client_data_json: String,
    /// Registration: attestation object.
    pub attestation_object: Option<String>,
    /// Registration: transports.
    pub transports: Option<Vec<String>>,
    /// Login: authenticator data.
    pub authenticator_data: Option<String>,
    /// Login: signature.
    pub signature: Option<String>,
    /// Login: user handle.
    pub user_handle: Option<String>,
}

/// A newly registered credential.
#[derive(Debug, Clone)]
pub struct NewCredential {
    /// Credential id bytes.
    pub id: Vec<u8>,
    /// COSE public key bytes.
    pub cose_key: Vec<u8>,
    /// Attestation format.
    pub fmt: String,
    /// Transports reported by the client.
    pub transports: Vec<String>,
    /// Initial sign count.
    pub sign_count: u32,
    /// Flags (UP, UV, BE, BS) for storage.
    pub flags: serde_json::Value,
}

fn flags_json(f: u8) -> serde_json::Value {
    serde_json::json!({
        "userPresent": f & FLAG_UP != 0,
        "userVerified": f & FLAG_UV != 0,
        "backupEligible": f & FLAG_BE != 0,
        "backupState": f & FLAG_BS != 0,
    })
}

/// Verify a registration (`navigator.credentials.create`) response.
pub fn verify_registration(
    cred: &CredentialJson,
    challenge: &str,
    rp_id: &str,
    origins: &[String],
) -> Result<NewCredential, WebauthnError> {
    if !cred.typ.is_empty() && cred.typ != "public-key" {
        return err("credential type");
    }
    let cdj = decode_b64url(&cred.response.client_data_json)?;
    check_client_data(&cdj, "webauthn.create", challenge, origins)?;
    let ao = decode_b64url(
        cred.response
            .attestation_object
            .as_deref()
            .ok_or(WebauthnError("attestationObject missing".into()))?,
    )?;
    let v: Value =
        ciborium::de::from_reader(ao.as_slice()).map_err(|_| WebauthnError("attObj".into()))?;
    let Value::Map(m) = v else {
        return err("attObj not a map");
    };
    let get = |k: &str| {
        m.iter().find_map(|(key, v)| match key {
            Value::Text(t) if t == k => Some(v),
            _ => None,
        })
    };
    let fmt = match get("fmt") {
        Some(Value::Text(t)) => t.clone(),
        _ => return err("fmt"),
    };
    if fmt != "none" && fmt != "packed" {
        return err(format!("unsupported attestation format {fmt}"));
    }
    let auth = match get("authData") {
        Some(Value::Bytes(b)) => b.clone(),
        _ => return err("authData"),
    };
    let ad = parse_auth_data(&auth)?;
    if ad.rp_id_hash[..] != Sha256::digest(rp_id.as_bytes())[..] {
        return err("rpIdHash mismatch");
    }
    if ad.flags & FLAG_UP == 0 {
        return err("user not present");
    }
    let id = ad
        .credential_id
        .ok_or(WebauthnError("no credential".into()))?;
    let cose_key = ad.cose_key.ok_or(WebauthnError("no key".into()))?;
    parse_cose_key(&cose_key)?;
    if !cred.id.is_empty() && decode_b64url(&cred.id)? != id {
        return err("credential id mismatch");
    }
    Ok(NewCredential {
        id,
        cose_key,
        fmt,
        transports: cred.response.transports.clone().unwrap_or_default(),
        sign_count: ad.sign_count,
        flags: flags_json(ad.flags),
    })
}

/// Verify an assertion (`navigator.credentials.get`) against a stored key. Returns the new
/// sign count.
pub fn verify_assertion(
    cred: &CredentialJson,
    challenge: &str,
    rp_id: &str,
    origins: &[String],
    cose_key: &[u8],
    stored_count: u32,
) -> Result<u32, WebauthnError> {
    let cdj = decode_b64url(&cred.response.client_data_json)?;
    check_client_data(&cdj, "webauthn.get", challenge, origins)?;
    let auth = decode_b64url(
        cred.response
            .authenticator_data
            .as_deref()
            .ok_or(WebauthnError("authenticatorData".into()))?,
    )?;
    let sig = decode_b64url(
        cred.response
            .signature
            .as_deref()
            .ok_or(WebauthnError("signature".into()))?,
    )?;
    let ad = parse_auth_data(&auth)?;
    if ad.rp_id_hash[..] != Sha256::digest(rp_id.as_bytes())[..] {
        return err("rpIdHash mismatch");
    }
    if ad.flags & FLAG_UP == 0 {
        return err("user not present");
    }
    let mut msg = auth.clone();
    msg.extend_from_slice(&Sha256::digest(&cdj));
    verify_sig(&parse_cose_key(cose_key)?, &msg, &sig)?;
    if (ad.sign_count != 0 || stored_count != 0) && ad.sign_count <= stored_count {
        // go-webauthn only flags this (CloneWarning); we log and accept like museum.
        tracing::warn!("passkey sign counter did not increase");
    }
    Ok(ad.sign_count)
}

/// Algorithms advertised in `pubKeyCredParams`.
pub fn pub_key_cred_params() -> serde_json::Value {
    serde_json::json!([
        {"type": "public-key", "alg": ALG_ES256},
        {"type": "public-key", "alg": ALG_EDDSA},
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{SigningKey, signature::Signer};

    fn cbor(v: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::ser::into_writer(v, &mut out).expect("cbor");
        out
    }

    fn es256_cose(sk: &SigningKey) -> Vec<u8> {
        let pt = sk.verifying_key().to_encoded_point(false);
        cbor(&Value::Map(vec![
            (Value::Integer(1.into()), Value::Integer(2.into())),
            (Value::Integer(3.into()), Value::Integer((-7).into())),
            (Value::Integer((-1).into()), Value::Integer(1.into())),
            (
                Value::Integer((-2).into()),
                Value::Bytes(pt.x().expect("x").to_vec()),
            ),
            (
                Value::Integer((-3).into()),
                Value::Bytes(pt.y().expect("y").to_vec()),
            ),
        ]))
    }

    #[test]
    fn es256_register_and_assert() {
        let sk = SigningKey::from_slice(&[7u8; 32]).expect("key");
        let rp = "localhost";
        let origins = vec!["http://localhost:3001".to_string()];
        let chal = encode_b64url(b"challenge-bytes-0123456789abcdef");
        let cred_id = b"cred-id-1".to_vec();
        let mut ad = Sha256::digest(rp.as_bytes()).to_vec();
        ad.push(FLAG_UP | FLAG_AT);
        ad.extend_from_slice(&0u32.to_be_bytes());
        ad.extend_from_slice(&[0u8; 16]);
        ad.extend_from_slice(&(cred_id.len() as u16).to_be_bytes());
        ad.extend_from_slice(&cred_id);
        ad.extend_from_slice(&es256_cose(&sk));
        let att = cbor(&Value::Map(vec![
            (Value::Text("fmt".into()), Value::Text("none".into())),
            (Value::Text("attStmt".into()), Value::Map(vec![])),
            (Value::Text("authData".into()), Value::Bytes(ad)),
        ]));
        let cdj = format!(
            r#"{{"type":"webauthn.create","challenge":"{chal}","origin":"http://localhost:3001"}}"#
        );
        let reg = CredentialJson {
            id: encode_b64url(&cred_id),
            raw_id: String::new(),
            typ: "public-key".into(),
            response: ResponseJson {
                client_data_json: encode_b64url(cdj.as_bytes()),
                attestation_object: Some(encode_b64url(&att)),
                ..Default::default()
            },
        };
        let nc = verify_registration(&reg, &chal, rp, &origins).expect("registration");
        assert_eq!(nc.id, cred_id);

        let mut ad2 = Sha256::digest(rp.as_bytes()).to_vec();
        ad2.push(FLAG_UP);
        ad2.extend_from_slice(&5u32.to_be_bytes());
        let cdj2 = format!(
            r#"{{"type":"webauthn.get","challenge":"{chal}","origin":"http://localhost:3001"}}"#
        );
        let mut msg = ad2.clone();
        msg.extend_from_slice(&Sha256::digest(cdj2.as_bytes()));
        let sig: p256::ecdsa::Signature = sk.sign(&msg);
        let asr = CredentialJson {
            id: encode_b64url(&cred_id),
            raw_id: String::new(),
            typ: "public-key".into(),
            response: ResponseJson {
                client_data_json: encode_b64url(cdj2.as_bytes()),
                authenticator_data: Some(encode_b64url(&ad2)),
                signature: Some(encode_b64url(sig.to_der().as_bytes())),
                ..Default::default()
            },
        };
        assert_eq!(
            verify_assertion(&asr, &chal, rp, &origins, &nc.cose_key, 0),
            Ok(5)
        );
        // Wrong origin and tampered data are rejected.
        assert!(
            verify_assertion(&asr, &chal, rp, &["https://evil".into()], &nc.cose_key, 0).is_err()
        );
        assert!(verify_assertion(&asr, &chal, "other", &origins, &nc.cose_key, 0).is_err());
    }
}
