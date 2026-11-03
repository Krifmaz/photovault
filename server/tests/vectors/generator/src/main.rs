//! Generates SRP / key-derivation / sealed-token fixtures with the upstream
//! Ente client crates (the same Rust code the web app runs as WebAssembly:
//! `rust/crates/accounts` and `rust/crates/core`, bound by
//! `rust/bindings/wasm/lib/src/prelogin.rs` into `ente-prelogin-wasm`).
//!
//! Usage:
//!   photovault-vecgen generate <out-dir>        write srp.json, keys.json, sealed_token.json
//!   photovault-vecgen verify-sealed <file.json> open a token sealed by OUR server (see README)

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ente_accounts::auth::{self, KeyDerivationStrength, SrpSession};
use ente_core::crypto::{self, Key, PublicKey, SecretKey, argon, sealed, secretbox};
use num_bigint::BigUint;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use srp::{ClientG4096, ServerG4096};
use std::path::Path;

const N_HEX: &str = "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aaac42dad33170d04507a33a85521abdf1cba64ecfb850458dbef0a8aea71575d060c7db3970f85a6e1e4c7abf5ae8cdb0933d71e8c94e04a25619dcee3d2261ad2ee6bf12ffa06d98a0864d87602733ec86a64521f2b18177b200cbbe117577a615d6c770988c0bad946e208e24fa074e5ab3143db5bfce0fd108e4b82d120a92108011a723c12a787e6d788719a10bdba5b2699c327186af4e23c1a946834b6150bda2583e9ca2ad44ce8dbbbc2db04de8ef92e8efc141fbecaa6287c59474e6bc05d99b2964fa090c3a2233ba186515be7ed1f612970cee2d7afb81bdd762170481cd0069127d5b05aa993b4ea988d8fddc186ffb7dc90a6c08f4df435c934063199ffffffffffffffff";
const N_BYTES: usize = 512;

fn b64(b: &[u8]) -> String {
    STANDARD.encode(b)
}

fn unb64(s: &str) -> Vec<u8> {
    STANDARD.decode(s).expect("base64")
}

fn pad(b: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; N_BYTES.saturating_sub(b.len())];
    out.extend_from_slice(b);
    out
}

fn sha(parts: &[&[u8]]) -> Vec<u8> {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().to_vec()
}

fn seq(start: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| start.wrapping_add(i as u8)).collect()
}

/// Independent server-side SRP-6a (what museum's go-srp does and what our
/// server must do), written from the formulas, not from the srp crate.
struct SrpServerMath {
    n: BigUint,
    g: BigUint,
    k: BigUint,
}

impl SrpServerMath {
    fn new() -> Self {
        let n = BigUint::parse_bytes(N_HEX.as_bytes(), 16).expect("N");
        let g = BigUint::from(5u32);
        // k = H(N | PAD(g))
        let k = BigUint::from_bytes_be(&sha(&[&n.to_bytes_be(), &pad(&g.to_bytes_be())]));
        Self { n, g, k }
    }

    fn x(&self, identity: &[u8], password: &[u8], salt: &[u8]) -> BigUint {
        let inner = sha(&[identity, b":", password]);
        BigUint::from_bytes_be(&sha(&[salt, &inner]))
    }

    fn verifier(&self, identity: &[u8], password: &[u8], salt: &[u8]) -> Vec<u8> {
        self.g
            .modpow(&self.x(identity, password, salt), &self.n)
            .to_bytes_be()
    }

    /// B = k*v + g^b mod N
    fn b_pub(&self, b: &[u8], v: &[u8]) -> Vec<u8> {
        let v = BigUint::from_bytes_be(v);
        let gb = self.g.modpow(&BigUint::from_bytes_be(b), &self.n);
        ((&self.k * v + gb) % &self.n).to_bytes_be()
    }

    /// Returns (u, S, M1, K, M2) as the server computes them from A.
    #[allow(clippy::type_complexity)]
    fn finish(
        &self,
        a_pub: &[u8],
        b_pub: &[u8],
        b: &[u8],
        v: &[u8],
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let u = sha(&[&pad(a_pub), &pad(b_pub)]);
        let a = BigUint::from_bytes_be(a_pub);
        let v = BigUint::from_bytes_be(v);
        let base = (a * v.modpow(&BigUint::from_bytes_be(&u), &self.n)) % &self.n;
        let s = base.modpow(&BigUint::from_bytes_be(b), &self.n).to_bytes_be();
        let m1 = sha(&[&pad(a_pub), &pad(b_pub), &pad(&s)]);
        let k = sha(&[&pad(&s)]);
        let m2 = sha(&[&pad(a_pub), &m1, &k]);
        (u, s, m1, k, m2)
    }
}

