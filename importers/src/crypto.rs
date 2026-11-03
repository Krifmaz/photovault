//! Safe wrappers over libsodium, matching exactly what the Ente apps do.
//!
//! Reference: `web/packages/base/crypto/libsodium.ts` and the Go CLI
//! `cli/internal/crypto` in `ente-io/ente`. Every primitive here is checked
//! against vectors produced by that code (see `tests/vectors/`).
//!
//! # Ente conventions this module encodes
//!
//! * **Password → key-encryption key**: `crypto_pwhash` (Argon2id v1.3),
//!   32-byte output, `mem_limit` in bytes, using the user's `kekSalt`,
//!   `opsLimit` and `memLimit` from the key attributes.
//! * **SRP login sub-key**: `crypto_kdf_derive_from_key(32, 1, "loginctx", kek)`,
//!   of which only the first 16 bytes are used as the SRP password.
//! * **Keys** (master key, collection keys, file keys) are wrapped with
//!   `crypto_secretbox_easy` under a random 24-byte nonce.
//! * **Blobs** (file metadata, magic metadata, thumbnails) are a *single*
//!   secretstream message tagged `FINAL`; the 24-byte header is the
//!   `decryptionHeader`.
//! * **File contents** are a secretstream split into 4 MiB plaintext chunks.
//!   Every chunk but the last is exactly [`ENCRYPTION_CHUNK_SIZE`] bytes and
//!   tagged `MESSAGE`; the last is tagged `FINAL`. When the size is an exact
//!   multiple of 4 MiB the last *full* chunk carries `FINAL` (no trailing
//!   empty chunk), and an empty input is one empty `FINAL` chunk (17 bytes).
//!   Decryptors split the ciphertext at `4 MiB + 17` bytes.
//! * **Content hash**: unkeyed BLAKE2b-512 (`crypto_generichash`, 64 bytes)
//!   over the plaintext.
//! * **Base64**: standard alphabet with padding on the wire.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use libsodium_sys as ffi;
use std::sync::Once;

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

/// Size of one ciphertext chunk of a file (`4 MiB + 17`).
const DECRYPTION_CHUNK_SIZE: usize = ENCRYPTION_CHUNK_SIZE + STREAM_ABYTES;
/// Symmetric key size for secretbox and secretstream (32).
const KEY_BYTES: usize = 32;
/// `crypto_secretbox_NONCEBYTES` (24).
const SECRETBOX_NONCE_BYTES: usize = 24;
/// `crypto_secretbox_MACBYTES` (16).
const SECRETBOX_MAC_BYTES: usize = 16;
/// `crypto_box_PUBLICKEYBYTES` / `SECRETKEYBYTES` (32).
const BOX_KEY_BYTES: usize = 32;
/// `crypto_box_SEALBYTES` (48).
const BOX_SEAL_BYTES: usize = 48;
/// `crypto_generichash_BYTES_MAX` (64), the digest size Ente uses.
const HASH_BYTES: usize = 64;
/// Context for the SRP login sub-key, exactly 8 bytes as `crypto_kdf` needs.
const LOGIN_SUBKEY_CONTEXT: &[u8; 8] = b"loginctx";
/// Sub-key id for the SRP login sub-key.
const LOGIN_SUBKEY_ID: u64 = 1;
/// Length of the derived sub-key before truncation.
const LOGIN_SUBKEY_LEN: usize = 32;

const TAG_MESSAGE: u8 = ffi::crypto_secretstream_xchacha20poly1305_TAG_MESSAGE as u8;
const TAG_FINAL: u8 = ffi::crypto_secretstream_xchacha20poly1305_TAG_FINAL as u8;

// Our constants must agree with the linked libsodium.
const _: () = {
    assert!(ffi::crypto_secretstream_xchacha20poly1305_ABYTES as usize == STREAM_ABYTES);
    assert!(ffi::crypto_secretstream_xchacha20poly1305_HEADERBYTES as usize == STREAM_HEADER_BYTES);
    assert!(ffi::crypto_secretstream_xchacha20poly1305_KEYBYTES as usize == KEY_BYTES);
    assert!(ffi::crypto_secretbox_KEYBYTES as usize == KEY_BYTES);
    assert!(ffi::crypto_secretbox_NONCEBYTES as usize == SECRETBOX_NONCE_BYTES);
    assert!(ffi::crypto_secretbox_MACBYTES as usize == SECRETBOX_MAC_BYTES);
    assert!(ffi::crypto_box_PUBLICKEYBYTES as usize == BOX_KEY_BYTES);
    assert!(ffi::crypto_box_SECRETKEYBYTES as usize == BOX_KEY_BYTES);
    assert!(ffi::crypto_box_SEALBYTES as usize == BOX_SEAL_BYTES);
    assert!(ffi::crypto_generichash_BYTES_MAX as usize == HASH_BYTES);
    assert!(ffi::crypto_kdf_CONTEXTBYTES as usize == LOGIN_SUBKEY_CONTEXT.len());
};

