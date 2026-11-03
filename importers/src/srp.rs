//! SRP-6a client, wire-compatible with Ente (4096-bit group, SHA-256, the
//! `fast-srp-hap` / `go-srp` conventions Ente uses).
//!
//! Ente's server (museum) uses `github.com/ente/go-srp`; the apps use clients
//! that agree with it byte for byte. With `N` the RFC 5054 4096-bit prime,
//! `g = 5`, `H = SHA-256`, `pad(x)` the 512-byte big-endian encoding and `|`
//! concatenation:
//!
//! * `k = H(pad(N) | pad(g))`
//! * `x = H(salt | H(identity | ":" | password))`, where `identity` is the
//!   `srpUserID` UUID string and `password` the raw 16-byte login sub-key
//! * `A = g^a mod N`, sent padded to 512 bytes
//! * `u = H(pad(A) | pad(B))`
//! * `S = (B - k·g^x)^(a + u·x) mod N`
//! * `M1 = H(pad(A) | pad(B) | pad(S))`
//! * `K = H(pad(S))` and `M2 = H(pad(A) | M1 | K)`
//!
//! museum hashes `A` *exactly as received* in `M1` and `M2`, so a client must
//! hash the same bytes it sent; we always send and hash the padded form, as
//! Ente's web, mobile and Rust clients do. (The Go CLI's older
//! `kong/go-srp` client sends and hashes `A` unpadded, which also works but
//! differs whenever `A` has a leading zero byte.)

use crate::crypto::{self, CryptoError};
use libsodium_sys as ffi;
use num_bigint_dig::BigUint;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Byte length of group elements (4096 bits).
const N_LEN: usize = 512;
/// Length of the random client secret `a` (512 bits, as Ente's Rust client).
const SECRET_LEN: usize = 64;

/// RFC 5054 4096-bit group prime.
const N_HEX: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A92108011A723C12A787E6D788719A10BDBA5B2699C327186AF4E23C1A946834B6150BDA2583E9CA2AD44CE8DBBBC2DB04DE8EF92E8EFC141FBECAA6287C59474E6BC05D99B2964FA090C3A2233BA186515BE7ED1F612970CEE2D7AFB81BDD762170481CD0069127D5B05AA993B4EA988D8FDDC186FFB7DC90A6C08F4DF435C934063199FFFFFFFFFFFFFFFF";

/// Group generator.
const G: u32 = 5;

fn n() -> &'static BigUint {
    static N: OnceLock<BigUint> = OnceLock::new();
    N.get_or_init(|| BigUint::parse_bytes(N_HEX.as_bytes(), 16).expect("valid group prime"))
}

fn g() -> BigUint {
    BigUint::from(G)
}

/// `k = H(pad(N) | pad(g))`.
fn k() -> &'static BigUint {
    static K: OnceLock<BigUint> = OnceLock::new();
    K.get_or_init(|| int(&h(&[&pad(n()), &pad(&g())])))
}

/// Big-endian encoding left-padded with zeros to [`N_LEN`] bytes. Callers
/// only pass values reduced mod `N`, which always fit.
fn pad(x: &BigUint) -> Vec<u8> {
    let b = x.to_bytes_be();
    let mut out = vec![0u8; N_LEN.saturating_sub(b.len())];
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

/// `x = H(salt | H(identity | ":" | password))`.
fn compute_x(identity: &[u8], salt: &[u8], password: &[u8]) -> BigUint {
    let inner = h(&[identity, b":", password]);
    int(&h(&[salt, &inner]))
}

/// Constant-time equality (libsodium `sodium_memcmp`).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    crypto::init();
    // SAFETY: both pointers are valid for `a.len()` bytes (lengths checked
    // equal above).
    unsafe { ffi::sodium_memcmp(a.as_ptr().cast(), b.as_ptr().cast(), a.len()) == 0 }
}

/// An in-progress SRP login.
pub struct SrpClient {
    /// Client secret `a`.
    a: BigUint,
    /// `x`, derived from the identity, salt and password.
    x: BigUint,
    /// `A = g^a mod N`, padded to [`N_LEN`].
    a_pub: Vec<u8>,
    /// `(M1, K)` once [`Self::compute_m1`] has run.
    proof: Option<(Vec<u8>, Vec<u8>)>,
}

impl SrpClient {
    /// Start a session. `identity` is the `srpUserID` (UUID string), `salt`
    /// the decoded `srpSalt`, `password` the 16-byte login sub-key from
    /// [`crate::crypto::derive_login_key`].
    pub fn new(identity: &[u8], salt: &[u8], password: &[u8]) -> Self {
        let mut secret = crypto::random_bytes(SECRET_LEN);
        let client = Self::with_secret(identity, salt, password, &secret);
        crypto::init();
        // SAFETY: `secret` is valid for writes of secret.len() bytes.
        unsafe { ffi::sodium_memzero(secret.as_mut_ptr().cast(), secret.len()) };
        client
    }

