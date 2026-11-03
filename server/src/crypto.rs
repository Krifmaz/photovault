//! Thin safe wrappers over libsodium (the same library the Ente apps use).
//!
//! The server never decrypts user content. It only needs: secure random bytes,
//! sealed boxes (to hand auth tokens to clients encrypted to their public key,
//! exactly like museum), secretbox for encrypting emails at rest, and
//! generichash for keyed email hashes.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE};
use libsodium_sys as ffi;
use std::sync::Once;

static INIT: Once = Once::new();

/// Initialise libsodium. Safe to call many times.
pub fn init() {
    INIT.call_once(|| {
        // SAFETY: sodium_init is thread-safe and idempotent.
        let rc = unsafe { ffi::sodium_init() };
        assert!(rc >= 0, "libsodium failed to initialise");
    });
}

/// `n` cryptographically secure random bytes.
pub fn random_bytes(n: usize) -> Vec<u8> {
    init();
    let mut buf = vec![0u8; n];
    // SAFETY: buf is valid for n bytes.
    unsafe { ffi::randombytes_buf(buf.as_mut_ptr().cast(), n) };
    buf
}

/// Errors from crypto helpers.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Input had the wrong length or failed authentication.
    #[error("invalid input")]
    Invalid,
}

/// `crypto_box_seal`: anonymously encrypt `msg` to `public_key` (32 bytes).
pub fn box_seal(msg: &[u8], public_key: &[u8]) -> Result<Vec<u8>, CryptoError> {
    init();
    if public_key.len() != ffi::crypto_box_PUBLICKEYBYTES as usize {
        return Err(CryptoError::Invalid);
    }
    let mut out = vec![0u8; msg.len() + ffi::crypto_box_SEALBYTES as usize];
    // SAFETY: sizes checked above.
    let rc = unsafe {
        ffi::crypto_box_seal(
            out.as_mut_ptr(),
            msg.as_ptr(),
            msg.len() as u64,
            public_key.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(CryptoError::Invalid);
    }
    Ok(out)
}

/// `crypto_box_seal_open`. Used only in tests to play the client role.
pub fn box_seal_open(
    ct: &[u8],
    public_key: &[u8],
    secret_key: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    init();
    let seal = ffi::crypto_box_SEALBYTES as usize;
    if ct.len() < seal || public_key.len() != 32 || secret_key.len() != 32 {
        return Err(CryptoError::Invalid);
    }
    let mut out = vec![0u8; ct.len() - seal];
    // SAFETY: sizes checked above.
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
        return Err(CryptoError::Invalid);
    }
    Ok(out)
}

/// `crypto_box_keypair` → (public, secret). Used in tests to play the client role.
pub fn box_keypair() -> (Vec<u8>, Vec<u8>) {
    init();
    let mut pk = vec![0u8; 32];
    let mut sk = vec![0u8; 32];
    // SAFETY: both buffers are 32 bytes.
    unsafe { ffi::crypto_box_keypair(pk.as_mut_ptr(), sk.as_mut_ptr()) };
    (pk, sk)
}

/// `crypto_secretbox_easy` with a random nonce; returns (ciphertext, nonce).
pub fn secretbox(msg: &[u8], key: &[u8]) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
    init();
    if key.len() != 32 {
        return Err(CryptoError::Invalid);
    }
    let nonce = random_bytes(ffi::crypto_secretbox_NONCEBYTES as usize);
    let mut out = vec![0u8; msg.len() + ffi::crypto_secretbox_MACBYTES as usize];
    // SAFETY: sizes checked.
    unsafe {
        ffi::crypto_secretbox_easy(
            out.as_mut_ptr(),
            msg.as_ptr(),
            msg.len() as u64,
            nonce.as_ptr(),
            key.as_ptr(),
        )
    };
    Ok((out, nonce))
}

/// `crypto_secretbox_open_easy`.
pub fn secretbox_open(ct: &[u8], nonce: &[u8], key: &[u8]) -> Result<Vec<u8>, CryptoError> {
    init();
    let mac = ffi::crypto_secretbox_MACBYTES as usize;
    if key.len() != 32 || nonce.len() != ffi::crypto_secretbox_NONCEBYTES as usize || ct.len() < mac
    {
        return Err(CryptoError::Invalid);
    }
    let mut out = vec![0u8; ct.len() - mac];
    // SAFETY: sizes checked.
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
        return Err(CryptoError::Invalid);
    }
    Ok(out)
}

/// Keyed BLAKE2b (`crypto_generichash`), 32-byte output.
pub fn generichash(msg: &[u8], key: &[u8]) -> Vec<u8> {
    init();
    let mut out = vec![0u8; 32];
    // SAFETY: out is 32 bytes; key length is passed explicitly.
    unsafe {
        ffi::crypto_generichash(
            out.as_mut_ptr(),
            32,
            msg.as_ptr(),
            msg.len() as u64,
            key.as_ptr(),
            key.len(),
        );
    }
    out
}

/// Standard base64 (what the Ente apps send for keys, nonces and ciphertexts).
pub fn b64(data: &[u8]) -> String {
    STANDARD.encode(data)
}

/// Decode standard base64.
pub fn unb64(s: &str) -> Result<Vec<u8>, CryptoError> {
    STANDARD.decode(s).map_err(|_| CryptoError::Invalid)
}

/// URL-safe base64 with padding (museum encodes auth tokens this way).
pub fn b64url(data: &[u8]) -> String {
    URL_SAFE.encode(data)
}

/// Decode URL-safe base64 (with or without padding).
pub fn unb64url(s: &str) -> Result<Vec<u8>, CryptoError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .map_err(|_| CryptoError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_roundtrip() {
        let (pk, sk) = box_keypair();
        let ct = box_seal(b"token", &pk).unwrap();
        assert_eq!(box_seal_open(&ct, &pk, &sk).unwrap(), b"token");
    }

    #[test]
    fn secretbox_roundtrip() {
        let key = random_bytes(32);
        let (ct, n) = secretbox(b"a@b.c", &key).unwrap();
        assert_eq!(secretbox_open(&ct, &n, &key).unwrap(), b"a@b.c");
    }
}