static INIT: Once = Once::new();

/// Initialise libsodium. Safe to call many times.
pub fn init() {
    INIT.call_once(|| {
        // SAFETY: sodium_init is thread-safe and idempotent; it has no
        // preconditions.
        let rc = unsafe { ffi::sodium_init() };
        assert!(rc >= 0, "libsodium failed to initialise");
    });
}

fn check_len(data: &[u8], len: usize, what: &'static str) -> Result<()> {
    if data.len() == len {
        Ok(())
    } else {
        Err(CryptoError::Invalid(what))
    }
}

/// `n` cryptographically secure random bytes.
pub fn random_bytes(n: usize) -> Vec<u8> {
    init();
    let mut buf = vec![0u8; n];
    // SAFETY: `buf` is valid for writes of `n` bytes.
    unsafe { ffi::randombytes_buf(buf.as_mut_ptr().cast(), n) };
    buf
}

/// A fresh 32-byte key (`crypto_secretbox_keygen` / `crypto_secretstream_..._keygen`).
///
/// Both libsodium keygens are plain `randombytes_buf(32)`.
pub fn generate_key() -> Vec<u8> {
    random_bytes(KEY_BYTES)
}

/// Standard base64 with padding (what Ente sends on the wire).
pub fn b64(data: &[u8]) -> String {
    STANDARD.encode(data)
}

/// Decode standard base64 (also accepts URL-safe input, like the Ente apps).
///
/// Padding is optional in either alphabet. Surrounding whitespace is ignored.
pub fn unb64(s: &str) -> Result<Vec<u8>> {
    let s = s.trim();
    STANDARD
        .decode(s)
        .or_else(|_| URL_SAFE.decode(s))
        .or_else(|_| STANDARD_NO_PAD.decode(s))
        .or_else(|_| URL_SAFE_NO_PAD.decode(s))
        .map_err(|_| CryptoError::Invalid("bad base64"))
}

/// Argon2id key derivation (`crypto_pwhash` with `ALG_ARGON2ID13`), 32-byte
/// output, as Ente derives the key-encryption key from the password.
///
/// `mem_limit` is in bytes (Ente's `memLimit`), `ops_limit` is Ente's
/// `opsLimit`. The salt must be 16 bytes.
pub fn derive_argon_key(
    password: &[u8],
    salt: &[u8],
    ops_limit: u64,
    mem_limit: u64,
) -> Result<Vec<u8>> {
    init();
    check_len(
        salt,
        ffi::crypto_pwhash_SALTBYTES as usize,
        "argon salt must be 16 bytes",
    )?;
    // SAFETY: these getters have no preconditions.
    let (ops_min, mem_min) = unsafe {
        (
            ffi::crypto_pwhash_opslimit_min(),
            ffi::crypto_pwhash_memlimit_min(),
        )
    };
    if ops_limit < ops_min as u64 || mem_limit < mem_min as u64 {
        return Err(CryptoError::Invalid("argon limits below libsodium minimum"));
    }
    let mem_limit = usize::try_from(mem_limit)
        .map_err(|_| CryptoError::Invalid("argon mem limit too large"))?;
    let mut out = vec![0u8; KEY_BYTES];
    // SAFETY: `out` is valid for `out.len()` bytes; `password` for
    // `password.len()` bytes; `salt` was checked to be SALTBYTES long.
    let rc = unsafe {
        ffi::crypto_pwhash(
            out.as_mut_ptr(),
            out.len() as u64,
            password.as_ptr().cast(),
            password.len() as u64,
            salt.as_ptr(),
            ops_limit,
            mem_limit,
            ffi::crypto_pwhash_ALG_ARGON2ID13 as i32,
        )
    };
    if rc != 0 {
        return Err(CryptoError::Invalid(
            "argon2id derivation failed (out of memory?)",
        ));
    }
    Ok(out)
}

/// Ente's login sub-key for SRP: `crypto_kdf_derive_from_key(32, 1, "loginctx", kek)`
/// truncated to the first 16 bytes.
pub fn derive_login_key(kek: &[u8]) -> Result<Vec<u8>> {
    init();
    check_len(
        kek,
        ffi::crypto_kdf_KEYBYTES as usize,
        "kek must be 32 bytes",
    )?;
    let mut sub = [0u8; LOGIN_SUBKEY_LEN];
    // SAFETY: `sub` is valid for LOGIN_SUBKEY_LEN bytes (within
    // crypto_kdf_BYTES_MIN..=MAX); the context is exactly CONTEXTBYTES
    // (it need not be NUL-terminated); `kek` was checked to be KEYBYTES.
    let rc = unsafe {
        ffi::crypto_kdf_derive_from_key(
            sub.as_mut_ptr(),
            sub.len(),
            LOGIN_SUBKEY_ID,
            LOGIN_SUBKEY_CONTEXT.as_ptr().cast(),
            kek.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Invalid("kdf failed"));
    }
    let out = sub[..16].to_vec();
    zero(&mut sub);
    Ok(out)
}

