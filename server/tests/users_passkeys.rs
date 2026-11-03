//! Passkey registration (accounts app) and passkey 2FA login, driven over HTTP with an
//! ES256 (P-256) software authenticator, as the accounts web app and mobile apps do it.
//! Runs on SQLite, or Postgres with `PHOTOVAULT_TEST_DATABASE_URL`.

mod users_common;

use ciborium::value::Value as Cbor;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use photovault_server::crypto;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use users_common::*;

const ORIGIN: &str = "http://localhost:3001";
const RP_ID: &str = "localhost";

/// Unpadded base64url, as browsers encode WebAuthn buffers.
fn u(b: &[u8]) -> String {
    crypto::b64url(b).trim_end_matches('=').to_string()
}

fn cbor(v: &Cbor) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(v, &mut out).unwrap();
    out
}

struct Authenticator {
    key: SigningKey,
    cred_id: Vec<u8>,
    count: u32,
}

impl Authenticator {
    fn new() -> Self {
        Authenticator {
            key: SigningKey::random(&mut p256::elliptic_curve::rand_core::OsRng),
            cred_id: crypto::random_bytes(16),
            count: 0,
        }
    }

    fn cose(&self) -> Vec<u8> {
        let pt = self.key.verifying_key().to_encoded_point(false);
        cbor(&Cbor::Map(vec![
            (Cbor::Integer(1.into()), Cbor::Integer(2.into())),
            (Cbor::Integer(3.into()), Cbor::Integer((-7).into())),
            (Cbor::Integer((-1).into()), Cbor::Integer(1.into())),
            (
                Cbor::Integer((-2).into()),
                Cbor::Bytes(pt.x().unwrap().to_vec()),
            ),
            (
                Cbor::Integer((-3).into()),
                Cbor::Bytes(pt.y().unwrap().to_vec()),
            ),
        ]))
    }

    /// `navigator.credentials.create` result as JSON (fmt "none").
    fn create(&self, options: &Value) -> Value {
        let pk = &options["publicKey"];
        assert_eq!(pk["rp"]["id"], RP_ID);
        assert!(
            pk["pubKeyCredParams"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["alg"] == -7)
        );
        let challenge = pk["challenge"].as_str().unwrap();
        let mut ad = Sha256::digest(RP_ID.as_bytes()).to_vec();
        ad.push(0x01 | 0x04 | 0x40); // UP | UV | AT
        ad.extend_from_slice(&self.count.to_be_bytes());
        ad.extend_from_slice(&[0u8; 16]);
        ad.extend_from_slice(&(self.cred_id.len() as u16).to_be_bytes());
        ad.extend_from_slice(&self.cred_id);
        ad.extend_from_slice(&self.cose());
        let att = cbor(&Cbor::Map(vec![
            (Cbor::Text("fmt".into()), Cbor::Text("none".into())),
            (Cbor::Text("attStmt".into()), Cbor::Map(vec![])),
            (Cbor::Text("authData".into()), Cbor::Bytes(ad)),
        ]));
        let cdj = json!({"type": "webauthn.create", "challenge": challenge, "origin": ORIGIN})
            .to_string();
        json!({
            "id": u(&self.cred_id),
            "rawId": u(&self.cred_id),
            "type": "public-key",
            "response": {
                "clientDataJSON": u(cdj.as_bytes()),
                "attestationObject": u(&att),
                "transports": ["internal"],
            }
        })
    }

    /// `navigator.credentials.get` result as JSON.
    fn get(&mut self, options: &Value, tamper: bool) -> Value {
        let pk = &options["publicKey"];
        assert_eq!(pk["rpId"], RP_ID);
        assert_eq!(pk["allowCredentials"][0]["id"], u(&self.cred_id));
        let challenge = pk["challenge"].as_str().unwrap();
        self.count += 1;
        let mut ad = Sha256::digest(RP_ID.as_bytes()).to_vec();
        ad.push(0x01 | 0x04);
        ad.extend_from_slice(&self.count.to_be_bytes());
        let cdj =
            json!({"type": "webauthn.get", "challenge": challenge, "origin": ORIGIN}).to_string();
        let mut msg = ad.clone();
        msg.extend_from_slice(&Sha256::digest(cdj.as_bytes()));
        if tamper {
            msg[0] ^= 1;
        }
        let sig: Signature = self.key.sign(&msg);
        json!({
            "id": u(&self.cred_id),
            "rawId": u(&self.cred_id),
            "type": "public-key",
            "response": {
                "clientDataJSON": u(cdj.as_bytes()),
                "authenticatorData": u(&ad),
                "signature": u(sig.to_der().as_bytes()),
                "userHandle": "",
            }
        })
    }
}

