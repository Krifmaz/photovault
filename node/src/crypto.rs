//! Ente's encryption scheme, via libsodium (`libsodium-sys-stable`).
//!
//! Everything here is wire-compatible with the official Ente apps, which use
//! libsodium (web: `libsodium-wrappers`, mobile: `flutter_sodium`, Ente's own
//! Rust core: pure-Rust ports checked against libsodium):
//!
//! - **Blobs** ([`encrypt_blob`]/[`decrypt_blob`]): one
//!   `crypto_secretstream_xchacha20poly1305` message tagged `FINAL`. Used for
//!   file metadata, magic metadata, ML data and video playlists.
//! - **Files** ([`encrypt_file`]/[`decrypt_file`]/[`ChunkedDecryptor`]): the
//!   same secretstream, cut into 4 MiB plaintext chunks, the last tagged
//!   `FINAL`. Used for originals, thumbnails and video segments.
//! - **Secretbox** ([`secretbox_seal`]/[`secretbox_open`]): XSalsa20-Poly1305
//!   with a separate 24-byte nonce. Wraps master key, collection and file keys.
//! - **Sealed box** ([`sealed_box_seal`]/[`sealed_box_open`]): anonymous
//!   X25519 box. Wraps shared collection keys and the login token.
//! - **Argon2id** ([`derive_key`]) and the SRP login sub-key
//!   ([`derive_login_key`], `crypto_kdf`).
//!
//! `unsafe` FFI is confined to this module; every call documents why it is sound.
//!
//! OWNER: crypto/api agent. Signatures here are the contract other modules
//! compile against; keep them stable.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use libsodium_sys as ffi;
use std::sync::OnceLock;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// Errors from cryptographic operations.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// libsodium failed to initialise.
    #[error("libsodium initialisation failed")]
    Init,
    /// Authentication failed or the input was malformed.
    #[error("decryption failed")]
    Decrypt,
    /// Encryption failed.
    #[error("encryption failed")]
    Encrypt,
    /// A key, nonce or header had the wrong length.
    #[error("invalid length: {0}")]
    Length(&'static str),
    /// A chunked stream ended without its `FINAL` chunk, or had data after it.
    #[error("encrypted stream is truncated or has trailing data")]
    Truncated,
    /// Key-derivation parameters were out of range (or derivation ran out of memory).
    #[error("invalid key derivation parameters: {0}")]
    KdfParams(String),
    /// Base64 input could not be decoded.
    #[error("invalid base64")]
    Base64,
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, CryptoError>;

/// Byte length of symmetric keys (secretbox, secretstream) and X25519 keys.
pub const KEY_BYTES: usize = 32;
/// Byte length of a secretbox nonce.
pub const NONCE_BYTES: usize = 24;
/// Byte length of a secretstream header.
pub const HEADER_BYTES: usize = 24;
/// Byte length of an Argon2id salt.
pub const SALT_BYTES: usize = 16;
/// Secretstream per-message overhead (1 tag byte + 16 MAC bytes).
pub const STREAM_ABYTES: usize = 17;
/// Secretbox MAC length.
pub const SECRETBOX_MAC_BYTES: usize = 16;
/// Sealed-box overhead (ephemeral public key + MAC).
pub const SEAL_BYTES: usize = 48;
/// Plaintext bytes per chunk when Ente encrypts a file (4 MiB).
pub const ENCRYPTION_CHUNK_SIZE: usize = 4 * 1024 * 1024;
/// Ciphertext bytes per chunk of an Ente-encrypted file.
pub const DECRYPTION_CHUNK_SIZE: usize = ENCRYPTION_CHUNK_SIZE + STREAM_ABYTES;

const TAG_MESSAGE: u8 = 0;
const TAG_FINAL: u8 = 3;

/// A 32-byte symmetric key (or X25519 secret key) that is wiped from memory on drop.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey(pub [u8; 32]);

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(..)")
    }
}

impl SecretKey {
    /// A fresh random key.
    pub fn generate() -> Self {
        let mut k = [0u8; KEY_BYTES];
        fill_random(&mut k);
        Self(k)
    }

