//! SRP-6a exactly as museum's `github.com/ente/go-srp` (4096-bit RFC 5054 group, g = 5, SHA-256)
//! and the clients (`rust/crates/accounts/src/auth/srp.rs` in the Ente repo, used by web via
//! the `ente-prelogin-wasm` package, built on the RustCrypto `srp` 0.7.0-rc.3 crate plus
//! Ente's M1/M2 framing).
//!
//! With `pad()` = left-pad to 512 bytes, `H` = SHA-256 and all integers big-endian:
//!
//! * `k  = H(pad(N) | pad(g))`                          (go-srp `getMultiplier`)
//! * `x  = H(salt | H(I | ":" | P))`, I = srpUserID string, P = 16-byte login key (`getx`)
//! * `v  = g^x mod N`                                   (client-side verifier)
//! * `B  = (k·v + g^b) mod N`, sent as pad(B)           (`getB`; b = 32 random bytes)
//! * `u  = H(pad(A) | pad(B))`                          (`getu`)
//! * `S  = (A · v^u)^b mod N`  (server) = `(B − k·g^x)^(a + u·x) mod N` (client)
//! * `K  = H(pad(S))`
//! * `M1 = H(A | pad(B) | pad(S))`, A as the 512-byte value the client sent
//! * `M2 = H(A | M1 | K)`
//!
//! The server keeps only `b` (base64, column `srp_sessions.server_key`) and `A`.

use num_bigint::BigUint;
use num_traits::Zero;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

/// Byte length of N (4096 bits).
pub const N_BYTES: usize = 512;

const N_HEX: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A92108011A723C12A787E6D788719A10BDBA5B2699C327186AF4E23C1A946834B6150BDA2583E9CA2AD44CE8DBBBC2DB04DE8EF92E8EFC141FBECAA6287C59474E6BC05D99B2964FA090C3A2233BA186515BE7ED1F612970CEE2D7AFB81BDD762170481CD0069127D5B05AA993B4EA988D8FDDC186FFB7DC90A6C08F4DF435C934063199FFFFFFFFFFFFFFFF";

struct Group {
    n: BigUint,
    g: BigUint,
    k: BigUint,
}

static GROUP: LazyLock<Group> = LazyLock::new(|| {
    let n = BigUint::parse_bytes(N_HEX.as_bytes(), 16).expect("valid group constant");
    let g = BigUint::from(5u32);
    let k = BigUint::from_bytes_be(&h(&[&pad(&n), &pad(&g)]));
    Group { n, g, k }
});

/// SRP failure.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SrpError {
    /// A or B outside 1..N-1.
    #[error("invalid public value")]
    InvalidPublic,
    /// The client's proof did not match.
    #[error("client did not use the same password")]
    BadProof,
}

fn h(parts: &[&[u8]]) -> Vec<u8> {
    let mut d = Sha256::new();
    for p in parts {
        d.update(p);
    }
    d.finalize().to_vec()
}

/// Left-pad a number to [`N_BYTES`].
pub fn pad(x: &BigUint) -> Vec<u8> {
    let b = x.to_bytes_be();
    if b.len() >= N_BYTES {
        return b;
    }
    let mut out = vec![0u8; N_BYTES - b.len()];
    out.extend_from_slice(&b);
    out
}

fn check_public(x: &BigUint) -> Result<(), SrpError> {
    if x.is_zero() || x >= &GROUP.n {
        return Err(SrpError::InvalidPublic);
    }
    Ok(())
}

/// Server's public ephemeral `pad(B)` for verifier `v` and secret `b`.
pub fn server_b(verifier: &[u8], b_secret: &[u8]) -> Vec<u8> {
    let gp = &*GROUP;
    let v = BigUint::from_bytes_be(verifier);
    let b = BigUint::from_bytes_be(b_secret);
    pad(&((&gp.k * v + gp.g.modpow(&b, &gp.n)) % &gp.n))
}