/// `crypto_secretbox_easy` with a fresh random nonce. Returns `(ciphertext, nonce)`.
pub fn secretbox_seal(msg: &[u8], key: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let nonce = random_bytes(SECRETBOX_NONCE_BYTES);
    let ct = secretbox_seal_with_nonce(msg, &nonce, key)?;
    Ok((ct, nonce))
}

fn secretbox_seal_with_nonce(msg: &[u8], nonce: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    init();
    check_len(key, KEY_BYTES, "secretbox key must be 32 bytes")?;
    check_len(
        nonce,
        SECRETBOX_NONCE_BYTES,
        "secretbox nonce must be 24 bytes",
    )?;
    let mut out = vec![0u8; msg.len() + SECRETBOX_MAC_BYTES];
    // SAFETY: `out` holds msg.len() + MACBYTES bytes as required; `nonce`
    // and `key` lengths were checked.
    let rc = unsafe {
        ffi::crypto_secretbox_easy(
            out.as_mut_ptr(),
            msg.as_ptr(),
            msg.len() as u64,
            nonce.as_ptr(),
            key.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Invalid("secretbox seal failed"));
    }
    Ok(out)
}

/// `crypto_secretbox_open_easy`.
pub fn secretbox_open(ct: &[u8], nonce: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    init();
    check_len(key, KEY_BYTES, "secretbox key must be 32 bytes")?;
    check_len(
        nonce,
        SECRETBOX_NONCE_BYTES,
        "secretbox nonce must be 24 bytes",
    )?;
    if ct.len() < SECRETBOX_MAC_BYTES {
        return Err(CryptoError::Invalid("secretbox ciphertext too short"));
    }
    let mut out = vec![0u8; ct.len() - SECRETBOX_MAC_BYTES];
    // SAFETY: `ct` is at least MACBYTES long and `out` holds
    // ct.len() - MACBYTES bytes; `nonce` and `key` lengths were checked.
    let rc = unsafe {
        ffi::crypto_secretbox_open_easy(
            out.as_mut_ptr(),
            ct.as_ptr(),
            ct.len() as u64,
            nonce.as_ptr(),
            key.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Invalid("secretbox authentication failed"));
    }
    Ok(out)
}

/// `crypto_box_seal_open` (used to decrypt the auth token).
pub fn box_seal_open(ct: &[u8], public_key: &[u8], secret_key: &[u8]) -> Result<Vec<u8>> {
    init();
    check_len(public_key, BOX_KEY_BYTES, "box public key must be 32 bytes")?;
    check_len(secret_key, BOX_KEY_BYTES, "box secret key must be 32 bytes")?;
    if ct.len() < BOX_SEAL_BYTES {
        return Err(CryptoError::Invalid("sealed box too short"));
    }
    let mut out = vec![0u8; ct.len() - BOX_SEAL_BYTES];
    // SAFETY: `ct` is at least SEALBYTES long and `out` holds
    // ct.len() - SEALBYTES bytes; key lengths were checked.
    let rc = unsafe {
        ffi::crypto_box_seal_open(
            out.as_mut_ptr(),
            ct.as_ptr(),
            ct.len() as u64,
            public_key.as_ptr(),
            secret_key.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Invalid("sealed box authentication failed"));
    }
    Ok(out)
}

/// `crypto_box_seal` (used when sharing keys; kept for tests and symmetry).
pub fn box_seal(msg: &[u8], public_key: &[u8]) -> Result<Vec<u8>> {
    init();
    check_len(public_key, BOX_KEY_BYTES, "box public key must be 32 bytes")?;
    let mut out = vec![0u8; msg.len() + BOX_SEAL_BYTES];
    // SAFETY: `out` holds msg.len() + SEALBYTES bytes; the key length was
    // checked.
    let rc = unsafe {
        ffi::crypto_box_seal(
            out.as_mut_ptr(),
            msg.as_ptr(),
            msg.len() as u64,
            public_key.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Invalid("sealed box failed (bad public key?)"));
    }
    Ok(out)
}