#[tokio::test]
async fn passkey_register_and_login() {
    let email = "pk@example.org";
    let srv = server(&[email]).await;
    let c = Client::new(&srv);
    let acc = signup(&c, email).await;
    let authed = c.with_token(&acc.token);

    // The accounts app gets an ACCOUNTS JWT; plain session tokens are refused.
    let (s, v) = authed.get("/users/accounts-token").await;
    assert_eq!(s, 200, "{v}");
    let accounts = c.with_token(v["accountsToken"].as_str().unwrap());
    let (s, _) = authed.get("/passkeys").await;
    assert_eq!(s, 401);
    let (s, v) = accounts.get("/passkeys").await;
    assert_eq!(s, 200);
    assert_eq!(v["passkeys"], Value::Null);

    // Registration.
    let mut auth = Authenticator::new();
    let (s, begin) = accounts
        .post("/passkeys/registration/begin", json!({}))
        .await;
    assert_eq!(s, 200, "{begin}");
    let cred = auth.create(&begin["options"]);
    let sid = begin["sessionID"].as_str().unwrap();
    let (s, _) = accounts
        .post(
            &format!("/passkeys/registration/finish?friendlyName=&sessionID={sid}"),
            cred.clone(),
        )
        .await;
    assert_eq!(s, 400, "empty name rejected");
    let (s, v) = accounts
        .post(
            &format!("/passkeys/registration/finish?friendlyName=Laptop&sessionID={sid}"),
            cred,
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let (_, v) = accounts.get("/passkeys").await;
    let list = v["passkeys"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["friendlyName"], "Laptop");
    let passkey_id = list[0]["id"].as_str().unwrap().to_string();
    let (s, _) = accounts
        .req(
            "PATCH",
            &format!("/passkeys/{passkey_id}?friendlyName=Phone"),
            None,
        )
        .await;
    assert_eq!(s, 200);
    let (_, v) = authed.get("/users/details/v2").await;
    assert_eq!(v["profileData"]["passkeyCount"], 1);

    // Login now stops at the passkey step.
    let (s, v) = srp_login(&c, &acc).await;
    assert_eq!(s, 200, "{v}");
    let session = v["passkeySessionID"].as_str().unwrap().to_string();
    assert!(!session.is_empty());
    assert!(v.get("encryptedToken").is_none() && v.get("token").is_none());
    let (s, v) = c
        .get(&format!(
            "/users/two-factor/passkeys/get-token?sessionID={session}"
        ))
        .await;
    assert_eq!((s, v["code"].as_str()), (400, Some("SESSION_NOT_VERIFIED")));

    // A bad signature is refused.
    let (s, begin) = c
        .post(
            "/users/two-factor/passkeys/begin",
            json!({"sessionID": session}),
        )
        .await;
    assert_eq!(s, 200, "{begin}");
    let ceremony = begin["ceremonySessionID"].as_str().unwrap().to_string();
    let bad = auth.get(&begin["options"], true);
    let (s, _) = c
        .post(
            &format!(
                "/users/two-factor/passkeys/finish?sessionID={session}&ceremonySessionID={ceremony}"
            ),
            bad,
        )
        .await;
    assert_eq!(s, 401);

    // A valid assertion yields the sealed token.
    let (_, begin) = c
        .post(
            "/users/two-factor/passkeys/begin",
            json!({"sessionID": session}),
        )
        .await;
    let ceremony = begin["ceremonySessionID"].as_str().unwrap().to_string();
    let good = auth.get(&begin["options"], false);
    let (s, v) = c
        .post(
            &format!(
                "/users/two-factor/passkeys/finish?sessionID={session}&ceremonySessionID={ceremony}"
            ),
            good,
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let token = open_token(
        &acc.public_key,
        &acc.secret_key,
        v["encryptedToken"].as_str().unwrap(),
    );
    let (s, _) = c.with_token(&token).get("/users/details/v2").await;
    assert_eq!(s, 200);
    // The web flow polls get-token once the session is verified.
    let (s, v) = c
        .get(&format!(
            "/users/two-factor/passkeys/get-token?sessionID={session}"
        ))
        .await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = c
        .post(
            "/users/two-factor/passkeys/begin",
            json!({"sessionID": session}),
        )
        .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (409, Some("SESSION_ALREADY_CLAIMED"))
    );

    // Deleting the passkey restores plain SRP login.
    let (s, _) = accounts
        .delete(&format!("/passkeys/{passkey_id}"), None)
        .await;
    assert_eq!(s, 200);
    let (s, v) = srp_login(&c, &acc).await;
    assert_eq!(s, 200);
    assert_eq!(v["passkeySessionID"], "");
    assert!(v["encryptedToken"].is_string());
}
