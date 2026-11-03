//! Checks the server's libsodium against fixtures produced by the real Ente
//! client code (see tests/vectors/README.md for provenance).
//!
//! What the server itself does (sealed-box tokens, secretbox, generichash) is
//! tested through `photovault_server::crypto`; the rest (argon2id, KDF,
//! secretstream) is checked with the raw libsodium bindings so the fixtures
//! are proven self-consistent and usable by later workstreams.
//!
//! Set `PHOTOVAULT_VECTORS_OUT=<dir>` to also write `server_sealed_token.json`
//! (a token sealed by the server's `box_seal`) for the client-side verifiers
//! described in the README.

use libsodium_sys as ffi;
use photovault_server::crypto;
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vectors")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture")).expect("json")
}

fn b(v: &Value) -> Vec<u8> {
    crypto::unb64(v.as_str().expect("string field")).expect("base64")
}

fn pwhash(password: &str, salt: &[u8], ops: u64, mem: usize) -> Vec<u8> {
    crypto::init();
    let mut out = vec![0u8; 32];
    // SAFETY: all buffers are valid for the lengths passed.
    let rc = unsafe {
        ffi::crypto_pwhash(
            out.as_mut_ptr(),
            32,
            password.as_ptr().cast(),
            password.len() as u64,
            salt.as_ptr(),
            ops,
            mem,
            ffi::crypto_pwhash_ALG_ARGON2ID13 as i32,
        )
    };
    assert_eq!(rc, 0, "crypto_pwhash");
    out
}

fn login_subkey(kek: &[u8]) -> Vec<u8> {
    crypto::init();
    let mut out = vec![0u8; 32];
    let ctx = b"loginctx";
    // SAFETY: out is 32 bytes, ctx is 8 bytes, kek is 32 bytes.
    let rc = unsafe {
        ffi::crypto_kdf_derive_from_key(out.as_mut_ptr(), 32, 1, ctx.as_ptr().cast(), kek.as_ptr())
    };
    assert_eq!(rc, 0);
    out.truncate(16);
    out
}

fn scalarmult_base(sk: &[u8]) -> Vec<u8> {
    let mut pk = vec![0u8; 32];
    // SAFETY: both 32 bytes.
    unsafe { ffi::crypto_scalarmult_base(pk.as_mut_ptr(), sk.as_ptr()) };
    pk
}

/// Pull every chunk of a secretstream; returns (plaintexts, tags).
fn stream_pull(header: &[u8], key: &[u8], chunks: &[Vec<u8>]) -> (Vec<Vec<u8>>, Vec<u8>) {
    crypto::init();
    assert_eq!(header.len(), 24);
    assert_eq!(key.len(), 32);
    let mut st =
        std::mem::MaybeUninit::<ffi::crypto_secretstream_xchacha20poly1305_state>::uninit();
    // SAFETY: header is 24 bytes and key 32 bytes as checked above.
    let rc = unsafe {
        ffi::crypto_secretstream_xchacha20poly1305_init_pull(
            st.as_mut_ptr(),
            header.as_ptr(),
            key.as_ptr(),
        )
    };
    assert_eq!(rc, 0);
    let mut outs = vec![];
    let mut tags = vec![];
    for c in chunks {
        let mut m = vec![0u8; c.len() - 17];
        let mut mlen = 0u64;
        let mut tag = 0u8;
        // SAFETY: m has room for c.len() - ABYTES bytes; state was initialised.
        let rc = unsafe {
            ffi::crypto_secretstream_xchacha20poly1305_pull(
                st.as_mut_ptr(),
                m.as_mut_ptr(),
                &mut mlen,
                &mut tag,
                c.as_ptr(),
                c.len() as u64,
                std::ptr::null(),
                0,
            )
        };
        assert_eq!(rc, 0, "secretstream pull failed");
        m.truncate(mlen as usize);
        outs.push(m);
        tags.push(tag);
    }
    (outs, tags)
}

const TAG_MESSAGE: u8 = 0;
const TAG_FINAL: u8 = 3;