/// `crypto_box_keypair`. Returns `(public_key, secret_key)`.
pub fn box_keypair() -> (Vec<u8>, Vec<u8>) {
    init();
    let mut pk = vec![0u8; BOX_KEY_BYTES];
    let mut sk = vec![0u8; BOX_KEY_BYTES];
    // SAFETY: both buffers are PUBLICKEYBYTES / SECRETKEYBYTES long.
    let rc = unsafe { ffi::crypto_box_keypair(pk.as_mut_ptr(), sk.as_mut_ptr()) };
    assert_eq!(rc, 0, "crypto_box_keypair cannot fail");
    (pk, sk)
}

/// Encrypt a small blob as a single secretstream message tagged FINAL, as
/// Ente does for metadata, magic metadata and thumbnails.
/// Returns `(ciphertext, header)`.
pub fn encrypt_blob(data: &[u8], key: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let (mut enc, header) = StreamEncryptor::new(key)?;
    let ct = enc.push_tagged(data, TAG_FINAL)?;
    Ok((ct, header))
}

/// Inverse of [`encrypt_blob`].
///
/// The ciphertext must be exactly one secretstream message tagged FINAL.
pub fn decrypt_blob(ct: &[u8], header: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    let mut dec = StreamDecryptor::new(key, header)?;
    let (pt, is_final) = dec.pull_any(ct)?;
    if !is_final {
        return Err(CryptoError::Invalid("blob is not tagged FINAL"));
    }
    Ok(pt)
}

/// Streaming secretstream encryptor for file contents: plaintext in
/// [`ENCRYPTION_CHUNK_SIZE`] chunks, every chunk tagged MESSAGE except the
/// last, which is tagged FINAL (exactly like the Ente apps).
///
/// Ente's decryptors cut the ciphertext at `ENCRYPTION_CHUNK_SIZE +
/// STREAM_ABYTES` bytes, so every non-final chunk must be exactly
/// [`ENCRYPTION_CHUNK_SIZE`] bytes; [`Self::push`] enforces that. The final
/// chunk may be 0..=[`ENCRYPTION_CHUNK_SIZE`] bytes; for a file whose size is
/// an exact multiple of the chunk size, mark the last full chunk final (do
/// not push a trailing empty chunk).
pub struct StreamEncryptor {
    state: Box<ffi::crypto_secretstream_xchacha20poly1305_state>,
    finished: bool,
}

impl StreamEncryptor {
    /// Start a stream with `key` (32 bytes). Returns the encryptor and the
    /// 24-byte header the server stores as `decryptionHeader`.
    pub fn new(key: &[u8]) -> Result<(Self, Vec<u8>)> {
        init();
        check_len(key, KEY_BYTES, "stream key must be 32 bytes")?;
        let mut state = new_stream_state();
        let mut header = vec![0u8; STREAM_HEADER_BYTES];
        // SAFETY: `state` points to a valid state struct, `header` holds
        // HEADERBYTES bytes and `key` was checked to be KEYBYTES.
        let rc = unsafe {
            ffi::crypto_secretstream_xchacha20poly1305_init_push(
                &mut *state,
                header.as_mut_ptr(),
                key.as_ptr(),
            )
        };
        if rc != 0 {
            return Err(CryptoError::Invalid("secretstream init failed"));
        }
        Ok((
            Self {
                state,
                finished: false,
            },
            header,
        ))
    }

    /// Start a stream with a caller-chosen header, for reproducing known
    /// ciphertexts in tests. libsodium's push and pull states are identical
    /// after init (HChaCha20 sub-key, counter 1, header nonce), so
    /// `init_pull` with the header yields the state `init_push` would have
    /// produced had it drawn this header.
    #[cfg(test)]
    pub(crate) fn with_header(key: &[u8], header: &[u8]) -> Result<Self> {
        let mut dec = StreamDecryptor::new(key, header)?;
        let state = std::mem::replace(&mut dec.state, new_stream_state());
        Ok(Self {
            state,
            finished: false,
        })
    }

    /// Encrypt one chunk. `last` must be true for the final chunk only.
    pub fn push(&mut self, chunk: &[u8], last: bool) -> Result<Vec<u8>> {
        if chunk.len() > ENCRYPTION_CHUNK_SIZE {
            return Err(CryptoError::Invalid("chunk larger than 4 MiB"));
        }
        if !last && chunk.len() != ENCRYPTION_CHUNK_SIZE {
            return Err(CryptoError::Invalid(
                "non-final chunk must be exactly 4 MiB",
            ));
        }
        self.push_tagged(chunk, if last { TAG_FINAL } else { TAG_MESSAGE })
    }