fn provenance(commit: &str) -> Value {
    json!({
        "generator": "server/tests/vectors/generator (photovault-vecgen)",
        "upstream": "https://github.com/ente-io/ente",
        "upstreamCommit": commit,
        "clientCode": [
            "rust/crates/accounts/src/auth/srp.rs (SrpSession: what web/packages/accounts/services/srp.ts calls via ente-prelogin-wasm)",
            "rust/crates/accounts/src/auth/api.rs (derive_kek, derive_srp_login_key, generate_srp_setup_with_login_key)",
            "rust/crates/accounts/src/auth/key_gen.rs (generate_keys_with_strength)",
            "rust/crates/core/src/crypto/{argon,kdf,sealed,secretbox}.rs",
            "rust/bindings/wasm/lib/src/prelogin.rs (wasm entry points authDeriveKek, authGenerateSrpSetup, authGenerateSignupKeys)"
        ],
        "srpCrate": "srp 0.7.0-rc.3 (RustCrypto), ClientG4096<Sha256>"
    })
}

fn gen_srp(commit: &str) -> Value {
    let math = SrpServerMath::new();
    let password = "correct horse battery staple";
    let email = "vectors@example.com";
    let kek_salt = seq(0x01, 16);
    // The web app's signup (KeyDerivationStrength::Sensitive) first tries
    // mem 256 MiB / ops 16 (argon.rs derive_sensitive_adaptive), so that is
    // what servers usually see. Recorded exactly as the client derives it.
    let params = argon::Params {
        mem_limit: 268_435_456,
        ops_limit: 16,
    };
    let kek = auth::derive_kek(password, &b64(&kek_salt), params.mem_limit, params.ops_limit)
        .expect("derive_kek");
    let login_key = auth::derive_srp_login_key(&kek).expect("login key");
    let srp_user_id = "4f9c2b1e-7d3a-4c5b-9e8f-0a1b2c3d4e5f";
    let srp_salt = seq(0x30, 16);

    // Setup exactly as generate_srp_setup_with_login_key does it (that fn
    // draws a random salt; we fix the salt, then cross-check the random path).
    let client = ClientG4096::<Sha256>::new();
    let verifier = client.compute_verifier(srp_user_id.as_bytes(), &login_key, &srp_salt);
    assert_eq!(
        verifier,
        math.verifier(srp_user_id.as_bytes(), &login_key, &srp_salt)
    );
    let rnd = auth::generate_srp_setup_with_login_key(&login_key, srp_user_id).expect("setup");
    assert_eq!(
        rnd.srp_verifier,
        math.verifier(srp_user_id.as_bytes(), &login_key, &rnd.srp_salt),
        "client verifier must match independent formula"
    );

    // Login transcript with fixed a and b.
    let a_priv = seq(0x40, 64);
    let b_priv = seq(0x80, 64);
    let a_pub = client.compute_public_ephemeral(&a_priv);
    let b_pub = math.b_pub(&b_priv, &verifier);
    let srv_b = ServerG4096::<Sha256>::new().compute_public_ephemeral(&b_priv, &verifier);
    assert_eq!(b_pub, srv_b, "B formula matches srp crate server");
    // Client side, exactly SrpSession::compute_m1 with a fixed `a`.
    let cv = client
        .process_reply(&a_priv, srp_user_id.as_bytes(), &login_key, &srp_salt, &b_pub)
        .expect("process_reply");
    let s_client = cv.key().to_vec();
    let m1 = sha(&[&pad(&a_pub), &pad(&b_pub), &pad(&s_client)]);
    let k_client = sha(&[&pad(&s_client)]);
    let m2_client = sha(&[&pad(&a_pub), &m1, &k_client]);
    let (u, s, m1_srv, k_srv, m2) = math.finish(&a_pub, &b_pub, &b_priv, &verifier);
    assert_eq!(s, s_client);
    assert_eq!(m1, m1_srv);
    assert_eq!(k_client, k_srv);
    assert_eq!(m2, m2_client);

    // Cross-check: the REAL client session (random a) against the
    // independent server math, several times.
    for _ in 0..8 {
        let mut sess = SrpSession::new(srp_user_id, &srp_salt, &login_key).expect("session");
        let a_wire = sess.public_a();
        assert_eq!(a_wire.len(), N_BYTES);
        let b_r = crypto::random_bytes(64);
        let bp = math.b_pub(&b_r, &verifier);
        let a_trim: Vec<u8> = a_wire.iter().copied().skip_while(|&x| x == 0).collect();
        let (_, _, m1_expected, _, m2_r) = math.finish(&a_trim, &bp, &b_r, &verifier);
        let m1_r = sess.compute_m1(&bp).expect("m1");
        assert_eq!(m1_r, m1_expected, "real client M1 == server expectation");
        sess.verify_m2(&m2_r).expect("real client accepts server M2");
    }

    // Upstream's own Go-server-checked vector (srp.rs
    // test_known_vector_leading_zero_a_matches_go_server_m1).
    let go_id = "repro-user-id";
    let go_salt = b"0123456789abcdef";
    let go_login = b"1234567890abcdef";
    let go_a = hex::decode(format!("{}06de", "0".repeat(124))).expect("hex");
    let go_b = [0x42u8; 64];
    let go_v = math.verifier(go_id.as_bytes(), go_login, go_salt);
    let go_bp = math.b_pub(&go_b, &go_v);
    let go_ap = client.compute_public_ephemeral(&go_a);
    let (_, _, go_m1, _, go_m2) = math.finish(&go_ap, &go_bp, &go_b, &go_v);
    assert_eq!(
        hex::encode(&go_m1),
        "c0953b8c74d400fbf664a515deb700d73b65231e7e207a2e2326ff8cf70567ac"
    );

    json!({
        "description": "SRP-6a as the Ente web client does it (RFC 5054 4096-bit group, g=5, SHA-256; I = srpUserID string bytes; P = loginSubKey (16 bytes); x = H(s | H(I | ':' | P)); k = H(N | PAD(g)); u = H(PAD(A) | PAD(B)); M1 = H(PAD(A) | PAD(B) | PAD(S)); K = H(PAD(S)); M2 = H(PAD(A) | M1 | K); PAD = left-pad to 512 bytes). Wire values are standard base64; the client sends srpA PADDED to 512 bytes and expects srpM2 to verify against PADDED A.",
        "provenance": provenance(commit),
        "group": {"N_hex": N_HEX, "g": 5, "hash": "SHA-256", "k_hex": hex::encode(math.k.to_bytes_be())},
        "inputs": {
            "email": email,
            "password": password,
            "kekSalt": b64(&kek_salt),
            "memLimit": params.mem_limit,
            "opsLimit": params.ops_limit
        },
        "derived": {
            "kek": b64(&kek),
            "loginSubKey": b64(&login_key),
            "loginSubKeyDerivation": "crypto_kdf_derive_from_key(32, id=1, ctx='loginctx', kek)[..16]"
        },
        "setup": {
            "srpUserID": srp_user_id,
            "srpSalt": b64(&srp_salt),
            "srpVerifier": b64(&verifier),
            "note": "POST /users/srp/setup body: {srpUserID, srpSalt, srpVerifier, srpA} (srpA from a separate client session)"
        },
        "login": {
            "a_hex": hex::encode(&a_priv),
            "srpA": b64(&pad(&a_pub)),
            "srpA_unpadded": b64(&a_pub),
            "b_hex": hex::encode(&b_priv),
            "srpB": b64(&b_pub),
            "u_hex": hex::encode(&u),
            "S_hex": hex::encode(&s),
            "srpM1": b64(&m1),
            "sessionKeyK": b64(&k_srv),
            "srpM2": b64(&m2),
            "note": "create-session request {srpUserID, srpA}; response {sessionID, srpB}. verify-session request {srpUserID, sessionID, srpM1}; response includes srpM2 (standard base64)."
        },
        "goServerVector": {
            "source": "upstream srp.rs test_known_vector_leading_zero_a_matches_go_server_m1 (checked against github.com/ente/go-srp)",
            "srpUserID": go_id,
            "srpSalt": b64(go_salt),
            "loginSubKey": b64(go_login),
            "a_hex": hex::encode(&go_a),
            "b_hex": hex::encode(go_b),
            "srpVerifier": b64(&go_v),
            "srpA": b64(&pad(&go_ap)),
            "srpB": b64(&go_bp),
            "srpM1": b64(&go_m1),
            "srpM2": b64(&go_m2)
        }
    })
}

