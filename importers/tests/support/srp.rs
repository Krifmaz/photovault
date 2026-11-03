//! Server side of SRP-6a for the mock Ente server, copied from Photovault's
//! `server/src/legacy/srp.rs` (byte-for-byte the `go-srp` conventions museum
//! uses): 4096-bit RFC 5054 group, SHA-256, `k = H(pad(N) | pad(g))`,
//! `u = H(pad(A) | pad(B))`, `M1 = H(A | pad(B) | pad(S))`,
//! `M2 = H(A | M1 | H(pad(S)))`.

use num_bigint_dig::BigUint;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Byte length of group elements.
pub const N_LEN: usize = 512;

const N_HEX: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A92108011A723C12A787E6D788719A10BDBA5B2699C327186AF4E23C1A946834B6150BDA2583E9CA2AD44CE8DBBBC2DB04DE8EF92E8EFC141FBECAA6287C59474E6BC05D99B2964FA090C3A2233BA186515BE7ED1F612970CEE2D7AFB81BDD762170481CD0069127D5B05AA993B4EA988D8FDDC186FFB7DC90A6C08F4DF435C934063199FFFFFFFFFFFFFFFF";

fn n() -> &'static BigUint {
    static N: OnceLock<BigUint> = OnceLock::new();
    N.get_or_init(|| BigUint::parse_bytes(N_HEX.as_bytes(), 16).unwrap_or_default())
}

fn g() -> BigUint {
    BigUint::from(5u32)
}

fn pad(x: &BigUint) -> Vec<u8> {
    let b = x.to_bytes_be();
    if b.len() >= N_LEN {
        return b;
    }
    let mut out = vec![0u8; N_LEN - b.len()];
    out.extend_from_slice(&b);
    out
}

fn h(parts: &[&[u8]]) -> Vec<u8> {
    let mut d = Sha256::new();
    for p in parts {
        d.update(p);
    }
    d.finalize().to_vec()
}

fn int(b: &[u8]) -> BigUint {
    BigUint::from_bytes_be(b)
}

fn k() -> BigUint {
    int(&h(&[&pad(n()), &pad(&g())]))
}

/// Verifier `v = g^x mod N` with `x = H(salt | H(identity ":" password))`,
/// padded. This is what an app sends in `/users/srp/setup`.
pub fn verifier(salt: &[u8], identity: &[u8], password: &[u8]) -> Vec<u8> {
    let ip = h(&[identity, b":", password]);
    let x = int(&h(&[salt, &ip]));
    pad(&g().modpow(&x, n()))
}

/// Server public value `B` for verifier `v` and server secret `b`, padded.
pub fn compute_b(verifier: &[u8], secret: &[u8]) -> Vec<u8> {
    let v = int(verifier);
    let gb = g().modpow(&int(secret), n());
    pad(&((k() * v + gb) % n()))
}

/// Verify the client's `M1` (`a` exactly as the client sent it). Returns
/// `M2` on success.
pub fn verify_m1(verifier: &[u8], secret: &[u8], a: &[u8], m1: &[u8]) -> Option<Vec<u8>> {
    let a_int = int(a);
    if a_int == BigUint::from(0u32) || &a_int >= n() {
        return None;
    }
    let v = int(verifier);
    let b_pad = compute_b(verifier, secret);
    let u = int(&h(&[&pad(&a_int), &b_pad]));
    let s = pad(&(a_int * v.modpow(&u, n())).modpow(&int(secret), n()));
    let key = h(&[&s]);
    let expected = h(&[a, &b_pad, &s]);
    if expected != m1 {
        return None;
    }
    Some(h(&[a, &expected, &key]))
}