    fn push_tagged(&mut self, msg: &[u8], tag: u8) -> Result<Vec<u8>> {
        if self.finished {
            return Err(CryptoError::Invalid("stream already finished"));
        }
        let mut out = vec![0u8; msg.len() + STREAM_ABYTES];
        let mut out_len: u64 = 0;
        // SAFETY: `state` was initialised by init_push/init_pull; `out`
        // holds msg.len() + ABYTES bytes; no additional data is passed.
        let rc = unsafe {
            ffi::crypto_secretstream_xchacha20poly1305_push(
                &mut *self.state,
                out.as_mut_ptr(),
                &mut out_len,
                msg.as_ptr(),
                msg.len() as u64,
                std::ptr::null(),
                0,
                tag,
            )
        };
        if rc != 0 {
            return Err(CryptoError::Invalid("secretstream push failed"));
        }
        out.truncate(out_len as usize);
        if tag == TAG_FINAL {
            self.finished = true;
        }
        Ok(out)
    }
}

impl Drop for StreamEncryptor {
    fn drop(&mut self) {
        zero_state(&mut self.state);
    }
}

/// Streaming secretstream decryptor (used by tests and dedup checks).
pub struct StreamDecryptor {
    state: Box<ffi::crypto_secretstream_xchacha20poly1305_state>,
    finished: bool,
}

impl StreamDecryptor {
    /// Start decrypting a stream with `key` and `header`.
    pub fn new(key: &[u8], header: &[u8]) -> Result<Self> {
        init();
        check_len(key, KEY_BYTES, "stream key must be 32 bytes")?;
        check_len(
            header,
            STREAM_HEADER_BYTES,
            "stream header must be 24 bytes",
        )?;
        let mut state = new_stream_state();
        // SAFETY: `state` points to a valid state struct; `header` and
        // `key` lengths were checked.
        let rc = unsafe {
            ffi::crypto_secretstream_xchacha20poly1305_init_pull(
                &mut *state,
                header.as_ptr(),
                key.as_ptr(),
            )
        };
        if rc != 0 {
            return Err(CryptoError::Invalid("secretstream header rejected"));
        }
        Ok(Self {
            state,
            finished: false,
        })
    }

    /// Decrypt one ciphertext chunk (`ENCRYPTION_CHUNK_SIZE + STREAM_ABYTES`
    /// bytes, or less for the last). Returns `(plaintext, is_final)`.
    ///
    /// A MESSAGE-tagged chunk must be full size, as Ente never writes
    /// anything else; tags other than MESSAGE and FINAL are rejected.
    pub fn pull(&mut self, chunk: &[u8]) -> Result<(Vec<u8>, bool)> {
        if chunk.len() > DECRYPTION_CHUNK_SIZE {
            return Err(CryptoError::Invalid(
                "ciphertext chunk larger than 4 MiB + 17",
            ));
        }
        let (pt, is_final) = self.pull_any(chunk)?;
        if !is_final && chunk.len() != DECRYPTION_CHUNK_SIZE {
            return Err(CryptoError::Invalid("short non-final chunk"));
        }
        Ok((pt, is_final))
    }