    /// Start a session with a fixed client secret `a` (big-endian bytes).
    fn with_secret(identity: &[u8], salt: &[u8], password: &[u8], secret: &[u8]) -> Self {
        let a = int(secret);
        let a_pub = pad(&g().modpow(&a, n()));
        Self {
            a,
            x: compute_x(identity, salt, password),
            a_pub,
            proof: None,
        }
    }

    /// The public ephemeral `A`, big-endian, padded to the group size.
    pub fn public_a(&self) -> Vec<u8> {
        self.a_pub.clone()
    }

    /// Process the server's `B` and return the client proof `M1`.
    ///
    /// Rejects a `B` that is zero, not below `N`, or longer than 512 bytes,
    /// and a degenerate `u = 0`.
    pub fn compute_m1(&mut self, server_b: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if server_b.len() > N_LEN {
            return Err(CryptoError::Invalid("SRP B too long"));
        }
        let b = int(server_b);
        let zero = BigUint::from(0u32);
        if b == zero || &b >= n() {
            return Err(CryptoError::Invalid("SRP B out of range"));
        }
        let b_pad = pad(&b);
        let u = int(&h(&[&self.a_pub, &b_pad]));
        if u == zero {
            return Err(CryptoError::Invalid("SRP u is zero"));
        }
        // S = (B - k·g^x)^(a + u·x) mod N, with the subtraction kept non-negative.
        let kgx = (k() * g().modpow(&self.x, n())) % n();
        let base = (b + n() - kgx) % n();
        let exp = &self.a + u * &self.x;
        let s = pad(&base.modpow(&exp, n()));
        let m1 = h(&[&self.a_pub, &b_pad, &s]);
        let key = h(&[&s]);
        self.proof = Some((m1.clone(), key));
        Ok(m1)
    }

    /// Check the server proof `M2` (call after [`Self::compute_m1`]).
    pub fn verify_m2(&self, m2: &[u8]) -> Result<(), CryptoError> {
        let (m1, key) = self
            .proof
            .as_ref()
            .ok_or(CryptoError::Invalid("SRP M1 not computed yet"))?;
        let expected = h(&[&self.a_pub, m1, key]);
        if ct_eq(&expected, m2) {
            Ok(())
        } else {
            Err(CryptoError::Invalid("SRP server proof mismatch"))
        }
    }

    /// The shared session key `K = H(pad(S))` (tests only; Ente does not use it).
    #[cfg(test)]
    fn session_key(&self) -> Option<&[u8]> {
        self.proof.as_ref().map(|(_, k)| k.as_slice())
    }
}