#[test]
fn keys_fixture_opens_with_server_libsodium() {
    let v = fixture("keys.json");
    let ka = &v["keyAttributes"];
    let pt = &v["plaintext"];
    let kek = pwhash(
        v["password"].as_str().expect("password"),
        &b(&ka["kekSalt"]),
        ka["opsLimit"].as_u64().expect("ops"),
        ka["memLimit"].as_u64().expect("mem") as usize,
    );
    assert_eq!(kek, b(&pt["kek"]), "argon2id kek");
    assert_eq!(login_subkey(&kek), b(&pt["loginSubKey"]));
    let master =
        crypto::secretbox_open(&b(&ka["encryptedKey"]), &b(&ka["keyDecryptionNonce"]), &kek)
            .expect("master key");
    assert_eq!(master, b(&pt["masterKey"]));
    let sk = crypto::secretbox_open(
        &b(&ka["encryptedSecretKey"]),
        &b(&ka["secretKeyDecryptionNonce"]),
        &master,
    )
    .expect("secret key");
    assert_eq!(sk, b(&pt["secretKey"]));
    assert_eq!(scalarmult_base(&sk), b(&ka["publicKey"]));
    let recovery = hex::decode(pt["recoveryKeyHex"].as_str().expect("hex")).expect("hex");
    assert_eq!(
        crypto::secretbox_open(
            &b(&ka["masterKeyEncryptedWithRecoveryKey"]),
            &b(&ka["masterKeyDecryptionNonce"]),
            &recovery
        )
        .expect("master via recovery"),
        master
    );
    assert_eq!(
        crypto::secretbox_open(
            &b(&ka["recoveryKeyEncryptedWithMasterKey"]),
            &b(&ka["recoveryKeyDecryptionNonce"]),
            &master
        )
        .expect("recovery via master"),
        recovery
    );
    let cb = &v["collectionKeyBox"];
    assert_eq!(
        crypto::secretbox_open(&b(&cb["encryptedData"]), &b(&cb["nonce"]), &b(&cb["key"]))
            .expect("collection key"),
        b(&cb["plaintext"])
    );
}

#[test]
fn client_sealed_token_opens_with_server_box_seal_open() {
    let v = fixture("sealed_token.json");
    let pt = crypto::box_seal_open(
        &b(&v["encryptedToken"]),
        &b(&v["publicKey"]),
        &b(&v["secretKey"]),
    )
    .expect("open");
    assert_eq!(pt, b(&v["plaintext"]));
    assert_eq!(pt.len(), 42);
    assert!(pt.starts_with(b"auth-token"));
}

/// The server seals; the result must open with the fixture's client keypair.
/// The same bytes are what the README's Node / ente-core verifiers check.
#[test]
fn server_sealed_token_opens_with_client_keypair() {
    let v = fixture("sealed_token.json");
    let pk = b(&v["publicKey"]);
    let sk = b(&v["secretKey"]);
    let mut token = b"auth-token".to_vec();
    token.extend(crypto::random_bytes(32));
    let sealed = crypto::box_seal(&token, &pk).expect("seal");
    assert_eq!(sealed.len(), token.len() + 48);
    assert_eq!(
        crypto::box_seal_open(&sealed, &pk, &sk).expect("open"),
        token
    );
    if let Ok(dir) = std::env::var("PHOTOVAULT_VECTORS_OUT") {
        let out = serde_json::json!({
            "description": "Token sealed by photovault_server::crypto::box_seal (tests/vectors_crypto.rs); verify with the client code per tests/vectors/README.md",
            "publicKey": crypto::b64(&pk),
            "secretKey": crypto::b64(&sk),
            "plaintext": crypto::b64(&token),
            "encryptedToken": crypto::b64(&sealed),
        });
        let path = std::path::Path::new(&dir).join("server_sealed_token.json");
        std::fs::write(&path, serde_json::to_string_pretty(&out).expect("ser")).expect("write");
        eprintln!("wrote {}", path.display());
    }
}

#[test]
fn srp_fixture_login_subkey_matches_libsodium_kdf() {
    let v = fixture("srp.json");
    assert_eq!(
        login_subkey(&b(&v["derived"]["kek"])),
        b(&v["derived"]["loginSubKey"])
    );
    let a = b(&v["login"]["srpA"]);
    assert_eq!(a.len(), 512, "client sends A padded to the group size");
}

