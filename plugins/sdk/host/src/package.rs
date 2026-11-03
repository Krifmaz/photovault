//! Signed plugin packages (`.pvplugin`).
//!
//! # Format
//!
//! ```text
//! "PVPLUG01"                     8-byte magic
//! u32 LE length, manifest bytes  plugin.toml, UTF-8, at most MAX_MANIFEST_BYTES
//! u32 LE length, wasm bytes      the module, at most MAX_WASM_BYTES
//! u32 LE length (= 32), key      Ed25519 public key of the publisher
//! u32 LE length (= 64), sig      Ed25519 signature
//! ```
//!
//! Nothing may follow the signature. The signature is libsodium's
//! `crypto_sign_detached` over
//!
//! ```text
//! b"photovault-plugin-v1\0" || BLAKE2b-256(manifest) || BLAKE2b-256(wasm)
//! ```
//!
//! so it covers both the exact manifest bytes and the exact module. Parsing
//! is strict: wrong magic, a short or oversized section, trailing bytes, an
//! invalid manifest, a non-Wasm module or a bad signature all fail.
//!
//! Signing keys are trusted on first use: the [`crate::registry`] pins the
//! first key it sees for a plugin id and refuses updates signed by another.

use crate::crypto;
use photovault_plugin_manifest::{Manifest, ManifestError};
use std::fmt;

/// File magic, including the format version.
pub const MAGIC: &[u8; 8] = b"PVPLUG01";
/// Domain separation prefix of the signed message.
pub const SIGNING_CONTEXT: &[u8] = b"photovault-plugin-v1\0";
/// Largest accepted `plugin.toml`.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Largest accepted Wasm module.
pub const MAX_WASM_BYTES: usize = 64 * 1024 * 1024;
/// Largest accepted package (all sections plus framing).
pub const MAX_PACKAGE_BYTES: usize =
    MAGIC.len() + 4 * 4 + MAX_MANIFEST_BYTES + MAX_WASM_BYTES + 32 + 64;

const WASM_MAGIC: &[u8; 4] = b"\0asm";
const SECRET_KEY_HEADER: &str = "photovault-plugin-secret-key-v1";