/// The verifier `v = g^x mod N`, padded, as the apps register it at sign-up.
#[cfg(test)]
fn verifier(identity: &[u8], salt: &[u8], password: &[u8]) -> Vec<u8> {
    pad(&g().modpow(&compute_x(identity, salt, password), n()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::unb64;

    /// A minimal go-srp-style server, for round trips with random secrets.
    struct Server {
        v: BigUint,
        b: BigUint,
        b_pub: Vec<u8>,
    }

    impl Server {
        fn new(verifier: &[u8], secret: &[u8]) -> Self {
            let v = int(verifier);
            let b = int(secret);
            let b_pub = pad(&((k() * &v + g().modpow(&b, n())) % n()));
            Self { v, b, b_pub }
        }

        fn check(&self, a: &[u8], m1: &[u8]) -> Option<Vec<u8>> {
            let a_int = int(a);
            let u = int(&h(&[&pad(&a_int), &self.b_pub]));
            let s = pad(&(a_int * self.v.modpow(&u, n())).modpow(&self.b, n()));
            let expected = h(&[a, &self.b_pub, &s]);
            (expected == m1).then(|| h(&[a, &expected, &h(&[&s])]))
        }
    }

    #[test]
    fn go_srp_verifier_vector() {
        // go-srp's TestCreateVerifier.
        let expected = "F0E47F50F5DEAD8DB8D93A279E3B62D6FF50854B31FBD3474A886BEF916261717E84DD4FB8B4D27FEAA5146DB7B1CBBC274FDF96A132B5029C2CD72527427A9B9809D5A4D018252928B4FC343BC17CE63C1859D5806F5466014FC361002D8890AEB4D6316FF37331FC2761BE0144C91CDD8E00ED0138C0CE51534D1B9A9BA629D7BE34D2742DD4097DAABC9ECB7AAAD89E53C342B038F1D2ADAE1F2410B7884A3E9A124C357E421BCCD4524467E1922660E0A4460C5F7C38C0877B65F6E32F28296282A93FC11BBABB7BB69BF1B3F9391991D8A86DD05E15000B7E38BA38A536BB0BF59C808EC25E791B8944719488B8087DF8BFD7FF20822997A53F6C86F3D45D004476D6303301376BB25A9F94B552CCE5ED40DE5DD7DA8027D754FA5F66738C7E3FC4EF3E20D625DF62CBE6E7ADFC21E47880D8A6ADA37E60370FD4D8FC82672A90C29F2E72F35652649D68348DE6F36D0E435C8BD42DD00155D35D501BECC0661B43E04CDB2DA84CE92B8BF49935D73D75EFCBD1176D7BBCCC3CC4D4B5FEFCC02D478614EE1681D2FF3C711A61A7686EB852AE06FB8227BE21FB8802719B1271BA1C02B13BBF0A2C2E459D9BEDCC8D1269F6A785CB4563AA791B38FB038269F63F58F47E9051499549789269CC7B8EC7026FC34BA73289C4AF829D5A532E723967CE9B6C023EF0FD0CFE37F51F10F19463B6534159A09DDD2F51F3B30033";
        assert_eq!(
            hex::encode_upper(verifier(b"alice", b"salty", b"password123")),
            expected
        );
    }

    /// Vectors from museum's `ente/go-srp` (server and client) with fixed
    /// secrets, including cases where `A`, `B` or `S` have a leading zero.
    #[test]
    fn ente_go_srp_vectors() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/vectors/go.json")).unwrap();
        let cases = v["srp"].as_array().unwrap();
        let notes: Vec<&str> = cases.iter().map(|c| c["note"].as_str().unwrap()).collect();
        for want in [
            "A has a leading zero byte",
            "B has a leading zero byte",
            "S has a leading zero byte",
        ] {
            assert!(notes.contains(&want), "missing vector: {want}");
        }
        for c in cases {
            let f = |k: &str| unb64(c[k].as_str().unwrap()).unwrap();
            let identity = c["identity"].as_str().unwrap().as_bytes();
            let (salt, login) = (f("salt"), f("login_key"));
            assert_eq!(verifier(identity, &salt, &login), f("verifier"));
            let mut client = SrpClient::with_secret(identity, &salt, &login, &f("client_secret"));
            assert_eq!(client.public_a(), f("a_pub"));
            assert_eq!(
                client.compute_m1(&f("b_pub")).unwrap(),
                f("m1"),
                "{}",
                c["note"]
            );
            assert_eq!(client.session_key().unwrap(), f("k").as_slice());
            client.verify_m2(&f("m2")).unwrap();
            let mut bad = f("m2");
            bad[0] ^= 1;
            assert!(client.verify_m2(&bad).is_err());
            // B without its leading zeros is the same number.
            let b = f("b_pub");
            let stripped: Vec<u8> = b.iter().copied().skip_while(|&x| x == 0).collect();
            let mut c2 = SrpClient::with_secret(identity, &salt, &login, &f("client_secret"));
            assert_eq!(c2.compute_m1(&stripped).unwrap(), f("m1"));
        }
    }

    #[test]
    fn random_roundtrip() {
        let salt = crypto::random_bytes(16);
        let login = crypto::random_bytes(16);
        let id = b"0f9d7a52-3f11-4a3c-9a8e-1b2c3d4e5f60";
        let v = verifier(id, &salt, &login);
        let server = Server::new(&v, &crypto::random_bytes(32));
        let mut client = SrpClient::new(id, &salt, &login);
        let a = client.public_a();
        assert_eq!(a.len(), N_LEN);
        let m1 = client.compute_m1(&server.b_pub).unwrap();
        let m2 = server.check(&a, &m1).expect("server accepts M1");
        client.verify_m2(&m2).unwrap();

        // Wrong password: server rejects.
        let mut wrong = SrpClient::new(id, &salt, &crypto::random_bytes(16));
        let m1 = wrong.compute_m1(&server.b_pub).unwrap();
        assert!(server.check(&wrong.public_a(), &m1).is_none());
    }

    #[test]
    fn rejects_bad_b_and_early_m2() {
        let mut c = SrpClient::new(b"id", b"salt", b"pw");
        assert!(c.verify_m2(&[0; 32]).is_err(), "M2 before M1");
        assert!(c.compute_m1(&[]).is_err());
        assert!(c.compute_m1(&[0; N_LEN]).is_err());
        assert!(c.compute_m1(&pad(n())).is_err());
        assert!(c.compute_m1(&[1; N_LEN + 1]).is_err());
        assert!(c.compute_m1(&pad(&(n() + BigUint::from(1u32)))).is_err());
        // A valid B still works afterwards.
        assert!(c.compute_m1(&pad(&BigUint::from(2u32))).is_ok());
    }
}
