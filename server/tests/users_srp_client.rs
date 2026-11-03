//! Client-side SRP compatibility test.
//!
//! This file re-implements the *client* half of SRP independently of the server code
//! (it does not use `photovault_server::users::srp`), following the Ente clients line by
//! line, then signs up and logs in against the server over HTTP.
//!
//! Client sources matched (Ente monorepo, read-only reference):
//!
//! * `rust/crates/accounts/src/auth/srp.rs` — the SRP client used by the CLI, and by the
//!   web/desktop apps through the `ente-prelogin-wasm` build
//!   (`web/packages/accounts/services/srp.ts` calls `rustSetupSRP` / `rustVerifySRP`):
//!   - L22 `pad_to_n`: every value is left-padded to 512 bytes (4096-bit group);
//!   - L41 `ClientG4096::<Sha256>::new()`: RFC 5054 4096-bit group, g = 5, SHA-256,
//!     `k = H(pad(N) | pad(g))`, `x = H(salt | H(I ":" P))`, `u = H(pad(A) | pad(B))`;
//!   - L64-66 `public_a`: A is sent padded;
//!   - L84-95 `M1 = H(A | B | S)` over padded values;
//!   - L97-100 `K = H(S)`;
//!   - L123-127 `M2 = H(A | M1 | K)`, compared in constant time;
//!   - L188-189 `compute_verifier(srp_user_id, login_key, salt)`: identity is the
//!     srpUserID string, password is the 16-byte login key.
//! * `rust/crates/accounts/src/auth/key_gen.rs` L48/L107: the login key is
//!   `kdf::derive_login_key(kek)`; the server only ever sees it through the verifier.
//!
//! Runs on SQLite, or Postgres with `PHOTOVAULT_TEST_DATABASE_URL`.

mod users_common;

use num_bigint::BigUint;
use photovault_server::crypto;
use serde_json::json;
use sha2::{Digest, Sha256};
use users_common::*;

/// RFC 5054 Appendix A, 4096-bit group.
const N_HEX: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A92108011A723C12A787E6D788719A10BDBA5B2699C327186AF4E23C1A946834B6150BDA2583E9CA2AD44CE8DBBBC2DB04DE8EF92E8EFC141FBECAA6287C59474E6BC05D99B2964FA090C3A2233BA186515BE7ED1F612970CEE2D7AFB81BDD762170481CD0069127D5B05AA993B4EA988D8FDDC186FFB7DC90A6C08F4DF435C934063199FFFFFFFFFFFFFFFF";
const N_LEN: usize = 512;

fn n() -> BigUint {
    BigUint::parse_bytes(N_HEX.as_bytes(), 16).unwrap()
}

/// `pad_to_n` (srp.rs L22).
fn pad_to_n(b: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; N_LEN - b.len()];
    out.extend_from_slice(b);
    out
}

fn sha(parts: &[&[u8]]) -> Vec<u8> {
    let mut d = Sha256::new();
    for p in parts {
        d.update(p);
    }
    d.finalize().to_vec()
}

/// RustCrypto `srp` client: `x = H(salt | H(I ":" P))`.
fn x(identity: &str, login_key: &[u8], salt: &[u8]) -> BigUint {
    let inner = sha(&[identity.as_bytes(), b":", login_key]);
    BigUint::from_bytes_be(&sha(&[salt, &inner]))
}

/// `compute_verifier` (srp.rs L189).
fn verifier(identity: &str, login_key: &[u8], salt: &[u8]) -> Vec<u8> {
    let g = BigUint::from(5u32);
    g.modpow(&x(identity, login_key, salt), &n()).to_bytes_be()
}

struct ClientSession {
    a: BigUint,
    a_pub: Vec<u8>,
    m1: Vec<u8>,
    k: Vec<u8>,
}

impl ClientSession {
    fn new() -> Self {
        let a = BigUint::from_bytes_be(&crypto::random_bytes(64));
        let a_pub = pad_to_n(&BigUint::from(5u32).modpow(&a, &n()).to_bytes_be());
        ClientSession {
            a,
            a_pub,
            m1: vec![],
            k: vec![],
        }
    }

