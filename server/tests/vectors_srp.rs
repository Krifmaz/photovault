//! SRP-6a server-side verification against a transcript computed by the real
//! Ente client code (tests/vectors/srp.json; provenance in tests/vectors/README.md).
//!
//! The adapters below call the SRP code the login routes use
//! (`photovault_server::users::srp`: `server_b` for `srpB`, `server_check_m1` for
//! `srpM1` → `srpM2`) and check that the legacy-recovery implementation
//! (`photovault_server::legacy::srp`) gives the same answers.

use photovault_server::crypto;
use photovault_server::legacy::srp as legacy_srp;
use photovault_server::users::srp;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Adapter: compute B (sent as `srpB`, padded to 512 bytes) for verifier `v` and private `b`.
fn server_public_b(v: &[u8], b: &[u8]) -> Vec<u8> {
    let out = srp::server_b(v, b);
    assert_eq!(out, legacy_srp::compute_b(v, b), "legacy::srp B differs");
    out
}

/// Adapter: verify (A, M1) for verifier `v` and private `b`; Some(M2) on success.
fn server_verify(v: &[u8], b: &[u8], a: &[u8], m1: &[u8]) -> Option<Vec<u8>> {
    let out = srp::server_check_m1(v, b, a, m1).ok();
    assert_eq!(
        out,
        legacy_srp::verify_m1(v, b, a, m1),
        "legacy::srp M2 differs"
    );
    out
}

fn fixture() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/srp.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("json")
}

fn b64(v: &Value) -> Vec<u8> {
    crypto::unb64(v.as_str().expect("string")).expect("base64")
}

fn hexv(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().expect("string")).expect("hex")
}

fn pad(x: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 512usize.saturating_sub(x.len())];
    out.extend_from_slice(x);
    out
}

fn sha(parts: &[&[u8]]) -> Vec<u8> {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().to_vec()
}

/// Runs now: the fixture's hash layout is what the server must reproduce.
#[test]
fn srp_fixture_hash_layout() {
    let l = &fixture()["login"];
    let a = b64(&l["srpA"]);
    let bp = b64(&l["srpB"]);
    let s = hexv(&l["S_hex"]);
    assert_eq!(a.len(), 512, "client sends PAD(A)");
    assert_eq!(hexv(&l["u_hex"]), sha(&[&pad(&a), &pad(&bp)]));
    let m1 = sha(&[&pad(&a), &pad(&bp), &pad(&s)]);
    assert_eq!(m1, b64(&l["srpM1"]));
    let k = sha(&[&pad(&s)]);
    assert_eq!(k, b64(&l["sessionKeyK"]));
    assert_eq!(sha(&[&pad(&a), &m1, &k]), b64(&l["srpM2"]));
}

#[test]
fn srp_server_accepts_client_transcript() {
    let f = fixture();
    let v = b64(&f["setup"]["srpVerifier"]);
    let l = &f["login"];
    let bpriv = hexv(&l["b_hex"]);
    assert_eq!(server_public_b(&v, &bpriv), b64(&l["srpB"]));
    let a = b64(&l["srpA"]);
    let m1 = b64(&l["srpM1"]);
    assert_eq!(
        server_verify(&v, &bpriv, &a, &m1),
        Some(b64(&l["srpM2"])),
        "server must accept the real client's M1 and return the M2 it verifies"
    );
    // Unpadded A is the same group element; accept it too.
    assert_eq!(
        server_verify(&v, &bpriv, &b64(&l["srpA_unpadded"]), &m1),
        Some(b64(&l["srpM2"]))
    );
    let mut bad = m1.clone();
    bad[0] ^= 1;
    assert_eq!(
        server_verify(&v, &bpriv, &a, &bad),
        None,
        "wrong M1 rejected"
    );
    // A = 0 (mod N) must be rejected (SRP-6a safety check).
    assert_eq!(server_verify(&v, &bpriv, &[0u8; 512], &m1), None);
}

#[test]
fn srp_server_matches_go_server_vector() {
    let g = &fixture()["goServerVector"];
    let v = b64(&g["srpVerifier"]);
    let bpriv = hexv(&g["b_hex"]);
    assert_eq!(server_public_b(&v, &bpriv), b64(&g["srpB"]));
    assert_eq!(
        server_verify(&v, &bpriv, &b64(&g["srpA"]), &b64(&g["srpM1"])),
        Some(b64(&g["srpM2"]))
    );
}