/// Why a package or key was rejected.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    /// The input does not start with [`MAGIC`].
    #[error("not a Photovault plugin package (bad magic)")]
    BadMagic,
    /// The input ends inside a section.
    #[error("package is truncated")]
    Truncated,
    /// A section, or the whole input, is larger than allowed.
    #[error("package {0} is too large")]
    TooLarge(&'static str),
    /// A fixed-size section has the wrong length.
    #[error("package {0} has the wrong length")]
    BadLength(&'static str),
    /// Bytes follow the signature.
    #[error("package has trailing bytes")]
    TrailingBytes,
    /// The manifest is not UTF-8.
    #[error("plugin.toml is not UTF-8")]
    ManifestNotUtf8,
    /// The manifest does not parse or validate.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// The module does not start with the Wasm magic.
    #[error("module is not WebAssembly")]
    NotWasm,
    /// The signature does not verify against the embedded key.
    #[error("signature does not verify")]
    BadSignature,
    /// A secret or public key file or string is malformed.
    #[error("invalid key: {0}")]
    BadKey(&'static str),
}

/// An Ed25519 public key identifying a plugin publisher.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PublicKey(pub [u8; crypto::PUBLIC_KEY_BYTES]);

impl PublicKey {
    /// Parse 64 hex characters.
    pub fn from_hex(s: &str) -> Result<Self, PackageError> {
        let b = crypto::from_hex(s.trim()).ok_or(PackageError::BadKey("not hex"))?;
        let arr: [u8; 32] = b
            .try_into()
            .map_err(|_| PackageError::BadKey("public key must be 32 bytes"))?;
        Ok(Self(arr))
    }

    /// 64 lower-case hex characters.
    pub fn to_hex(&self) -> String {
        crypto::to_hex(&self.0)
    }

    /// A short fingerprint for people to compare, e.g.
    /// `3f2a-9c01-77de-0b4e-5a6f-12c3-d8e9-4b70`: the first 16 bytes of
    /// BLAKE2b-256 of the key, as eight groups of four hex digits.
    pub fn fingerprint(&self) -> String {
        let h = crypto::blake2b_256(&self.0);
        let hex = crypto::to_hex(&h[..16]);
        hex.as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).expect("hex is ASCII"))
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Verify a detached signature of `msg`.
    pub fn verify(&self, msg: &[u8], sig: &[u8; crypto::SIGNATURE_BYTES]) -> bool {
        crypto::verify_detached(sig, msg, &self.0)
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.to_hex())
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl serde::Serialize for PublicKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> serde::Deserialize<'de> for PublicKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        PublicKey::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

/// An Ed25519 signing key. The secret bytes are zeroed on drop.
pub struct SigningKey {
    secret: [u8; crypto::SECRET_KEY_BYTES],
    public: PublicKey,
}

impl SigningKey {
    /// A fresh random key.
    pub fn generate() -> Self {
        let (pk, sk) = crypto::sign_keypair();
        Self {
            secret: sk,
            public: PublicKey(pk),
        }
    }

    /// The key derived from a 32-byte seed (RFC 8032 "secret key").
    pub fn from_seed(seed: &[u8; crypto::SEED_BYTES]) -> Self {
        let (pk, sk) = crypto::sign_seed_keypair(seed);
        Self {
            secret: sk,
            public: PublicKey(pk),
        }
    }

    /// The public half.
    pub fn public_key(&self) -> PublicKey {
        self.public
    }

    /// A detached Ed25519 signature of `msg`.
    pub fn sign(&self, msg: &[u8]) -> [u8; crypto::SIGNATURE_BYTES] {
        crypto::sign_detached(msg, &self.secret)
    }

    /// The text stored in a secret key file:
    /// `photovault-plugin-secret-key-v1\n<128 hex chars>\n`.
    pub fn to_file_contents(&self) -> String {
        format!("{SECRET_KEY_HEADER}\n{}\n", crypto::to_hex(&self.secret))
    }

    /// Parse [`SigningKey::to_file_contents`] output. The embedded public key
    /// must match the secret half.
    pub fn from_file_contents(text: &str) -> Result<Self, PackageError> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(SECRET_KEY_HEADER) {
            return Err(PackageError::BadKey("not a Photovault plugin secret key"));
        }
        let mut bytes = lines
            .next()
            .and_then(|l| crypto::from_hex(l.trim()))
            .ok_or(PackageError::BadKey("secret key is not hex"))?;
        if lines.any(|l| !l.trim().is_empty()) {
            crypto::memzero(&mut bytes);
            return Err(PackageError::BadKey("unexpected data after the key"));
        }
        if bytes.len() != crypto::SECRET_KEY_BYTES {
            crypto::memzero(&mut bytes);
            return Err(PackageError::BadKey("secret key must be 64 bytes"));
        }
        let mut secret = [0u8; crypto::SECRET_KEY_BYTES];
        secret.copy_from_slice(&bytes);
        crypto::memzero(&mut bytes);
        // Re-derive the key pair from the seed half: the stored public half
        // must match, which catches corrupt or truncated key files.
        let mut seed = [0u8; crypto::SEED_BYTES];
        seed.copy_from_slice(&secret[..32]);
        let key = Self::from_seed(&seed);
        crypto::memzero(&mut seed);
        let matches = key.secret == secret;
        crypto::memzero(&mut secret);
        if !matches {
            return Err(PackageError::BadKey("secret key is corrupt"));
        }
        Ok(key)
    }
}

impl Drop for SigningKey {
    fn drop(&mut self) {
        crypto::memzero(&mut self.secret);
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningKey")
            .field("public", &self.public)
            .finish_non_exhaustive()
    }
}

/// The message a package signature covers.
pub fn signed_message(manifest: &[u8], wasm: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(SIGNING_CONTEXT.len() + 64);
    m.extend_from_slice(SIGNING_CONTEXT);
    m.extend_from_slice(&crypto::blake2b_256(manifest));
    m.extend_from_slice(&crypto::blake2b_256(wasm));
    m
}

/// A parsed package whose manifest is valid and whose signature verified.
///
/// The only way to get one is [`Package::parse`] (or [`Package::build`]
/// followed by parsing), so holding a `Package` means it was verified.
#[derive(Clone)]
pub struct Package {
    manifest: Manifest,
    manifest_bytes: Vec<u8>,
    wasm: Vec<u8>,
    publisher: PublicKey,
    signature: [u8; crypto::SIGNATURE_BYTES],
}

impl fmt::Debug for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Package")
            .field("id", &self.manifest.plugin.id)
            .field("version", &self.manifest.plugin.version)
            .field("publisher", &self.publisher)
            .field("wasm_bytes", &self.wasm.len())
            .finish_non_exhaustive()
    }
}

struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], PackageError> {
        if self.rest.len() < n {
            return Err(PackageError::Truncated);
        }
        let (a, b) = self.rest.split_at(n);
        self.rest = b;
        Ok(a)
    }

    fn section(&mut self, name: &'static str, max: usize) -> Result<&'a [u8], PackageError> {
        let len = u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")) as usize;
        if len > max {
            return Err(PackageError::TooLarge(name));
        }
        self.take(len)
    }

    fn fixed<const N: usize>(&mut self, name: &'static str) -> Result<[u8; N], PackageError> {
        let len = u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")) as usize;
        if len != N {
            return Err(PackageError::BadLength(name));
        }
        Ok(self.take(N)?.try_into().expect("N bytes"))
    }
}

impl Package {
    /// Sign `manifest` (the exact `plugin.toml` bytes) and `wasm` with `key`
    /// and return the package bytes. The manifest is validated and the module
    /// checked for the Wasm magic first.
    pub fn build(manifest: &[u8], wasm: &[u8], key: &SigningKey) -> Result<Vec<u8>, PackageError> {
        if manifest.len() > MAX_MANIFEST_BYTES {
            return Err(PackageError::TooLarge("manifest"));
        }
        if wasm.len() > MAX_WASM_BYTES {
            return Err(PackageError::TooLarge("module"));
        }
        let text = std::str::from_utf8(manifest).map_err(|_| PackageError::ManifestNotUtf8)?;
        Manifest::from_toml(text)?;
        if !wasm.starts_with(WASM_MAGIC) {
            return Err(PackageError::NotWasm);
        }
        let sig = key.sign(&signed_message(manifest, wasm));
        let pk = key.public_key();
        let mut out = Vec::with_capacity(MAGIC.len() + 16 + manifest.len() + wasm.len() + 96);
        out.extend_from_slice(MAGIC);
        for section in [manifest, wasm, &pk.0[..], &sig[..]] {
            out.extend_from_slice(&(section.len() as u32).to_le_bytes());
            out.extend_from_slice(section);
        }
        Ok(out)
    }

