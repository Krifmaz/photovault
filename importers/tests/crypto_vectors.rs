//! Crypto compatibility with Ente, against vectors produced by Ente's own
//! code (see `tests/vectors/gen/README.md`):
//!
//! * `go.json`: the Ente CLI's `internal/crypto` package and museum's
//!   `ente/go-srp`.
//! * `node.json`: Ente web's `base/crypto/libsodium.ts` functions on
//!   `libsodium-wrappers-sumo` 0.7.15.
//!
//! These tests read only the committed JSON. The `#[ignore]`d tests at the
//! bottom go the other way (our output, Ente's Go code decrypts / verifies)
//! and need Go plus the generator's `ente-cli` link; run them with
//! `cargo test -p photovault-import --test crypto_vectors -- --ignored`.

use photovault_import::crypto::{
    self, ENCRYPTION_CHUNK_SIZE, Hasher, STREAM_ABYTES, StreamDecryptor, StreamEncryptor,
};
use serde_json::{Value, json};

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

// ---- other direction: Ente's Go code checks our output (needs Go) ----

fn go_cmd() -> std::process::Command {
    let go = std::env::var_os("GO").unwrap_or_else(|| "go".into());
    let mut cmd = std::process::Command::new(go);
    cmd.current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/gen/go"));
    cmd
}

/// Encrypt with Ente's chunking (4 MiB chunks, the last one FINAL).
fn encrypt_file(data: &[u8], key: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut enc, header) = StreamEncryptor::new(key).unwrap();
    let mut out = Vec::new();
    if data.is_empty() {
        out.extend(enc.push(&[], true).unwrap());
    } else {
        let n = data.len().div_ceil(ENCRYPTION_CHUNK_SIZE);
        for (i, c) in data.chunks(ENCRYPTION_CHUNK_SIZE).enumerate() {
            out.extend(enc.push(c, i + 1 == n).unwrap());
        }
    }
    (out, header)
}

#[test]
#[ignore = "needs Go and tests/vectors/gen/go/ente-cli"]
fn ente_cli_decrypts_our_output() {
    let dir = tempfile::tempdir().unwrap();
    let c = ENCRYPTION_CHUNK_SIZE as u64;
    let mut streams = Vec::new();
    for (i, len) in [0, 1, c, c + 1, 2 * c + 77].into_iter().enumerate() {
        let seed = 900 + i as u64;
        let key = crypto::generate_key();
        let (ct, header) = encrypt_file(&pattern(seed, len), &key);
        let path = dir.path().join(format!("stream-{i}.bin"));
        std::fs::write(&path, ct).unwrap();
        streams.push(json!({
            "key": crypto::b64(&key), "header": crypto::b64(&header),
            "seed": seed, "len": len, "ciphertext_file": path.to_str().unwrap(),
        }));
    }
    let key = crypto::generate_key();
    let (bct, bheader) = crypto::encrypt_blob(br#"{"title":"IMG_0001.HEIC"}"#, &key).unwrap();
    let (sct, nonce) = crypto::secretbox_seal(b"collection key bytes", &key).unwrap();
    let (pk, sk) = crypto::box_keypair();
    let sealed = crypto::box_seal(b"auth token", &pk).unwrap();
    let salt = crypto::random_bytes(16);
    let kek = crypto::derive_argon_key(b"hunter2", &salt, 2, 64 << 20).unwrap();
    let input = json!({
        "stream": streams,
        "blob": [{"key": crypto::b64(&key), "header": crypto::b64(&bheader),
                  "plaintext": crypto::b64(br#"{"title":"IMG_0001.HEIC"}"#), "ciphertext": crypto::b64(&bct)}],
        "secretbox": [{"key": crypto::b64(&key), "nonce": crypto::b64(&nonce),
                       "plaintext": crypto::b64(b"collection key bytes"), "ciphertext": crypto::b64(&sct)}],
        "sealed": [{"public_key": crypto::b64(&pk), "secret_key": crypto::b64(&sk),
                    "plaintext": crypto::b64(b"auth token"), "ciphertext": crypto::b64(&sealed)}],
        "argon": [{"password": "hunter2", "salt": crypto::b64(&salt), "ops_limit": 2, "mem_limit": 64 << 20,
                   "key": crypto::b64(&kek), "login_key": crypto::b64(&crypto::derive_login_key(&kek).unwrap())}],
    });
    let input_path = dir.path().join("verify.json");
    std::fs::write(&input_path, serde_json::to_vec(&input).unwrap()).unwrap();

    let out = go_cmd()
        .arg("run")
        .arg(".")
        .arg("verify")
        .arg(&input_path)
        .output()
        .expect("run go");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.starts_with("OK 9 checks"), "{stdout}");
}

#[test]
#[ignore = "needs Go and tests/vectors/gen/go/ente-cli"]
fn srp_login_against_museum_go_srp_server() {
    use photovault_import::srp::SrpClient;
    use std::io::{BufRead, BufReader, Write};

    // Sign-up side: the verifier the apps register, from museum's go-srp
    // (taken from the committed vectors so we only use go-srp's math).
    let g = go();
    let v = &list(&g, "srp")[0];
    let identity = v["identity"].as_str().unwrap();
    let (salt, login, verifier) = (
        bytes(v, "salt"),
        bytes(v, "login_key"),
        bytes(v, "verifier"),
    );

    for (password, should_pass) in [(login.clone(), true), (vec![0u8; 16], false)] {
        let mut child = go_cmd()
            .args(["run", ".", "srp-server"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("run go");
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = SrpClient::new(identity.as_bytes(), &salt, &password);
        let msg =
            json!({"verifier": crypto::b64(&verifier), "a_pub": crypto::b64(&client.public_a())});
        writeln!(stdin, "{msg}").unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        let m1 = client.compute_m1(&bytes(&reply, "b_pub")).unwrap();
        writeln!(stdin, "{}", json!({"m1": crypto::b64(&m1)})).unwrap();
        line.clear();
        stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert!(child.wait().unwrap().success());
        if should_pass {
            client.verify_m2(&bytes(&reply, "m2")).unwrap();
        } else {
            assert!(
                reply.get("error").is_some(),
                "server accepted a wrong password: {reply}"
            );
        }
    }
}