fn gen_keys(commit: &str) -> (Value, Vec<u8>, Vec<u8>) {
    let password = "correct horse battery staple";
    // Interactive params keep tests fast; the structure is identical to signup.
    let keys = auth::generate_keys_with_strength(password, KeyDerivationStrength::Interactive)
        .expect("keys");
    let ka = &keys.key_attributes;
    let kek = auth::derive_kek(password, &ka.kek_salt, ka.mem_limit, ka.ops_limit).expect("kek");
    assert_eq!(&*kek, &*keys.key_encryption_key);
    let (master, secret) = auth::decrypt_keys_only(&kek, ka).expect("decrypt");
    let login_key = auth::derive_srp_login_key(&kek).expect("login");
    let recovery_hex: &str = &keys.private_key_attributes.recovery_key;
    let mnemonic = auth::recovery_key_to_mnemonic(&hex::decode(recovery_hex).expect("hex"))
        .expect("mnemonic");

    // A random secretbox under the master key, like collection keys.
    let collection_key = Key::generate();
    let enc = secretbox::encrypt(collection_key.as_bytes(), &master);
    let v = json!({
        "description": "Account key material exactly as the client generates it at signup (generate_keys_with_strength, Interactive argon2id params). Server stores keyAttributes verbatim; it can never decrypt them.",
        "provenance": provenance(commit),
        "password": password,
        "keyAttributes": serde_json::to_value(ka).expect("ser"),
        "plaintext": {
            "kek": b64(&kek),
            "loginSubKey": b64(&login_key),
            "masterKey": b64(master.as_bytes()),
            "secretKey": b64(secret.as_bytes()),
            "recoveryKeyHex": recovery_hex,
            "recoveryKeyMnemonic": mnemonic
        },
        "checks": [
            "kek = crypto_pwhash(32, password, kekSalt, opsLimit, memLimit, ARGON2ID13)",
            "masterKey = secretbox_open(encryptedKey, keyDecryptionNonce, kek)",
            "secretKey = secretbox_open(encryptedSecretKey, secretKeyDecryptionNonce, masterKey)",
            "publicKey = crypto_scalarmult_base(secretKey)",
            "masterKey = secretbox_open(masterKeyEncryptedWithRecoveryKey, masterKeyDecryptionNonce, recoveryKey)",
            "recoveryKey = secretbox_open(recoveryKeyEncryptedWithMasterKey, recoveryKeyDecryptionNonce, masterKey)"
        ],
        "collectionKeyBox": {
            "key": b64(master.as_bytes()),
            "plaintext": b64(collection_key.as_bytes()),
            "encryptedData": b64(&enc.encrypted_data),
            "nonce": b64(enc.nonce.as_bytes())
        }
    });
    (v, secret.as_bytes().to_vec(), secret.public_key().as_bytes().to_vec())
}