    /// Parse, validate and verify package bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, PackageError> {
        if bytes.len() > MAX_PACKAGE_BYTES {
            return Err(PackageError::TooLarge("file"));
        }
        if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
            return Err(PackageError::BadMagic);
        }
        let mut r = Reader {
            rest: &bytes[MAGIC.len()..],
        };
        let manifest_bytes = r.section("manifest", MAX_MANIFEST_BYTES)?;
        let wasm = r.section("module", MAX_WASM_BYTES)?;
        let pk: [u8; 32] = r.fixed("public key")?;
        let sig: [u8; 64] = r.fixed("signature")?;
        if !r.rest.is_empty() {
            return Err(PackageError::TrailingBytes);
        }
        // Verify before parsing anything attacker-controlled further.
        let publisher = PublicKey(pk);
        if !publisher.verify(&signed_message(manifest_bytes, wasm), &sig) {
            return Err(PackageError::BadSignature);
        }
        let text =
            std::str::from_utf8(manifest_bytes).map_err(|_| PackageError::ManifestNotUtf8)?;
        let manifest = Manifest::from_toml(text)?;
        if !wasm.starts_with(WASM_MAGIC) {
            return Err(PackageError::NotWasm);
        }
        Ok(Self {
            manifest,
            manifest_bytes: manifest_bytes.to_vec(),
            wasm: wasm.to_vec(),
            publisher,
            signature: sig,
        })
    }

    /// The validated manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The exact signed `plugin.toml` bytes.
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest_bytes
    }

    /// The Wasm module.
    pub fn wasm(&self) -> &[u8] {
        &self.wasm
    }

    /// The publisher key the package is signed with.
    pub fn publisher(&self) -> PublicKey {
        self.publisher
    }

    /// The signature.
    pub fn signature(&self) -> &[u8; crypto::SIGNATURE_BYTES] {
        &self.signature
    }

    /// Re-encode to package bytes (identical to the parsed input).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        for section in [
            &self.manifest_bytes[..],
            &self.wasm[..],
            &self.publisher.0[..],
            &self.signature[..],
        ] {
            out.extend_from_slice(&(section.len() as u32).to_le_bytes());
            out.extend_from_slice(section);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
[plugin]
id = "dev.photovault.test"
name = "Test"
version = "1.0.0"
api_version = 1
"#;
    const WASM: &[u8] = b"\0asm\x01\0\0\0";

    fn pkg() -> (SigningKey, Vec<u8>) {
        let k = SigningKey::generate();
        let b = Package::build(MANIFEST.as_bytes(), WASM, &k).unwrap();
        (k, b)
    }

    /// Byte offsets of the sections in a package built from MANIFEST/WASM.
    fn offsets() -> (usize, usize, usize, usize) {
        let m = 8 + 4;
        let w = m + MANIFEST.len() + 4;
        let k = w + WASM.len() + 4;
        let s = k + 32 + 4;
        (m, w, k, s)
    }

    #[test]
    fn round_trip() {
        let (k, b) = pkg();
        let p = Package::parse(&b).unwrap();
        assert_eq!(p.publisher(), k.public_key());
        assert_eq!(p.manifest().plugin.id, "dev.photovault.test");
        assert_eq!(p.wasm(), WASM);
        assert_eq!(p.manifest_bytes(), MANIFEST.as_bytes());
        assert_eq!(p.to_bytes(), b);
    }

    #[test]
    fn rfc8032_test_1() {
        let seed =
            crypto::from_hex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
                .unwrap();
        let k = SigningKey::from_seed(&seed.try_into().unwrap());
        assert_eq!(
            k.public_key().to_hex(),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        let sig = k.sign(b"");
        assert_eq!(
            crypto::to_hex(&sig),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
        assert!(k.public_key().verify(b"", &sig));
        assert!(!k.public_key().verify(b"x", &sig));
    }

    #[test]
    fn rejects_any_single_bit_flip() {
        let (_, b) = pkg();
        let (m, ..) = offsets();
        // Flip every bit after the magic: each must be rejected.
        for i in m - 4..b.len() {
            for bit in 0..8 {
                let mut t = b.clone();
                t[i] ^= 1 << bit;
                assert!(Package::parse(&t).is_err(), "byte {i} bit {bit} accepted");
            }
        }
    }

    #[test]
    fn rejects_tampering_kinds() {
        let (_, b) = pkg();
        let (m, w, k, s) = offsets();
        // Changed manifest byte (same length): bad signature.
        let mut t = b.clone();
        t[m + 3] = b'#';
        assert!(matches!(
            Package::parse(&t),
            Err(PackageError::BadSignature)
        ));
        // Changed wasm.
        let mut t = b.clone();
        t[w + 5] ^= 1;
        assert!(matches!(
            Package::parse(&t),
            Err(PackageError::BadSignature)
        ));
        // Different key (re-signed by someone else but key swapped back).
        let other = SigningKey::generate();
        let mut t = b.clone();
        t[k..k + 32].copy_from_slice(&other.public_key().0);
        assert!(matches!(
            Package::parse(&t),
            Err(PackageError::BadSignature)
        ));
        // Corrupt signature.
        let mut t = b.clone();
        t[s + 10] ^= 0x80;
        assert!(matches!(
            Package::parse(&t),
            Err(PackageError::BadSignature)
        ));
        // Trailing data.
        let mut t = b.clone();
        t.push(0);
        assert!(matches!(
            Package::parse(&t),
            Err(PackageError::TrailingBytes)
        ));
        // Bad magic.
        let mut t = b.clone();
        t[7] = b'2';
        assert!(matches!(Package::parse(&t), Err(PackageError::BadMagic)));
        // Wrong fixed-length section.
        let mut t = b.clone();
        t[k - 4] = 31;
        assert!(Package::parse(&t).is_err());
    }

    #[test]
    fn truncated_and_oversized() {
        let (_, b) = pkg();
        for n in 0..b.len() {
            assert!(Package::parse(&b[..n]).is_err(), "prefix {n} accepted");
        }
        // A section length larger than the cap is rejected before reading.
        let mut t = b.clone();
        t[8..12].copy_from_slice(&((MAX_MANIFEST_BYTES as u32) + 1).to_le_bytes());
        assert!(matches!(
            Package::parse(&t),
            Err(PackageError::TooLarge("manifest"))
        ));
        let mut t = b.clone();
        t[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Package::parse(&t).is_err());
        let huge = vec![0u8; MAX_PACKAGE_BYTES + 1];
        assert!(matches!(
            Package::parse(&huge),
            Err(PackageError::TooLarge("file"))
        ));
    }

    #[test]
    fn build_validates() {
        let k = SigningKey::generate();
        assert!(matches!(
            Package::build(MANIFEST.as_bytes(), b"notwasm", &k),
            Err(PackageError::NotWasm)
        ));
        assert!(matches!(
            Package::build(b"[plugin]\nid='x'", WASM, &k),
            Err(PackageError::Manifest(_))
        ));
        assert!(matches!(
            Package::build(&[0xff, 0xfe], WASM, &k),
            Err(PackageError::ManifestNotUtf8)
        ));
    }

    #[test]
    fn signed_invalid_manifest_rejected() {
        // A correctly signed package whose manifest is invalid still fails.
        let k = SigningKey::generate();
        let bad = MANIFEST.replace("api_version = 1", "api_version = 9");
        let sig = k.sign(&signed_message(bad.as_bytes(), WASM));
        let mut out = MAGIC.to_vec();
        for s in [bad.as_bytes(), WASM, &k.public_key().0[..], &sig[..]] {
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s);
        }
        assert!(matches!(
            Package::parse(&out),
            Err(PackageError::Manifest(_))
        ));
    }

    #[test]
    fn key_files() {
        let k = SigningKey::generate();
        let text = k.to_file_contents();
        let k2 = SigningKey::from_file_contents(&text).unwrap();
        assert_eq!(k.public_key(), k2.public_key());
        assert!(SigningKey::from_file_contents("nope\n00").is_err());
        let mut corrupt = text.clone().into_bytes();
        let n = corrupt.len();
        corrupt[n - 2] = if corrupt[n - 2] == b'0' { b'1' } else { b'0' };
        assert!(SigningKey::from_file_contents(std::str::from_utf8(&corrupt).unwrap()).is_err());
        let pk = PublicKey::from_hex(&k.public_key().to_hex()).unwrap();
        assert_eq!(pk, k.public_key());
        let fp = pk.fingerprint();
        assert_eq!(fp.len(), 39);
        assert_eq!(fp.split('-').count(), 8);
    }
}