/// Server side of the proof check. Returns `M2` when `M1` is right.
pub fn server_check_m1(
    verifier: &[u8],
    b_secret: &[u8],
    a_bytes: &[u8],
    m1: &[u8],
) -> Result<Vec<u8>, SrpError> {
    let gp = &*GROUP;
    let a = BigUint::from_bytes_be(a_bytes);
    check_public(&a)?;
    let v = BigUint::from_bytes_be(verifier);
    let b = BigUint::from_bytes_be(b_secret);
    let b_pub = pad(&((&gp.k * &v + gp.g.modpow(&b, &gp.n)) % &gp.n));
    let u = BigUint::from_bytes_be(&h(&[&pad(&a), &b_pub]));
    let s = pad(&((a * v.modpow(&u, &gp.n)) % &gp.n).modpow(&b, &gp.n));
    let k = h(&[&s]);
    let expected_m1 = h(&[a_bytes, &b_pub, &s]);
    if !bool::from(subtle::ConstantTimeEq::ct_eq(expected_m1.as_slice(), m1)) {
        return Err(SrpError::BadProof);
    }
    Ok(h(&[a_bytes, &expected_m1, &k]))
}

/// The client half, used by tests to play the apps' role (same formulas as
/// `SrpSession` in the Ente Rust accounts crate).
pub mod client {
    use super::*;

    /// `x = H(salt | H(I ":" P))`.
    pub fn x(identity: &[u8], password: &[u8], salt: &[u8]) -> BigUint {
        let ip = h(&[identity, b":", password]);
        BigUint::from_bytes_be(&h(&[salt, &ip]))
    }

    /// Verifier `pad(g^x)` as the apps upload it in `srpVerifier`.
    pub fn verifier(identity: &[u8], password: &[u8], salt: &[u8]) -> Vec<u8> {
        let gp = &*GROUP;
        pad(&gp.g.modpow(&x(identity, password, salt), &gp.n))
    }

    /// Client public ephemeral `pad(g^a)`.
    pub fn public_a(a_secret: &[u8]) -> Vec<u8> {
        let gp = &*GROUP;
        pad(&gp.g.modpow(&BigUint::from_bytes_be(a_secret), &gp.n))
    }

    /// Returns `(M1, expected M2)` for the server's `B`.
    pub fn proofs(
        identity: &[u8],
        password: &[u8],
        salt: &[u8],
        a_secret: &[u8],
        b_bytes: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), SrpError> {
        let gp = &*GROUP;
        let b_pub = BigUint::from_bytes_be(b_bytes);
        check_public(&b_pub)?;
        let a = BigUint::from_bytes_be(a_secret);
        let a_pub = pad(&gp.g.modpow(&a, &gp.n));
        let b_padded = pad(&b_pub);
        let u = BigUint::from_bytes_be(&h(&[&a_pub, &b_padded]));
        let x = x(identity, password, salt);
        let kgx = (&gp.k * gp.g.modpow(&x, &gp.n)) % &gp.n;
        let base = (b_pub + &gp.n - kgx) % &gp.n;
        let s = pad(&base.modpow(&(a + u * x), &gp.n));
        let k = h(&[&s]);
        let m1 = h(&[&a_pub, &b_padded, &s]);
        let m2 = h(&[&a_pub, &m1, &k]);
        Ok((m1, m2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vector from `test_known_vector_leading_zero_a_matches_go_server_m1` in the Ente
    /// repo (`rust/crates/accounts/src/auth/srp.rs`), which was generated with go-srp.
    #[test]
    fn go_srp_known_vector() {
        let id = b"repro-user-id";
        let salt = b"0123456789abcdef";
        let login_key = b"1234567890abcdef";
        let a_secret = hex::decode("000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000006de").unwrap();
        let b_secret = [0x42u8; 64];
        let v = client::verifier(id, login_key, salt);
        let b = server_b(&v, &b_secret);
        let a_pub = client::public_a(&a_secret);
        assert_eq!(a_pub.len(), N_BYTES);
        assert_eq!(a_pub[0], 0, "vector exercises a leading zero in A");
        let (m1, m2) = client::proofs(id, login_key, salt, &a_secret, &b).unwrap();
        assert_eq!(
            hex::encode(&m1),
            "c0953b8c74d400fbf664a515deb700d73b65231e7e207a2e2326ff8cf70567ac"
        );
        assert_eq!(server_check_m1(&v, &b_secret, &a_pub, &m1).unwrap(), m2);
        let mut bad = m1.clone();
        bad[0] ^= 1;
        assert_eq!(
            server_check_m1(&v, &b_secret, &a_pub, &bad),
            Err(SrpError::BadProof)
        );
        assert_eq!(
            server_check_m1(&v, &b_secret, &[0u8; N_BYTES], &m1),
            Err(SrpError::InvalidPublic)
        );
    }
}