    /// Copy a key from a slice that must be exactly 32 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let arr: [u8; KEY_BYTES] = bytes.try_into().map_err(|_| CryptoError::Length("key"))?;
        Ok(Self(arr))
    }

    /// Decode a key from standard base64 (as the Ente APIs and apps store it).
    pub fn from_b64(s: &str) -> Result<Self> {
        let bytes = Zeroizing::new(b64_decode(s)?);
        Self::from_slice(&bytes)
    }

    /// The raw key bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Standard base64 of the key. The caller must treat the string as secret.
    pub fn to_b64(&self) -> String {
        b64_encode(&self.0)
    }
}

impl serde::Serialize for SecretKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let enc = Zeroizing::new(self.to_b64());
        s.serialize_str(&enc)
    }
}

impl<'de> serde::Deserialize<'de> for SecretKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = Zeroizing::new(String::deserialize(d)?);
        Self::from_b64(&s).map_err(serde::de::Error::custom)
    }
}

/// Output of [`encrypt_blob`] and [`encrypt_file`]: ciphertext plus secretstream header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedBlob {
    /// Ciphertext (single secretstream message tagged FINAL for blobs, or
    /// 4 MiB chunks for files).
    pub encrypted_data: Vec<u8>,
    /// The 24-byte secretstream header.
    pub decryption_header: Vec<u8>,
}

/// Initialise libsodium. Safe to call many times.
pub fn init() -> Result<()> {
    static RC: OnceLock<i32> = OnceLock::new();
    // SAFETY: sodium_init is thread-safe and idempotent; it takes no arguments.
    let rc = *RC.get_or_init(|| unsafe { ffi::sodium_init() });
    if rc < 0 {
        Err(CryptoError::Init)
    } else {
        Ok(())
    }
}

fn ensure_init() {
    // sodium_init only fails if the system RNG is unusable; nothing in this
    // module is safe to run then.
    if init().is_err() {
        panic!("libsodium failed to initialise (no usable system RNG)");
    }
}

/// Fill `buf` with cryptographically secure random bytes.
pub fn fill_random(buf: &mut [u8]) {
    ensure_init();
    // SAFETY: buf is valid for writes of buf.len() bytes.
    unsafe { ffi::randombytes_buf(buf.as_mut_ptr().cast(), buf.len()) };
}

/// `n` cryptographically secure random bytes.
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    fill_random(&mut v);
    v
}

/// Constant-time equality of two byte strings (false when lengths differ).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    ensure_init();
    // SAFETY: both pointers are valid for a.len() bytes (lengths checked equal).
    unsafe { ffi::sodium_memcmp(a.as_ptr().cast(), b.as_ptr().cast(), a.len()) == 0 }
}

// ---------------------------------------------------------------------------
// Base64
// ---------------------------------------------------------------------------

/// Standard padded base64 (libsodium `ORIGINAL`): keys, nonces, ciphertexts in Ente's JSON.
pub fn b64_encode(data: &[u8]) -> String {
    STANDARD.encode(data)
}

/// Decode standard padded base64.
pub fn b64_decode(s: &str) -> Result<Vec<u8>> {
    STANDARD.decode(s.trim()).map_err(|_| CryptoError::Base64)
}

/// URL-safe padded base64 (libsodium `URLSAFE`, Go `base64.URLEncoding`): the auth token.
pub fn b64url_encode(data: &[u8]) -> String {
    URL_SAFE.encode(data)
}

