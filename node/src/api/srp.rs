//! SRP-6a client, byte-for-byte as Ente's apps (and museum's `go-srp`).
//!
//! Group: RFC 5054 4096-bit, `g = 5`, SHA-256. Ente's conventions:
//!
//! - identity `I` = the `srpUserID` UUID string, password `P` = the 16-byte
//!   login sub-key ([`crate::crypto::derive_login_key`]).
//! - `x = H(salt | H(I ":" P))`, `k = H(pad(N) | pad(g))`, `u = H(pad(A) | pad(B))`.
//! - `S = (B - k·g^x)^(a + u·x) mod N`.
//! - `M1 = H(pad(A) | pad(B) | pad(S))`, `K = H(pad(S))`, `M2 = H(pad(A) | M1 | K)`.
//!
//! `A` is sent padded to 512 bytes. Big-integer maths is pure Rust (`num-bigint-dig`).

use crate::crypto;
use num_bigint_dig::BigUint;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use zeroize::Zeroizing;

/// Byte length of group elements (4096 bits).
pub const N_LEN: usize = 512;

/// RFC 5054 4096-bit group prime.
pub const N_HEX: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A92108011A723C12A787E6D788719A10BDBA5B2699C327186AF4E23C1A946834B6150BDA2583E9CA2AD44CE8DBBBC2DB04DE8EF92E8EFC141FBECAA6287C59474E6BC05D99B2964FA090C3A2233BA186515BE7ED1F612970CEE2D7AFB81BDD762170481CD0069127D5B05AA993B4EA988D8FDDC186FFB7DC90A6C08F4DF435C934063199FFFFFFFFFFFFFFFF";

/// The group prime `N`.
pub fn n() -> &'static BigUint {
    static N: OnceLock<BigUint> = OnceLock::new();
    N.get_or_init(|| BigUint::parse_bytes(N_HEX.as_bytes(), 16).unwrap_or_default())
}

/// The generator `g = 5`.
pub fn g() -> BigUint {
    BigUint::from(5u32)
}

/// Big-endian bytes of `x`, left-padded with zeros to [`N_LEN`].
pub fn pad(x: &BigUint) -> Vec<u8> {
    let b = x.to_bytes_be();
    if b.len() >= N_LEN {
        return b;
    }
    let mut out = vec![0u8; N_LEN - b.len()];
    out.extend_from_slice(&b);
    out
}

/// SHA-256 of the concatenation of `parts`.
pub fn h(parts: &[&[u8]]) -> Vec<u8> {
    let mut d = Sha256::new();
    for p in parts {
        d.update(p);
    }
    d.finalize().to_vec()
}

fn int(b: &[u8]) -> BigUint {
    BigUint::from_bytes_be(b)
}

/// The multiplier `k = H(pad(N) | pad(g))`.
pub fn k() -> &'static BigUint {
    static K: OnceLock<BigUint> = OnceLock::new();
    K.get_or_init(|| int(&h(&[&pad(n()), &pad(&g())])))
}

/// `x = H(salt | H(identity ":" password))`.
fn x(salt: &[u8], identity: &[u8], password: &[u8]) -> BigUint {
    let ip = h(&[identity, b":", password]);
    int(&h(&[salt, &ip]))
}

/// The verifier `v = g^x mod N` (padded), as Ente registers it at sign-up.
pub fn verifier(salt: &[u8], identity: &[u8], password: &[u8]) -> Vec<u8> {
    pad(&g().modpow(&x(salt, identity, password), n()))
}

/// Errors from the SRP exchange.
#[derive(Debug, thiserror::Error)]
pub enum SrpError {
    /// The server's `B` was invalid (zero mod N or too long).
    #[error("invalid server public value B")]
    InvalidB,
    /// The server's `M2` did not match: the server does not know the verifier.
    #[error("server proof M2 did not verify")]
    BadServerProof,
}

/// One client-side SRP session.
pub struct SrpClient {
    identity: Vec<u8>,
    salt: Vec<u8>,
    password: Zeroizing<Vec<u8>>,
    a: BigUint,
    a_pub: Vec<u8>,
    m1: Option<Vec<u8>>,
    key: Option<Zeroizing<Vec<u8>>>,
}

impl SrpClient {
    /// A new session with a random 64-byte secret `a`.
    pub fn new(identity: &str, salt: &[u8], password: &[u8]) -> Self {
        let secret = Zeroizing::new(crypto::random_bytes(64));
        Self::with_secret(identity, salt, password, &secret)
    }