    /// `compute_m1` (srp.rs L69-104).
    fn compute_m1(&mut self, identity: &str, login_key: &[u8], salt: &[u8], b: &[u8]) -> Vec<u8> {
        assert!(b.len() <= N_LEN, "server B too large");
        let n = n();
        let g = BigUint::from(5u32);
        let k = BigUint::from_bytes_be(&sha(&[&pad_to_n(&n.to_bytes_be()), &pad_to_n(&[5])]));
        let b_pub = BigUint::from_bytes_be(b);
        let u = BigUint::from_bytes_be(&sha(&[&self.a_pub, &pad_to_n(b)]));
        let x = x(identity, login_key, salt);
        let kgx = (k * g.modpow(&x, &n)) % &n;
        let base = ((b_pub + &n) - kgx) % &n;
        let s = pad_to_n(&base.modpow(&(&self.a + u * x), &n).to_bytes_be());
        self.m1 = sha(&[&self.a_pub, &pad_to_n(b), &s]);
        self.k = sha(&[&s]);
        self.m1.clone()
    }

    /// `verify_m2` (srp.rs L111-133).
    fn verify_m2(&self, m2: &[u8]) -> bool {
        sha(&[&self.a_pub, &self.m1, &self.k]) == m2
    }
}

#[tokio::test]
async fn clients_srp_signup_and_login() {
    let email = "srp-client@example.org";
    let srv = server(&[email]).await;
    let c = Client::new(&srv);

    // Signup up to key attributes (shared helper), then SRP setup with this file's client.
    let (s, _) = c
        .post("/users/ott", json!({"email": email, "purpose": "signup"}))
        .await;
    assert_eq!(s, 200);
    let (_, v) = c
        .post("/users/verify-email", json!({"email": email, "ott": OTT}))
        .await;
    let authed = c.with_token(v["token"].as_str().unwrap());
    let (pk, sk) = crypto::box_keypair();
    let (s, _) = authed
        .put(
            "/users/attributes",
            json!({"keyAttributes": key_attributes(&pk)}),
        )
        .await;
    assert_eq!(s, 200);

    let identity = uuid::Uuid::new_v4().to_string();
    let salt = crypto::random_bytes(16);
    let login_key = crypto::random_bytes(16);
    let mut sess = ClientSession::new();
    let (s, setup) = authed
        .post(
            "/users/srp/setup",
            json!({
                "srpUserID": identity,
                "srpSalt": crypto::b64(&salt),
                "srpVerifier": crypto::b64(&verifier(&identity, &login_key, &salt)),
                "srpA": crypto::b64(&sess.a_pub),
            }),
        )
        .await;
    assert_eq!(s, 200, "{setup}");
    let b = crypto::unb64(setup["srpB"].as_str().unwrap()).unwrap();
    assert_eq!(b.len(), N_LEN, "server sends B padded");
    let m1 = sess.compute_m1(&identity, &login_key, &salt, &b);
    let (s, done) = authed
        .post(
            "/users/srp/complete",
            json!({"setupID": setup["setupID"], "srpM1": crypto::b64(&m1)}),
        )
        .await;
    assert_eq!(s, 200, "{done}");
    assert!(sess.verify_m2(&crypto::unb64(done["srpM2"].as_str().unwrap()).unwrap()));

    // Login: attributes → create-session → verify-session.
    let (_, attrs) = c.get(&format!("/users/srp/attributes?email={email}")).await;
    assert_eq!(attrs["attributes"]["srpUserID"], identity);
    assert_eq!(attrs["attributes"]["srpSalt"], crypto::b64(&salt));
    for (key, ok) in [(crypto::random_bytes(16), false), (login_key.clone(), true)] {
        let mut sess = ClientSession::new();
        let (s, cs) = c
            .post(
                "/users/srp/create-session",
                json!({"srpUserID": identity, "srpA": crypto::b64(&sess.a_pub)}),
            )
            .await;
        assert_eq!(s, 200, "{cs}");
        let b = crypto::unb64(cs["srpB"].as_str().unwrap()).unwrap();
        let m1 = sess.compute_m1(&identity, &key, &salt, &b);
        let (s, v) = c
            .post(
                "/users/srp/verify-session",
                json!({"sessionID": cs["sessionID"], "srpUserID": identity, "srpM1": crypto::b64(&m1)}),
            )
            .await;
        if !ok {
            assert_eq!(s, 401, "wrong password is rejected: {v}");
            continue;
        }
        assert_eq!(s, 200, "{v}");
        assert!(sess.verify_m2(&crypto::unb64(v["srpM2"].as_str().unwrap()).unwrap()));
        let token = open_token(&pk, &sk, v["encryptedToken"].as_str().unwrap());
        let (s, _) = c.with_token(&token).get("/users/details/v2").await;
        assert_eq!(s, 200);
    }
}
