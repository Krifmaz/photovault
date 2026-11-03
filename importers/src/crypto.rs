//! Safe wrappers over libsodium, matching exactly what the Ente apps do.
//!
//! Reference: `web/packages/base/crypto/libsodium.ts` and the Go CLI
//! `cli/internal/crypto` in `ente-io/ente`.
//!
//! STUB: signatures are fixed; bodies are filled in by the crypto work item.

/// Errors from crypto helpers.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Wrong key/nonce length, bad base64, or authentication failure.
    #[error("crypto: {0}")]
    Invalid(&'static str),
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, CryptoError>;

/// Size of one plaintext chunk in Ente's file encryption (4 MiB).
pub const ENCRYPTION_CHUNK_SIZE: usize = 4 * 1024 * 1024;
/// `crypto_secretstream_xchacha20poly1305_ABYTES` (17).
pub const STREAM_ABYTES: usize = 17;
/// `crypto_secretstream_xchacha20poly1305_HEADERBYTES` (24).
pub const STREAM_HEADER_BYTES: usize = 24;

/// Initialise libsodium. Safe to call many times.
pub fn init() {
    todo!()
}

/// `n` cryptographically secure random bytes.
pub fn random_bytes(n: usize) -> Vec<u8> {
    todo!("{n}")
}

/// A fresh 32-byte key (`crypto_secretbox_keygen` / `crypto_secretstream_..._keygen`).
pub fn generate_key() -> Vec<u8> {
    todo!()
}

/// Standard base64 with padding (what Ente sends on the wire).
pub fn b64(data: &[u8]) -> String {
    todo!("{data:?}")
}

/// Decode standard base64 (also accepts URL-safe input, like the Ente apps).
pub fn unb64(s: &str) -> Result<Vec<u8>> {
    todo!("{s}")
}

/// Argon2id key derivation (`crypto_pwhash` with `ALG_ARGON2ID13`), 32-byte
/// output, as Ente derives the key-encryption key from the password.
pub fn derive_argon_key(password: &[u8], salt: &[u8], ops_limit: u64, mem_limit: u64) -> Result<Vec<u8>> {
    todo!("{password:?}{salt:?}{ops_limit}{mem_limit}")
}

/// Ente's login sub-key for SRP: `crypto_kdf_derive_from_key(32, 1, "loginctx", kek)`
/// truncated to the first 16 bytes.
pub fn derive_login_key(kek: &[u8]) -> Result<Vec<u8>> {
    todo!("{kek:?}")
}

/// `crypto_secretbox_easy` with a fresh random nonce. Returns `(ciphertext, nonce)`.
pub fn secretbox_seal(msg: &[u8], key: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    todo!("{msg:?}{key:?}")
}

/// `crypto_secretbox_open_easy`.
pub fn secretbox_open(ct: &[u8], nonce: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    todo!("{ct:?}{nonce:?}{key:?}")
}

/// `crypto_box_seal_open` (used to decrypt the auth token).
pub fn box_seal_open(ct: &[u8], public_key: &[u8], secret_key: &[u8]) -> Result<Vec<u8>> {
    todo!("{ct:?}{public_key:?}{secret_key:?}")
}

/// `crypto_box_seal` (used when sharing keys; kept for tests and symmetry).
pub fn box_seal(msg: &[u8], public_key: &[u8]) -> Result<Vec<u8>> {
    todo!("{msg:?}{public_key:?}")
}

/// `crypto_box_keypair`. Returns `(public_key, secret_key)`.
pub fn box_keypair() -> (Vec<u8>, Vec<u8>) {
    todo!()
}

/// Encrypt a small blob as a single secretstream message tagged FINAL, as
/// Ente does for metadata, magic metadata and thumbnails.
/// Returns `(ciphertext, header)`.
pub fn encrypt_blob(data: &[u8], key: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    todo!("{data:?}{key:?}")
}

/// Inverse of [`encrypt_blob`].
pub fn decrypt_blob(ct: &[u8], header: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    todo!("{ct:?}{header:?}{key:?}")
}

/// Streaming secretstream encryptor for file contents: plaintext in
/// [`ENCRYPTION_CHUNK_SIZE`] chunks, every chunk tagged MESSAGE except the
/// last, which is tagged FINAL (exactly like the Ente apps).
pub struct StreamEncryptor {
    _private: (),
}

impl StreamEncryptor {
    /// Start a stream with `key` (32 bytes). Returns the encryptor and the
    /// 24-byte header the server stores as `decryptionHeader`.
    pub fn new(key: &[u8]) -> Result<(Self, Vec<u8>)> {
        todo!("{key:?}")
    }

    /// Encrypt one chunk. `last` must be true for the final chunk only.
    pub fn push(&mut self, chunk: &[u8], last: bool) -> Result<Vec<u8>> {
        todo!("{chunk:?}{last}")
    }
}

/// Streaming secretstream decryptor (used by tests and dedup checks).
pub struct StreamDecryptor {
    _private: (),
}

impl StreamDecryptor {
    /// Start decrypting a stream with `key` and `header`.
    pub fn new(key: &[u8], header: &[u8]) -> Result<Self> {
        todo!("{key:?}{header:?}")
    }

    /// Decrypt one ciphertext chunk (`ENCRYPTION_CHUNK_SIZE + STREAM_ABYTES`
    /// bytes, or less for the last). Returns `(plaintext, is_final)`.
    pub fn pull(&mut self, chunk: &[u8]) -> Result<(Vec<u8>, bool)> {
        todo!("{chunk:?}")
    }
}

/// Decrypt a whole secretstream ciphertext produced with Ente's chunking.
pub fn decrypt_stream(ct: &[u8], header: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    todo!("{ct:?}{header:?}{key:?}")
}

/// Streaming BLAKE2b (`crypto_generichash`, 64-byte output, no key), as Ente
/// computes the content hash stored in file metadata.
pub struct Hasher {
    _private: (),
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher {
    /// New hasher.
    pub fn new() -> Self {
        todo!()
    }
    /// Feed bytes.
    pub fn update(&mut self, data: &[u8]) {
        todo!("{data:?}")
    }
    /// Finish: 64-byte digest.
    pub fn finalize(self) -> Vec<u8> {
        todo!()
    }
}