    /// A session with a fixed secret `a` (for known-answer tests).
    pub fn with_secret(identity: &str, salt: &[u8], password: &[u8], secret: &[u8]) -> Self {
        let a = int(secret);
        let a_pub = pad(&g().modpow(&a, n()));
        Self {
            identity: identity.as_bytes().to_vec(),
            salt: salt.to_vec(),
            password: Zeroizing::new(password.to_vec()),
            a,
            a_pub,
            m1: None,
            key: None,
        }
    }

    /// `A = g^a mod N`, padded to 512 bytes (Ente's wire format).
    pub fn public_a(&self) -> &[u8] {
        &self.a_pub
    }

    /// Process the server's `B` and return the client proof `M1`.
    pub fn compute_m1(&mut self, b: &[u8]) -> Result<Vec<u8>, SrpError> {
        if b.len() > N_LEN {
            return Err(SrpError::InvalidB);
        }
        let b_int = int(b) % n();
        if b_int == BigUint::from(0u32) {
            return Err(SrpError::InvalidB);
        }
        let b_pad = pad(&b_int);
        let u = int(&h(&[&self.a_pub, &b_pad]));
        let x = x(&self.salt, &self.identity, &self.password);
        let kgx = (k() * g().modpow(&x, n())) % n();
        let base = (b_int + n() - kgx) % n();
        let s = Zeroizing::new(pad(&base.modpow(&(&self.a + u * &x), n())));
        let key = Zeroizing::new(h(&[&s]));
        let m1 = h(&[&self.a_pub, &b_pad, &s]);
        self.m1 = Some(m1.clone());
        self.key = Some(key);
        Ok(m1)
    }

    /// Check the server's proof `M2 = H(pad(A) | M1 | K)`.
    pub fn verify_m2(&self, m2: &[u8]) -> Result<(), SrpError> {
        let (Some(m1), Some(key)) = (&self.m1, &self.key) else {
            return Err(SrpError::BadServerProof);
        };
        let expected = h(&[&self.a_pub, m1, key]);
        if crypto::ct_eq(&expected, m2) {
            Ok(())
        } else {
            Err(SrpError::BadServerProof)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// go-srp's `TestCreateVerifier` vector (museum's SRP library).
    #[test]
    fn go_srp_verifier_vector() {
        let expected = "F0E47F50F5DEAD8DB8D93A279E3B62D6FF50854B31FBD3474A886BEF916261717E84DD4FB8B4D27FEAA5146DB7B1CBBC274FDF96A132B5029C2CD72527427A9B9809D5A4D018252928B4FC343BC17CE63C1859D5806F5466014FC361002D8890AEB4D6316FF37331FC2761BE0144C91CDD8E00ED0138C0CE51534D1B9A9BA629D7BE34D2742DD4097DAABC9ECB7AAAD89E53C342B038F1D2ADAE1F2410B7884A3E9A124C357E421BCCD4524467E1922660E0A4460C5F7C38C0877B65F6E32F28296282A93FC11BBABB7BB69BF1B3F9391991D8A86DD05E15000B7E38BA38A536BB0BF59C808EC25E791B8944719488B8087DF8BFD7FF20822997A53F6C86F3D45D004476D6303301376BB25A9F94B552CCE5ED40DE5DD7DA8027D754FA5F66738C7E3FC4EF3E20D625DF62CBE6E7ADFC21E47880D8A6ADA37E60370FD4D8FC82672A90C29F2E72F35652649D68348DE6F36D0E435C8BD42DD00155D35D501BECC0661B43E04CDB2DA84CE92B8BF49935D73D75EFCBD1176D7BBCCC3CC4D4B5FEFCC02D478614EE1681D2FF3C711A61A7686EB852AE06FB8227BE21FB8802719B1271BA1C02B13BBF0A2C2E459D9BEDCC8D1269F6A785CB4563AA791B38FB038269F63F58F47E9051499549789269CC7B8EC7026FC34BA73289C4AF829D5A532E723967CE9B6C023EF0FD0CFE37F51F10F19463B6534159A09DDD2F51F3B30033";
        assert_eq!(
            hex::encode_upper(verifier(b"salty", b"alice", b"password123")),
            expected
        );
    }

    #[test]
    fn rejects_zero_b() {
        let mut c = SrpClient::new("id", b"salt", b"pw");
        assert!(c.compute_m1(&[0u8; 512]).is_err());
        assert!(c.compute_m1(&pad(n())).is_err());
        assert!(c.compute_m1(&[1u8; 513]).is_err());
        assert!(c.verify_m2(&[0u8; 32]).is_err());
    }
}