    fn pull_any(&mut self, chunk: &[u8]) -> Result<(Vec<u8>, bool)> {
        if self.finished {
            return Err(CryptoError::Invalid("data after FINAL chunk"));
        }
        if chunk.len() < STREAM_ABYTES {
            return Err(CryptoError::Invalid("ciphertext chunk too short"));
        }
        let mut out = vec![0u8; chunk.len() - STREAM_ABYTES];
        let mut out_len: u64 = 0;
        let mut tag: u8 = 0;
        // SAFETY: `state` was initialised by init_pull; `chunk` is at least
        // ABYTES long and `out` holds chunk.len() - ABYTES bytes; no
        // additional data is passed.
        let rc = unsafe {
            ffi::crypto_secretstream_xchacha20poly1305_pull(
                &mut *self.state,
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
            return Err(CryptoError::Invalid("secretstream authentication failed"));
        }
        out.truncate(out_len as usize);
        match tag {
            TAG_FINAL => {
                self.finished = true;
                Ok((out, true))
            }
            TAG_MESSAGE => Ok((out, false)),
            _ => Err(CryptoError::Invalid("unexpected secretstream tag")),
        }
    }
}

impl Drop for StreamDecryptor {
    fn drop(&mut self) {
        zero_state(&mut self.state);
    }
}

/// Decrypt a whole secretstream ciphertext produced with Ente's chunking.
///
/// Fails unless the last chunk, and only the last, is tagged FINAL.
pub fn decrypt_stream(ct: &[u8], header: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    let mut dec = StreamDecryptor::new(key, header)?;
    if ct.is_empty() {
        return Err(CryptoError::Invalid("empty ciphertext"));
    }
    let mut out = Vec::with_capacity(
        ct.len()
            .saturating_sub(ct.len() / DECRYPTION_CHUNK_SIZE * STREAM_ABYTES),
    );
    let mut chunks = ct.chunks(DECRYPTION_CHUNK_SIZE).peekable();
    while let Some(chunk) = chunks.next() {
        let (pt, is_final) = dec.pull(chunk)?;
        out.extend_from_slice(&pt);
        match (is_final, chunks.peek().is_some()) {
            (true, true) => return Err(CryptoError::Invalid("data after FINAL chunk")),
            (false, false) => {
                return Err(CryptoError::Invalid("stream truncated (no FINAL chunk)"));
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Streaming BLAKE2b (`crypto_generichash`, 64-byte output, no key), as Ente
/// computes the content hash stored in file metadata.
pub struct Hasher {
    state: Box<ffi::crypto_generichash_state>,
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher {
    /// New hasher.
    pub fn new() -> Self {
        init();
        let mut state = Box::new(ffi::crypto_generichash_state { opaque: [0u8; 384] });
        // SAFETY: `state` is a valid, suitably aligned (Box honours the
        // struct's align(64)) state struct; no key; output length 64 is
        // within BYTES_MIN..=BYTES_MAX.
        let rc =
            unsafe { ffi::crypto_generichash_init(&mut *state, std::ptr::null(), 0, HASH_BYTES) };
        assert_eq!(
            rc, 0,
            "crypto_generichash_init cannot fail with these arguments"
        );
        Self { state }
    }

    /// Feed bytes.
    pub fn update(&mut self, data: &[u8]) {
        // SAFETY: `state` was initialised in `new`; `data` is valid for
        // data.len() bytes.
        let rc = unsafe {
            ffi::crypto_generichash_update(&mut *self.state, data.as_ptr(), data.len() as u64)
        };
        assert_eq!(rc, 0, "crypto_generichash_update cannot fail");
    }

    /// Finish: 64-byte digest.
    pub fn finalize(mut self) -> Vec<u8> {
        let mut out = vec![0u8; HASH_BYTES];
        // SAFETY: `state` was initialised in `new` and is finalised once
        // (`self` is consumed); `out` holds the 64 bytes chosen at init.
        let rc =
            unsafe { ffi::crypto_generichash_final(&mut *self.state, out.as_mut_ptr(), out.len()) };
        assert_eq!(rc, 0, "crypto_generichash_final cannot fail");
        out
    }
}

fn new_stream_state() -> Box<ffi::crypto_secretstream_xchacha20poly1305_state> {
    Box::new(ffi::crypto_secretstream_xchacha20poly1305_state {
        k: [0; 32],
        nonce: [0; 12],
        _pad: [0; 8],
    })
}

fn zero(buf: &mut [u8]) {
    // SAFETY: `buf` is valid for writes of buf.len() bytes.
    unsafe { ffi::sodium_memzero(buf.as_mut_ptr().cast(), buf.len()) };
}

fn zero_state(state: &mut ffi::crypto_secretstream_xchacha20poly1305_state) {
    zero(&mut state.k);
    zero(&mut state.nonce);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(seed: u32, n: usize) -> Vec<u8> {
        (0..n)
            .map(|i| ((i as u32).wrapping_add(seed).wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect()
    }

    /// Encrypt with Ente's chunking: 4 MiB chunks, last (possibly full or
    /// empty) one FINAL.
    fn encrypt_all(enc: &mut StreamEncryptor, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        if data.is_empty() {
            out.extend(enc.push(&[], true).unwrap());
            return out;
        }
        let n = data.len().div_ceil(ENCRYPTION_CHUNK_SIZE);
        for (i, c) in data.chunks(ENCRYPTION_CHUNK_SIZE).enumerate() {
            out.extend(enc.push(c, i + 1 == n).unwrap());
        }
        out
    }

    #[test]
    fn base64_variants() {
        let data = [0xfbu8, 0xff, 0xfe, 0x01];
        assert_eq!(b64(&data), "+//+AQ==");
        assert_eq!(unb64("+//+AQ==").unwrap(), data);
        assert_eq!(unb64("-__-AQ==").unwrap(), data);
        assert_eq!(unb64("-__-AQ").unwrap(), data);
        assert_eq!(unb64("+//+AQ").unwrap(), data);
        assert_eq!(unb64(" +//+AQ==\n").unwrap(), data);
        assert!(unb64("not base64!").is_err());
        assert_eq!(unb64("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn random_and_keys() {
        assert_eq!(random_bytes(0).len(), 0);
        let a = generate_key();
        let b = generate_key();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }

    #[test]
    fn argon_rejects_bad_input() {
        assert!(derive_argon_key(b"pw", &[0; 15], 2, 64 << 20).is_err());
        assert!(derive_argon_key(b"pw", &[0; 16], 0, 64 << 20).is_err());
        assert!(derive_argon_key(b"pw", &[0; 16], 1, 1024).is_err());
        let k = derive_argon_key(b"pw", &[0; 16], 1, 8192).unwrap();
        assert_eq!(k.len(), 32);
    }

    #[test]
    fn login_key_len() {
        assert_eq!(derive_login_key(&[7; 32]).unwrap().len(), 16);
        assert!(derive_login_key(&[7; 16]).is_err());
    }

    #[test]
    fn secretbox_roundtrip_and_tamper() {
        let key = generate_key();
        let (mut ct, nonce) = secretbox_seal(b"master key", &key).unwrap();
        assert_eq!(ct.len(), 10 + 16);
        assert_eq!(secretbox_open(&ct, &nonce, &key).unwrap(), b"master key");
        ct[0] ^= 1;
        assert!(secretbox_open(&ct, &nonce, &key).is_err());
        assert!(secretbox_open(&[0; 15], &nonce, &key).is_err());
        assert!(secretbox_open(&ct, &nonce[..23], &key).is_err());
        assert!(secretbox_seal(b"x", &key[..31]).is_err());
        let (ct, nonce) = secretbox_seal(b"", &key).unwrap();
        assert_eq!(secretbox_open(&ct, &nonce, &key).unwrap(), b"");
    }

    #[test]
    fn sealed_box_roundtrip() {
        let (pk, sk) = box_keypair();
        let ct = box_seal(b"token", &pk).unwrap();
        assert_eq!(ct.len(), 5 + 48);
        assert_eq!(box_seal_open(&ct, &pk, &sk).unwrap(), b"token");
        let (pk2, sk2) = box_keypair();
        assert!(box_seal_open(&ct, &pk2, &sk2).is_err());
        assert!(box_seal_open(&ct[..47], &pk, &sk).is_err());
        assert!(box_seal(b"x", &pk[..31]).is_err());
    }

    #[test]
    fn blob_roundtrip() {
        let key = generate_key();
        for data in [&b""[..], b"{}", &[9u8; 5000]] {
            let (ct, header) = encrypt_blob(data, &key).unwrap();
            assert_eq!(ct.len(), data.len() + STREAM_ABYTES);
            assert_eq!(header.len(), STREAM_HEADER_BYTES);
            assert_eq!(decrypt_blob(&ct, &header, &key).unwrap(), data);
            assert!(decrypt_blob(&ct, &header, &generate_key()).is_err());
        }
    }

    #[test]
    fn blob_must_be_final() {
        let key = generate_key();
        let (mut enc, header) = StreamEncryptor::new(&key).unwrap();
        let ct = enc.push_tagged(b"abc", TAG_MESSAGE).unwrap();
        assert!(decrypt_blob(&ct, &header, &key).is_err());
    }

    #[test]
    fn stream_sizes() {
        let key = generate_key();
        let c = ENCRYPTION_CHUNK_SIZE;
        for (len, chunks) in [
            (0, 1),
            (1, 1),
            (c - 1, 1),
            (c, 1),
            (c + 1, 2),
            (2 * c, 2),
            (2 * c + 3, 3),
        ] {
            let data = pattern(len as u32, len);
            let (mut enc, header) = StreamEncryptor::new(&key).unwrap();
            let ct = encrypt_all(&mut enc, &data);
            assert_eq!(ct.len(), len + chunks * STREAM_ABYTES, "len {len}");
            assert_eq!(
                decrypt_stream(&ct, &header, &key).unwrap(),
                data,
                "len {len}"
            );
        }
    }

    #[test]
    fn stream_rejects_misuse() {
        let key = generate_key();
        let (mut enc, header) = StreamEncryptor::new(&key).unwrap();
        assert!(enc.push(&[0; 10], false).is_err(), "short non-final chunk");
        assert!(enc.push(&vec![0; ENCRYPTION_CHUNK_SIZE + 1], true).is_err());
        let full = vec![1u8; ENCRYPTION_CHUNK_SIZE];
        let c1 = enc.push(&full, false).unwrap();
        let c2 = enc.push(b"tail", true).unwrap();
        assert!(enc.push(b"more", true).is_err(), "push after FINAL");

        // Truncated: only the MESSAGE chunk.
        assert!(decrypt_stream(&c1, &header, &key).is_err());
        // Trailing data after FINAL.
        let mut extra = [c1.clone(), c2.clone()].concat();
        let ok = decrypt_stream(&extra, &header, &key).unwrap();
        assert_eq!(ok.len(), ENCRYPTION_CHUNK_SIZE + 4);
        extra.extend_from_slice(&[0; STREAM_ABYTES]);
        assert!(decrypt_stream(&extra, &header, &key).is_err());
        // Reordered chunks fail authentication.
        assert!(decrypt_stream(&[c2.clone(), c1.clone()].concat(), &header, &key).is_err());
        assert!(decrypt_stream(&[], &header, &key).is_err());
        assert!(StreamDecryptor::new(&key, &header[..23]).is_err());
        assert!(StreamEncryptor::new(&key[..16]).is_err());

        // Pull after FINAL.
        let mut dec = StreamDecryptor::new(&key, &header).unwrap();
        assert!(!dec.pull(&c1).unwrap().1);
        assert!(dec.pull(&c2).unwrap().1);
        assert!(dec.pull(&c2).is_err());
    }

    #[test]
    fn short_message_chunk_rejected_by_pull() {
        let key = generate_key();
        let (mut enc, header) = StreamEncryptor::new(&key).unwrap();
        let ct = enc.push_tagged(b"short", TAG_MESSAGE).unwrap();
        let mut dec = StreamDecryptor::new(&key, &header).unwrap();
        assert!(dec.pull(&ct).is_err());
    }

    #[test]
    fn with_header_matches_init_push() {
        let key = generate_key();
        let (mut a, header) = StreamEncryptor::new(&key).unwrap();
        let mut b = StreamEncryptor::with_header(&key, &header).unwrap();
        let full = vec![3u8; ENCRYPTION_CHUNK_SIZE];
        assert_eq!(a.push(&full, false).unwrap(), b.push(&full, false).unwrap());
        assert_eq!(a.push(b"x", true).unwrap(), b.push(b"x", true).unwrap());
    }

    #[test]
    fn hasher_known_answer_and_streaming() {
        // BLAKE2b-512("abc"), RFC 7693 Appendix A.
        let mut h = Hasher::new();
        h.update(b"abc");
        assert_eq!(
            hex::encode(h.finalize()),
            "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d1\
             7d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923"
        );
        let data = pattern(1, 10_000);
        let mut one = Hasher::default();
        one.update(&data);
        let mut many = Hasher::new();
        for c in data.chunks(333) {
            many.update(c);
        }
        assert_eq!(one.finalize(), many.finalize());
    }

    // ---- vectors from Ente's own code (tests/vectors/*.json) ----

    fn node_vectors() -> serde_json::Value {
        serde_json::from_str(include_str!("../tests/vectors/node.json")).unwrap()
    }

    fn field(v: &serde_json::Value, k: &str) -> Vec<u8> {
        unb64(v[k].as_str().unwrap()).unwrap()
    }

    /// Multi-chunk and chunk-boundary streams encrypted by Ente's web
    /// `encryptStreamBytes`: we reproduce the ciphertext byte for byte from
    /// the recorded key and header (only its SHA-256 is committed), then
    /// decrypt it with our decryptor.
    #[test]
    fn ente_web_large_streams() {
        use sha2::{Digest, Sha256};
        let v = node_vectors();
        let cases = v["stream_large"].as_array().unwrap();
        assert!(cases.len() >= 5);
        for c in cases {
            let key = field(c, "key");
            let header = field(c, "header");
            let len = c["len"].as_u64().unwrap() as usize;
            let seed = c["seed"].as_u64().unwrap() as u32;
            let data = pattern(seed, len);
            let mut enc = StreamEncryptor::with_header(&key, &header).unwrap();
            let ct = encrypt_all(&mut enc, &data);
            assert_eq!(
                ct.len() as u64,
                c["ciphertext_len"].as_u64().unwrap(),
                "len {len}"
            );
            assert_eq!(
                b64(&Sha256::digest(&ct)),
                c["ciphertext_sha256"].as_str().unwrap(),
                "len {len}"
            );
            assert_eq!(
                decrypt_stream(&ct, &header, &key).unwrap(),
                data,
                "len {len}"
            );
        }
    }

    /// Our encryptor reproduces Ente's small streams exactly too.
    #[test]
    fn ente_web_small_streams_reproduced() {
        let v = node_vectors();
        for c in v["stream_small"].as_array().unwrap() {
            let key = field(c, "key");
            let header = field(c, "header");
            let data = pattern(
                c["seed"].as_u64().unwrap() as u32,
                c["len"].as_u64().unwrap() as usize,
            );
            let mut enc = StreamEncryptor::with_header(&key, &header).unwrap();
            assert_eq!(encrypt_all(&mut enc, &data), field(c, "ciphertext"));
        }
    }

    /// Secretbox with a recorded nonce reproduces Ente's ciphertext.
    #[test]
    fn ente_web_secretbox_reproduced() {
        let v = node_vectors();
        for c in v["secretbox"].as_array().unwrap() {
            let ct = secretbox_seal_with_nonce(
                &field(c, "plaintext"),
                &field(c, "nonce"),
                &field(c, "key"),
            )
            .unwrap();
            assert_eq!(ct, field(c, "ciphertext"));
        }
    }
}