fn gen_sealed(commit: &str, sk: &[u8], pk: &[u8]) -> Value {
    let pk_t = PublicKey::try_from_slice(pk).expect("pk");
    let mut token = b"auth-token".to_vec();
    token.extend(crypto::random_bytes(32));
    let ct = sealed::seal(&token, &pk_t).expect("seal");
    let sk_t = SecretKey::try_from_slice(sk).expect("sk");
    assert_eq!(sealed::open(&ct, &pk_t, &sk_t).expect("open"), token);
    json!({
        "description": "crypto_box_seal of an auth token as the client opens it (login.rs finish_unlock: sealed::open with the account keypair, then validate_auth_token). When the API host is api.ente.com, api.ente.in or localhost the client REQUIRES the plaintext to be b'auth-token' || 32 random bytes (auth/payload.rs). The client then uses the raw bytes; X-Auth-Token is their URL-safe base64 (museum generateAuthToken).",
        "provenance": provenance(commit),
        "publicKey": b64(pk),
        "secretKey": b64(sk),
        "plaintext": b64(&token),
        "encryptedToken": b64(&ct)
    })
}

fn verify_sealed(path: &Path) {
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("json");
    let pk = unb64(v["publicKey"].as_str().expect("publicKey"));
    let sk = unb64(v["secretKey"].as_str().expect("secretKey"));
    let ct = unb64(v["encryptedToken"].as_str().expect("encryptedToken"));
    let pt = sealed::open(
        &ct,
        &PublicKey::try_from_slice(&pk).expect("pk"),
        &SecretKey::try_from_slice(&sk).expect("sk"),
    )
    .expect("ente-core sealed::open failed: the client would reject this token");
    if let Some(expected) = v["plaintext"].as_str() {
        assert_eq!(pt, unb64(expected), "plaintext mismatch");
    }
    let well_formed = pt.len() == 42 && pt.starts_with(b"auth-token");
    println!(
        "OK: ente-core opened the server-sealed token ({} bytes, auth-token payload: {well_formed})",
        pt.len()
    );
    if !well_formed {
        eprintln!("WARNING: a client talking to localhost/api.ente.com rejects tokens without the auth-token prefix");
        std::process::exit(2);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("generate") => {
            let out = Path::new(args.get(2).expect("out dir"));
            let commit = std::env::var("UPSTREAM_COMMIT").unwrap_or_else(|_| "unknown".into());
            std::fs::create_dir_all(out).expect("mkdir");
            let write = |name: &str, v: &Value| {
                let mut s = serde_json::to_string_pretty(v).expect("ser");
                s.push('\n');
                std::fs::write(out.join(name), s).expect("write");
                println!("wrote {}", out.join(name).display());
            };
            write("srp.json", &gen_srp(&commit));
            let (keys, sk, pk) = gen_keys(&commit);
            write("keys.json", &keys);
            write("sealed_token.json", &gen_sealed(&commit, &sk, &pk));
        }
        Some("verify-sealed") => verify_sealed(Path::new(args.get(2).expect("file"))),
        _ => {
            eprintln!("usage: photovault-vecgen generate <out-dir> | verify-sealed <file.json>");
            std::process::exit(2);
        }
    }
}