/// Decode URL-safe base64, with or without padding.
pub fn b64url_decode(s: &str) -> Result<Vec<u8>> {
    let s = s.trim();
    URL_SAFE
        .decode(s)
        .or_else(|_| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')))
        .map_err(|_| CryptoError::Base64)
}

// ---------------------------------------------------------------------------
// Secretstream (blobs and chunked files)
// ---------------------------------------------------------------------------

/// Push (encrypt) side of a libsodium secretstream.
pub struct StreamEncryptor {
    state: ffi::crypto_secretstream_xchacha20poly1305_state,
    header: [u8; HEADER_BYTES],
}

impl StreamEncryptor {
    /// Start a new stream with a random header (`init_push`).
    pub fn new(key: &SecretKey) -> Result<Self> {
        ensure_init();
        let mut state = empty_state();
        let mut header = [0u8; HEADER_BYTES];
        // SAFETY: state is a valid, writable state struct; header has
        // HEADERBYTES (24) bytes; key has KEYBYTES (32) bytes.
        let rc = unsafe {
            ffi::crypto_secretstream_xchacha20poly1305_init_push(
                &mut state,
                header.as_mut_ptr(),
                key.0.as_ptr(),
            )
        };
        if rc != 0 {
            return Err(CryptoError::Encrypt);
        }
        Ok(Self { state, header })
    }

    /// Start a stream with a caller-chosen header.
    ///
    /// libsodium derives the push state from `(key, header)` exactly as the
    /// pull side does, so this reproduces another implementation's ciphertext
    /// byte for byte. **Only for known-answer tests**: reusing a header with
    /// the same key for different plaintexts destroys confidentiality.
    pub fn with_header_for_test_vectors(key: &SecretKey, header: &[u8]) -> Result<Self> {
        ensure_init();
        let header: [u8; HEADER_BYTES] = header
            .try_into()
            .map_err(|_| CryptoError::Length("header"))?;
        let mut state = empty_state();
        // SAFETY: as in `new`; header is 24 readable bytes.
        let rc = unsafe {
            ffi::crypto_secretstream_xchacha20poly1305_init_pull(
                &mut state,
                header.as_ptr(),
                key.0.as_ptr(),
            )
        };
        if rc != 0 {
            return Err(CryptoError::Encrypt);
        }
        Ok(Self { state, header })
    }

    /// The 24-byte header the decryptor needs.
    pub fn header(&self) -> &[u8; HEADER_BYTES] {
        &self.header
    }

    /// Encrypt one message; `is_final` sets the `FINAL` tag.
    pub fn push(&mut self, data: &[u8], is_final: bool) -> Result<Vec<u8>> {
        let mut out = vec![0u8; data.len() + STREAM_ABYTES];
        let mut out_len: u64 = 0;
        let tag = if is_final { TAG_FINAL } else { TAG_MESSAGE };
        // SAFETY: out has room for data.len() + ABYTES bytes (the maximum
        // libsodium writes); data is readable for data.len(); ad is null with
        // length 0, which libsodium accepts.
        let rc = unsafe {
            ffi::crypto_secretstream_xchacha20poly1305_push(
                &mut self.state,
                out.as_mut_ptr(),
                &mut out_len,
                data.as_ptr(),
                data.len() as u64,
                std::ptr::null(),
                0,
                tag,
            )
        };
        if rc != 0 {
            return Err(CryptoError::Encrypt);
        }
        out.truncate(out_len as usize);
        Ok(out)
    }
}

impl Drop for StreamEncryptor {
    fn drop(&mut self) {
        wipe_state(&mut self.state);
    }
}

/// Pull (decrypt) side of a libsodium secretstream, one message at a time.
pub struct StreamDecryptor {
    state: ffi::crypto_secretstream_xchacha20poly1305_state,
    finished: bool,
}

impl StreamDecryptor {
    /// Start decrypting a stream (`init_pull`).
    pub fn new(header: &[u8], key: &SecretKey) -> Result<Self> {
        ensure_init();
        if header.len() != HEADER_BYTES {
            return Err(CryptoError::Length("header"));
        }
        let mut state = empty_state();
        // SAFETY: header is 24 readable bytes, key is 32, state is writable.
        let rc = unsafe {
            ffi::crypto_secretstream_xchacha20poly1305_init_pull(
                &mut state,
                header.as_ptr(),
                key.0.as_ptr(),
            )
        };
        if rc != 0 {
            return Err(CryptoError::Decrypt);
        }
        Ok(Self {
            state,
            finished: false,
        })
    }

    /// Decrypt one message. Returns the plaintext and whether it was tagged `FINAL`.
    /// Pulling after the final message is an error.
    pub fn pull(&mut self, chunk: &[u8]) -> Result<(Vec<u8>, bool)> {
        if self.finished {
            return Err(CryptoError::Truncated);
        }
        if chunk.len() < STREAM_ABYTES {
            return Err(CryptoError::Decrypt);
        }
        let mut out = vec![0u8; chunk.len() - STREAM_ABYTES];
        let mut out_len: u64 = 0;
        let mut tag: u8 = 0;
        // SAFETY: out has chunk.len() - ABYTES bytes, the maximum libsodium
        // writes; chunk is readable; ad is null with length 0.
        let rc = unsafe {
            ffi::crypto_secretstream_xchacha20poly1305_pull(
                &mut self.state,
                out.as_mut_ptr(),
                &mut out_len,
                &mut tag,
                chunk.as_ptr(),
                chunk.len() as u64,
                std::ptr::null(),
                0,
            )
        };
        if rc != 0 {
            out.zeroize();
            return Err(CryptoError::Decrypt);
        }
        out.truncate(out_len as usize);
        let is_final = tag == TAG_FINAL;
        self.finished = is_final;
        Ok((out, is_final))
    }

    /// Whether the `FINAL` message has been pulled.
    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

impl Drop for StreamDecryptor {
    fn drop(&mut self) {
        wipe_state(&mut self.state);
    }
}

fn empty_state() -> ffi::crypto_secretstream_xchacha20poly1305_state {
    ffi::crypto_secretstream_xchacha20poly1305_state {
        k: [0; 32],
        nonce: [0; 12],
        _pad: [0; 8],
    }
}

fn wipe_state(state: &mut ffi::crypto_secretstream_xchacha20poly1305_state) {
    state.k.zeroize();
    state.nonce.zeroize();
}

/// Ente `encryptBlob`: one secretstream (XChaCha20-Poly1305) message tagged FINAL.
pub fn encrypt_blob(data: &[u8], key: &SecretKey) -> Result<EncryptedBlob> {
    let mut enc = StreamEncryptor::new(key)?;
    let encrypted_data = enc.push(data, true)?;
    Ok(EncryptedBlob {
        encrypted_data,
        decryption_header: enc.header().to_vec(),
    })
}

/// Ente `decryptBlob`: inverse of [`encrypt_blob`]. Requires the `FINAL` tag.
pub fn decrypt_blob(data: &[u8], header: &[u8], key: &SecretKey) -> Result<Vec<u8>> {
    let mut dec = StreamDecryptor::new(header, key)?;
    let (pt, is_final) = dec.pull(data)?;
    if !is_final {
        return Err(CryptoError::Truncated);
    }
    Ok(pt)
}

/// Ente file encryption: 4 MiB plaintext chunks, the last tagged `FINAL`
/// (an empty file is one empty `FINAL` chunk). Used by tests and by result
/// uploads that produce whole buffers (e.g. HLS segments).
pub fn encrypt_file(data: &[u8], key: &SecretKey) -> Result<EncryptedBlob> {
    let enc = StreamEncryptor::new(key)?;
    encrypt_chunks(enc, data)
}

/// [`encrypt_file`] with a fixed header, reproducing another implementation's
/// output. **Only for known-answer tests** (see
/// [`StreamEncryptor::with_header_for_test_vectors`]).
pub fn encrypt_file_with_header_for_test_vectors(
    data: &[u8],
    header: &[u8],
    key: &SecretKey,
) -> Result<EncryptedBlob> {
    let enc = StreamEncryptor::with_header_for_test_vectors(key, header)?;
    encrypt_chunks(enc, data)
}

fn encrypt_chunks(mut enc: StreamEncryptor, data: &[u8]) -> Result<EncryptedBlob> {
    let chunks = data.len().div_ceil(ENCRYPTION_CHUNK_SIZE).max(1);
    let mut out = Vec::with_capacity(data.len() + chunks * STREAM_ABYTES);
    if data.is_empty() {
        out.extend(enc.push(&[], true)?);
    } else {
        let mut iter = data.chunks(ENCRYPTION_CHUNK_SIZE).peekable();
        while let Some(chunk) = iter.next() {
            out.extend(enc.push(chunk, iter.peek().is_none())?);
        }
    }
    Ok(EncryptedBlob {
        encrypted_data: out,
        decryption_header: enc.header().to_vec(),
    })
}

/// Decrypt a whole Ente-encrypted file (original or thumbnail) held in memory.
///
/// Rejects streams without a `FINAL` chunk and data after it. Thumbnails,
/// encrypted as a single blob, decrypt here too.
pub fn decrypt_file(data: &[u8], header: &[u8], key: &SecretKey) -> Result<Vec<u8>> {
    let mut dec = ChunkedDecryptor::new(header, key)?;
    let mut out = dec.update(data)?;
    out.extend(dec.finish()?);
    Ok(out)
}

/// Incremental decryption of an Ente-encrypted file as bytes arrive
/// (e.g. from an HTTP body), without holding the whole ciphertext.
pub struct ChunkedDecryptor {
    inner: StreamDecryptor,
    buf: Vec<u8>,
}

impl ChunkedDecryptor {
    /// Start decrypting with the file's header and key.
    pub fn new(header: &[u8], key: &SecretKey) -> Result<Self> {
        Ok(Self {
            inner: StreamDecryptor::new(header, key)?,
            buf: Vec::with_capacity(DECRYPTION_CHUNK_SIZE),
        })
    }

    /// Feed ciphertext; returns whatever plaintext became available.
    pub fn update(&mut self, mut data: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while !data.is_empty() {
            if self.inner.is_finished() {
                return Err(CryptoError::Truncated);
            }
            let take = (DECRYPTION_CHUNK_SIZE - self.buf.len()).min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            // A full chunk is only decrypted once more input proves it is not
            // the last one; `finish` handles the final (possibly full) chunk.
            if self.buf.len() == DECRYPTION_CHUNK_SIZE && !data.is_empty() {
                let (pt, _) = self.inner.pull(&self.buf)?;
                self.buf.clear();
                out.extend(pt);
            }
        }
        Ok(out)
    }

    /// Decrypt the buffered last chunk and check the stream ended with `FINAL`.
    pub fn finish(mut self) -> Result<Vec<u8>> {
        if self.inner.is_finished() && self.buf.is_empty() {
            return Ok(Vec::new());
        }
        if self.buf.is_empty() {
            return Err(CryptoError::Truncated);
        }
        let (pt, is_final) = self.inner.pull(&self.buf)?;
        if !is_final {
            return Err(CryptoError::Truncated);
        }
        Ok(pt)
    }
}

// ---------------------------------------------------------------------------
// Secretbox
// ---------------------------------------------------------------------------

/// `crypto_secretbox_easy` with a random nonce. Returns `(ciphertext, nonce)`.
pub fn secretbox_seal(msg: &[u8], key: &SecretKey) -> Result<(Vec<u8>, Vec<u8>)> {
    let nonce = random_bytes(NONCE_BYTES);
    let ct = secretbox_seal_with_nonce(msg, &nonce, key)?;
    Ok((ct, nonce))
}

/// `crypto_secretbox_easy` with a caller-provided 24-byte nonce (never reuse one).
pub fn secretbox_seal_with_nonce(msg: &[u8], nonce: &[u8], key: &SecretKey) -> Result<Vec<u8>> {
    ensure_init();
    if nonce.len() != NONCE_BYTES {
        return Err(CryptoError::Length("nonce"));
    }
    let mut out = vec![0u8; msg.len() + SECRETBOX_MAC_BYTES];
    // SAFETY: out has msg.len() + MACBYTES bytes; nonce is 24, key 32 bytes.
    let rc = unsafe {
        ffi::crypto_secretbox_easy(
            out.as_mut_ptr(),
            msg.as_ptr(),
            msg.len() as u64,
            nonce.as_ptr(),
            key.0.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Encrypt);
    }
    Ok(out)
}

/// `crypto_secretbox_open_easy`.
pub fn secretbox_open(ct: &[u8], nonce: &[u8], key: &SecretKey) -> Result<Vec<u8>> {
    ensure_init();
    if nonce.len() != NONCE_BYTES {
        return Err(CryptoError::Length("nonce"));
    }
    if ct.len() < SECRETBOX_MAC_BYTES {
        return Err(CryptoError::Decrypt);
    }
    let mut out = vec![0u8; ct.len() - SECRETBOX_MAC_BYTES];
    // SAFETY: out has ct.len() - MACBYTES bytes; nonce is 24, key 32 bytes.
    let rc = unsafe {
        ffi::crypto_secretbox_open_easy(
            out.as_mut_ptr(),
            ct.as_ptr(),
            ct.len() as u64,
            nonce.as_ptr(),
            key.0.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Decrypt);
    }
    Ok(out)
}

/// [`secretbox_open`] for a wrapped 32-byte key, with base64 inputs as in Ente's JSON.
pub fn secretbox_open_key_b64(ct_b64: &str, nonce_b64: &str, key: &SecretKey) -> Result<SecretKey> {
    let pt = Zeroizing::new(secretbox_open(
        &b64_decode(ct_b64)?,
        &b64_decode(nonce_b64)?,
        key,
    )?);
    SecretKey::from_slice(&pt)
}

// ---------------------------------------------------------------------------
// Public-key: keypairs and sealed boxes
// ---------------------------------------------------------------------------

/// A fresh X25519 keypair `(public_key, secret_key)`.
pub fn box_keypair() -> ([u8; 32], SecretKey) {
    ensure_init();
    let mut pk = [0u8; 32];
    let mut sk = SecretKey([0u8; 32]);
    // SAFETY: both buffers are 32 bytes.
    unsafe { ffi::crypto_box_keypair(pk.as_mut_ptr(), sk.0.as_mut_ptr()) };
    (pk, sk)
}

/// The X25519 public key for `secret_key` (`crypto_scalarmult_base`).
pub fn public_key_from_secret(secret_key: &SecretKey) -> Result<[u8; 32]> {
    ensure_init();
    let mut pk = [0u8; 32];
    // SAFETY: both buffers are 32 bytes.
    let rc = unsafe { ffi::crypto_scalarmult_base(pk.as_mut_ptr(), secret_key.0.as_ptr()) };
    if rc != 0 {
        return Err(CryptoError::Length("secret key"));
    }
    Ok(pk)
}

/// `crypto_box_seal`: anonymously encrypt `msg` to `public_key`.
pub fn sealed_box_seal(msg: &[u8], public_key: &[u8]) -> Result<Vec<u8>> {
    ensure_init();
    if public_key.len() != KEY_BYTES {
        return Err(CryptoError::Length("public key"));
    }
    let mut out = vec![0u8; msg.len() + SEAL_BYTES];
    // SAFETY: out has msg.len() + SEALBYTES bytes; public_key is 32 bytes.
    let rc = unsafe {
        ffi::crypto_box_seal(
            out.as_mut_ptr(),
            msg.as_ptr(),
            msg.len() as u64,
            public_key.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Encrypt);
    }
    Ok(out)
}

/// `crypto_box_seal_open`: open a sealed box with the recipient's keypair.
pub fn sealed_box_open(ct: &[u8], public_key: &[u8], secret_key: &SecretKey) -> Result<Vec<u8>> {
    ensure_init();
    if public_key.len() != KEY_BYTES {
        return Err(CryptoError::Length("public key"));
    }
    if ct.len() < SEAL_BYTES {
        return Err(CryptoError::Decrypt);
    }
    let mut out = vec![0u8; ct.len() - SEAL_BYTES];
    // SAFETY: out has ct.len() - SEALBYTES bytes; keys are 32 bytes each.
    let rc = unsafe {
        ffi::crypto_box_seal_open(
            out.as_mut_ptr(),
            ct.as_ptr(),
            ct.len() as u64,
            public_key.as_ptr(),
            secret_key.0.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Decrypt);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Key derivation
// ---------------------------------------------------------------------------

/// Argon2id cost parameters, as stored in Ente's key attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KdfParams {
    /// Memory limit in bytes (`memLimit`).
    pub mem_limit: u64,
    /// Number of passes (`opsLimit`).
    pub ops_limit: u32,
}

impl KdfParams {
    /// libsodium `INTERACTIVE` (64 MiB, 2 passes).
    pub const INTERACTIVE: Self = Self {
        mem_limit: 67_108_864,
        ops_limit: 2,
    };
    /// libsodium `MODERATE` (256 MiB, 3 passes).
    pub const MODERATE: Self = Self {
        mem_limit: 268_435_456,
        ops_limit: 3,
    };
    /// libsodium `SENSITIVE` (1 GiB, 4 passes).
    pub const SENSITIVE: Self = Self {
        mem_limit: 1_073_741_824,
        ops_limit: 4,
    };

    /// Reject parameters libsodium refuses or that would exhaust the machine,
    /// with the same bounds as Ente's Rust core (server values are untrusted).
    pub fn validate(self) -> Result<()> {
        if self.mem_limit < 8192 || !self.mem_limit.is_multiple_of(1024) || self.ops_limit < 1 {
            return Err(CryptoError::KdfParams(format!(
                "memLimit {} / opsLimit {} below libsodium minimum",
                self.mem_limit, self.ops_limit
            )));
        }
        let max_mem = 4 * Self::SENSITIVE.mem_limit;
        let max_work = u128::from(max_mem) * u128::from(Self::SENSITIVE.ops_limit);
        if self.mem_limit > max_mem
            || u128::from(self.mem_limit) * u128::from(self.ops_limit) > max_work
        {
            return Err(CryptoError::KdfParams(
                "password derivation exceeds supported resource limits".into(),
            ));
        }
        Ok(())
    }
}

/// Ente `deriveKey`: Argon2id v1.3 (`crypto_pwhash`, `ALG_ARGON2ID13`) of the
/// UTF-8 password with a 16-byte salt, giving the 32-byte key-encryption key.
pub fn derive_key(password: &str, salt: &[u8], params: KdfParams) -> Result<SecretKey> {
    ensure_init();
    params.validate()?;
    if salt.len() != SALT_BYTES {
        return Err(CryptoError::Length("salt"));
    }
    let mem = usize::try_from(params.mem_limit)
        .map_err(|_| CryptoError::KdfParams("memLimit too large".into()))?;
    let mut out = SecretKey([0u8; 32]);
    // SAFETY: out is 32 bytes; password is readable for its length; salt is
    // 16 bytes (crypto_pwhash_SALTBYTES); limits validated above.
    let rc = unsafe {
        ffi::crypto_pwhash(
            out.0.as_mut_ptr(),
            32,
            password.as_ptr().cast(),
            password.len() as u64,
            salt.as_ptr(),
            u64::from(params.ops_limit),
            mem,
            ffi::crypto_pwhash_ALG_ARGON2ID13 as i32,
        )
    };
    if rc != 0 {
        return Err(CryptoError::KdfParams(
            "argon2id failed (out of memory?)".into(),
        ));
    }
    Ok(out)
}

/// `crypto_kdf_derive_from_key`: BLAKE2b sub-key `subkey_id` under an 8-byte context.
pub fn derive_subkey(
    key: &SecretKey,
    len: usize,
    subkey_id: u64,
    context: &[u8; 8],
) -> Result<Zeroizing<Vec<u8>>> {
    ensure_init();
    if !(16..=64).contains(&len) {
        return Err(CryptoError::Length("subkey"));
    }
    let mut out = Zeroizing::new(vec![0u8; len]);
    // SAFETY: out has `len` bytes (16..=64, libsodium's bounds); context is
    // exactly crypto_kdf_CONTEXTBYTES (8) bytes; key is 32 bytes.
    let rc = unsafe {
        ffi::crypto_kdf_derive_from_key(
            out.as_mut_ptr(),
            len,
            subkey_id,
            context.as_ptr().cast(),
            key.0.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Length("subkey"));
    }
    Ok(out)
}

/// Ente's SRP password: the first 16 bytes of `crypto_kdf` sub-key 1 (32 bytes,
/// context `"loginctx"`) of the key-encryption key.
pub fn derive_login_key(kek: &SecretKey) -> Result<Zeroizing<Vec<u8>>> {
    let sub = derive_subkey(kek, 32, 1, b"loginctx")?;
    Ok(Zeroizing::new(sub[..16].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_roundtrip_and_tamper() {
        let key = SecretKey::generate();
        let b = encrypt_blob(b"hello", &key).unwrap();
        assert_eq!(b.encrypted_data.len(), 5 + STREAM_ABYTES);
        assert_eq!(
            decrypt_blob(&b.encrypted_data, &b.decryption_header, &key).unwrap(),
            b"hello"
        );
        let mut bad = b.encrypted_data.clone();
        bad[3] ^= 1;
        assert!(decrypt_blob(&bad, &b.decryption_header, &key).is_err());
        assert!(
            decrypt_blob(
                &b.encrypted_data,
                &b.decryption_header,
                &SecretKey::generate()
            )
            .is_err()
        );
    }

    #[test]
    fn file_chunking_roundtrip_and_truncation() {
        let key = SecretKey::generate();
        for size in [0usize, 1, ENCRYPTION_CHUNK_SIZE, ENCRYPTION_CHUNK_SIZE + 5] {
            let pt: Vec<u8> = (0..size).map(|i| i as u8).collect();
            let e = encrypt_file(&pt, &key).unwrap();
            let expected_len = size + size.div_ceil(ENCRYPTION_CHUNK_SIZE).max(1) * STREAM_ABYTES;
            assert_eq!(e.encrypted_data.len(), expected_len);
            assert_eq!(
                decrypt_file(&e.encrypted_data, &e.decryption_header, &key).unwrap(),
                pt
            );
            // Byte-at-a-time feeding gives the same result.
            if size < 100 {
                let mut d = ChunkedDecryptor::new(&e.decryption_header, &key).unwrap();
                let mut got = Vec::new();
                for b in &e.encrypted_data {
                    got.extend(d.update(std::slice::from_ref(b)).unwrap());
                }
                got.extend(d.finish().unwrap());
                assert_eq!(got, pt);
            }
        }
        // Dropping the final chunk of a two-chunk file is detected.
        let pt = vec![7u8; ENCRYPTION_CHUNK_SIZE + 5];
        let e = encrypt_file(&pt, &key).unwrap();
        let cut = &e.encrypted_data[..DECRYPTION_CHUNK_SIZE];
        assert!(matches!(
            decrypt_file(cut, &e.decryption_header, &key),
            Err(CryptoError::Truncated)
        ));
        // Trailing garbage after FINAL is rejected.
        let mut extra = e.encrypted_data.clone();
        extra.extend_from_slice(&[0u8; 40]);
        assert!(decrypt_file(&extra, &e.decryption_header, &key).is_err());
        // Empty input has no FINAL chunk.
        assert!(decrypt_file(&[], &e.decryption_header, &key).is_err());
    }

    #[test]
    fn secretbox_and_sealed_box() {
        let key = SecretKey::generate();
        let (ct, nonce) = secretbox_seal(b"k", &key).unwrap();
        assert_eq!(secretbox_open(&ct, &nonce, &key).unwrap(), b"k");
        assert!(secretbox_open(&ct, &nonce, &SecretKey::generate()).is_err());
        let (pk, sk) = box_keypair();
        assert_eq!(public_key_from_secret(&sk).unwrap(), pk);
        let sealed = sealed_box_seal(b"token", &pk).unwrap();
        assert_eq!(sealed_box_open(&sealed, &pk, &sk).unwrap(), b"token");
        let (pk2, sk2) = box_keypair();
        assert!(sealed_box_open(&sealed, &pk2, &sk2).is_err());
    }

    #[test]
    fn kdf_params_bounds() {
        assert!(
            KdfParams {
                mem_limit: 4096,
                ops_limit: 1
            }
            .validate()
            .is_err()
        );
        assert!(
            KdfParams {
                mem_limit: 8193,
                ops_limit: 1
            }
            .validate()
            .is_err()
        );
        assert!(
            KdfParams {
                mem_limit: 8192,
                ops_limit: 0
            }
            .validate()
            .is_err()
        );
        assert!(
            KdfParams {
                mem_limit: 1 << 33,
                ops_limit: 1
            }
            .validate()
            .is_err()
        );
        assert!(KdfParams::SENSITIVE.validate().is_ok());
        assert!(
            KdfParams {
                mem_limit: 1 << 27,
                ops_limit: 32
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn base64_variants() {
        assert_eq!(b64url_encode(&[0xfb, 0xef]), "--8=");
        assert_eq!(b64url_decode("--8").unwrap(), vec![0xfb, 0xef]);
        assert_eq!(b64url_decode("--8=").unwrap(), vec![0xfb, 0xef]);
        assert_eq!(b64_encode(&[0xfb, 0xef]), "++8=");
        assert!(b64_decode("--8=").is_err());
    }

    #[test]
    fn secret_key_serde_is_base64() {
        let k = SecretKey([9u8; 32]);
        let s = serde_json::to_string(&k).unwrap();
        assert_eq!(s, format!("\"{}\"", b64_encode(&[9u8; 32])));
        let back: SecretKey = serde_json::from_str(&s).unwrap();
        assert_eq!(back, k);
        assert_eq!(format!("{k:?}"), "SecretKey(..)");
    }
}