/// Argon2id at the signup parameters the web app actually uses (256 MiB,
/// ops 16). Slow, so ignored by default: `cargo test --test vectors_crypto -- --ignored`.
#[test]
#[ignore = "slow: 256 MiB x 16 passes argon2id"]
fn srp_fixture_kek_matches_libsodium_pwhash() {
    let v = fixture("srp.json");
    let i = &v["inputs"];
    let kek = pwhash(
        i["password"].as_str().expect("password"),
        &b(&i["kekSalt"]),
        i["opsLimit"].as_u64().expect("ops"),
        i["memLimit"].as_u64().expect("mem") as usize,
    );
    assert_eq!(kek, b(&v["derived"]["kek"]));
}

#[test]
fn libsodium_fixture_secretbox_and_file_key() {
    let v = fixture("libsodium.json");
    let s = &v["secretbox"];
    assert_eq!(
        crypto::secretbox_open(&b(&s["encryptedData"]), &b(&s["nonce"]), &b(&s["key"]))
            .expect("secretbox"),
        b(&s["plaintext"])
    );
    let f = &v["fileKey"];
    assert_eq!(
        crypto::secretbox_open(
            &b(&f["encryptedKey"]),
            &b(&f["keyDecryptionNonce"]),
            &b(&f["collectionKey"])
        )
        .expect("file key"),
        b(&f["fileKey"])
    );
}

#[test]
fn libsodium_fixture_secretstream_payloads() {
    let v = fixture("libsodium.json");
    let m = &v["metadata"];
    let (out, tags) = stream_pull(
        &b(&m["decryptionHeader"]),
        &b(&m["key"]),
        &[b(&m["encryptedData"])],
    );
    assert_eq!(tags, vec![TAG_FINAL]);
    assert_eq!(out[0], m["json"].as_str().expect("json").as_bytes());

    for name in ["thumbnail", "file"] {
        let t = &v[name];
        let (out, tags) = stream_pull(
            &b(&t["decryptionHeader"]),
            &b(&t["key"]),
            &[b(&t["encryptedData"])],
        );
        assert_eq!(tags, vec![TAG_FINAL], "{name}");
        assert_eq!(out[0], b(&t["plaintext"]), "{name}");
    }

    let fs = &v["fileStream"];
    let chunks: Vec<Vec<u8>> = fs["chunks"]
        .as_array()
        .expect("chunks")
        .iter()
        .map(b)
        .collect();
    let (out, tags) = stream_pull(&b(&fs["decryptionHeader"]), &b(&fs["key"]), &chunks);
    assert_eq!(tags, vec![TAG_MESSAGE, TAG_FINAL]);
    let expected: Vec<Vec<u8>> = fs["plaintexts"]
        .as_array()
        .expect("plaintexts")
        .iter()
        .map(b)
        .collect();
    assert_eq!(out, expected);
}

#[test]
fn libsodium_fixture_sealed_box_and_hashes() {
    let v = fixture("libsodium.json");
    let s = &v["sealedBox"];
    assert_eq!(
        crypto::box_seal_open(&b(&s["sealed"]), &b(&s["publicKey"]), &b(&s["secretKey"]))
            .expect("sealed"),
        b(&s["plaintext"])
    );
    let g = &v["generichash"];
    assert_eq!(
        crypto::generichash(
            g["message"].as_str().expect("msg").as_bytes(),
            &b(&g["key"])
        ),
        b(&g["hash"])
    );
    // Unkeyed BLAKE2b-512 (the client's chunkHash* over file contents).
    let f = &v["file"];
    let data = b(&f["plaintext"]);
    let mut out = vec![0u8; 64];
    // SAFETY: out is 64 bytes; no key.
    unsafe {
        ffi::crypto_generichash(
            out.as_mut_ptr(),
            64,
            data.as_ptr(),
            data.len() as u64,
            std::ptr::null(),
            0,
        )
    };
    assert_eq!(out, b(&f["blake2b512"]));
}

#[test]
fn libsodium_fixture_argon2id_interactive() {
    let v = fixture("libsodium.json");
    let a = &v["argon2id"];
    let key = pwhash(
        a["password"].as_str().expect("password"),
        &b(&a["salt"]),
        a["opsLimit"].as_u64().expect("ops"),
        a["memLimit"].as_u64().expect("mem") as usize,
    );
    assert_eq!(key, b(&a["key"]));
}
