//! Crypto compatibility with Ente, against vectors produced by Ente's own
//! code (see `tests/vectors/gen/README.md`):
//!
//! * `go.json`: the Ente CLI's `internal/crypto` package and museum's
//!   `ente/go-srp`.
//! * `node.json`: Ente web's `base/crypto/libsodium.ts` functions on
//!   `libsodium-wrappers-sumo` 0.7.15.
//!
//! Copied from `importers/tests/crypto_vectors.rs` (branch
//! `claude/project-thread-cp9x81`; the vectors and their generators live
//! there under `importers/tests/vectors/`). The tests that run Ente's Go code
//! against our output stay in the importers crate.

use photovault_core::crypto::{
    self, ENCRYPTION_CHUNK_SIZE, Hasher, STREAM_ABYTES, StreamDecryptor,
};
use serde_json::Value;

fn go() -> Value {
    serde_json::from_str(include_str!("vectors/go.json")).unwrap()
}

fn node() -> Value {
    serde_json::from_str(include_str!("vectors/node.json")).unwrap()
}

fn bytes(v: &Value, k: &str) -> Vec<u8> {
    crypto::unb64(v[k].as_str().unwrap_or_else(|| panic!("missing {k}"))).unwrap()
}

fn list<'a>(v: &'a Value, k: &str) -> &'a Vec<Value> {
    let l = v[k].as_array().unwrap_or_else(|| panic!("missing {k}"));
    assert!(!l.is_empty(), "{k} is empty");
    l
}

/// Deterministic plaintext shared with the generators.
fn pattern(seed: u64, n: u64) -> Vec<u8> {
    (0..n as usize)
        .map(|i| {
            ((i as u32)
                .wrapping_add(seed as u32)
                .wrapping_mul(2_654_435_761)
                >> 24) as u8
        })
        .collect()
}

#[test]
fn argon2id_matches_ente_cli_and_web() {
    let (g, n) = (go(), node());
    let (ga, na) = (list(&g, "argon"), list(&n, "argon"));
    assert_eq!(ga.len(), na.len());
    for (gv, nv) in ga.iter().zip(na) {
        let pw = gv["password"].as_str().unwrap();
        let salt = bytes(gv, "salt");
        let ops = gv["ops_limit"].as_u64().unwrap();
        let mem = gv["mem_limit"].as_u64().unwrap();
        let key = crypto::derive_argon_key(pw.as_bytes(), &salt, ops, mem).unwrap();
        assert_eq!(key, bytes(gv, "key"), "Go CLI, password {pw:?}");
        assert_eq!(key, bytes(nv, "key"), "web, password {pw:?}");
        assert_eq!(
            crypto::derive_login_key(&key).unwrap(),
            bytes(gv, "login_key")
        );
    }
}

#[test]
fn login_subkey_matches_ente_cli_and_libsodium() {
    for src in [go(), node()] {
        for v in list(&src, "login_key") {
            assert_eq!(
                crypto::derive_login_key(&bytes(v, "kek")).unwrap(),
                bytes(v, "login_key")
            );
        }
    }
}

#[test]
fn blobs_from_ente_cli_and_web_decrypt() {
    for src in [go(), node()] {
        for v in list(&src, "blob") {
            let pt = crypto::decrypt_blob(
                &bytes(v, "ciphertext"),
                &bytes(v, "header"),
                &bytes(v, "key"),
            )
            .unwrap();
            assert_eq!(pt, bytes(v, "plaintext"));
            // A single FINAL chunk is also a valid whole stream.
            let pt2 = crypto::decrypt_stream(
                &bytes(v, "ciphertext"),
                &bytes(v, "header"),
                &bytes(v, "key"),
            )
            .unwrap();
            assert_eq!(pt2, pt);
        }
    }
}

#[test]
fn streams_from_ente_web_decrypt() {
    let n = node();
    for v in list(&n, "stream_small") {
        let (key, header, ct) = (bytes(v, "key"), bytes(v, "header"), bytes(v, "ciphertext"));
        let want = pattern(v["seed"].as_u64().unwrap(), v["len"].as_u64().unwrap());
        assert_eq!(
            ct.len(),
            want.len() + STREAM_ABYTES,
            "one chunk, even when empty"
        );
        assert_eq!(crypto::decrypt_stream(&ct, &header, &key).unwrap(), want);

        let mut dec = StreamDecryptor::new(&key, &header).unwrap();
        let (pt, is_final) = dec.pull(&ct).unwrap();
        assert!(is_final);
        assert_eq!(pt, want);

        let mut tampered = ct.clone();
        *tampered.last_mut().unwrap() ^= 0x80;
        assert!(crypto::decrypt_stream(&tampered, &header, &key).is_err());
    }
}

#[test]
fn secretbox_from_ente_web_opens() {
    let n = node();
    for v in list(&n, "secretbox") {
        let pt = crypto::secretbox_open(
            &bytes(v, "ciphertext"),
            &bytes(v, "nonce"),
            &bytes(v, "key"),
        )
        .unwrap();
        assert_eq!(pt, bytes(v, "plaintext"));
    }
}

#[test]
fn sealed_box_from_ente_web_opens() {
    let n = node();
    for v in list(&n, "sealed") {
        let pt = crypto::box_seal_open(
            &bytes(v, "ciphertext"),
            &bytes(v, "public_key"),
            &bytes(v, "secret_key"),
        )
        .unwrap();
        assert_eq!(pt, bytes(v, "plaintext"));
    }
}

#[test]
fn content_hash_matches_ente_web() {
    let n = node();
    for v in list(&n, "hash") {
        let data = pattern(v["seed"].as_u64().unwrap(), v["len"].as_u64().unwrap());
        let mut h = Hasher::new();
        for c in data.chunks(ENCRYPTION_CHUNK_SIZE) {
            h.update(c);
        }
        assert_eq!(h.finalize(), bytes(v, "hash"));
        // Chunking does not matter.
        let mut h = Hasher::new();
        for c in data.chunks(7919) {
            h.update(c);
        }
        assert_eq!(h.finalize(), bytes(v, "hash"));
    }
    let mut h = Hasher::new();
    h.update(b"abc");
    assert_eq!(h.finalize(), bytes(&n, "hash_abc"));
}
