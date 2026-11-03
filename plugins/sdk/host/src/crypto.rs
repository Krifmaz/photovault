//! Thin safe wrappers over the few libsodium functions plugin signing needs:
//! Ed25519 (`crypto_sign_*`), BLAKE2b (`crypto_generichash`) and secure
//! zeroing. Same library and wrapper style as the server's `crypto` module.

use libsodium_sys as ffi;
use std::sync::Once;

static INIT: Once = Once::new();

/// Length of an Ed25519 public key.
pub const PUBLIC_KEY_BYTES: usize = 32;
/// Length of a libsodium Ed25519 secret key (seed followed by public key).
pub const SECRET_KEY_BYTES: usize = 64;
/// Length of an Ed25519 seed.
pub const SEED_BYTES: usize = 32;
/// Length of an Ed25519 signature.
pub const SIGNATURE_BYTES: usize = 64;

/// Initialise libsodium. Safe to call many times.
pub(crate) fn init() {
    INIT.call_once(|| {
        // SAFETY: sodium_init is thread-safe and idempotent.
        let rc = unsafe { ffi::sodium_init() };
        assert!(rc >= 0, "libsodium failed to initialise");
    });
}

/// BLAKE2b with a 32-byte output and no key (`crypto_generichash`).
pub(crate) fn blake2b_256(data: &[u8]) -> [u8; 32] {
    init();
    let mut out = [0u8; 32];
    // SAFETY: out is 32 bytes, data is valid for data.len() bytes, no key.
    let rc = unsafe {
        ffi::crypto_generichash(
            out.as_mut_ptr(),
            out.len(),
            data.as_ptr(),
            data.len() as u64,
            std::ptr::null(),
            0,
        )
    };
    assert_eq!(rc, 0, "crypto_generichash failed");
    out
}

/// A fresh random Ed25519 key pair: `(public, secret)`.
pub(crate) fn sign_keypair() -> ([u8; PUBLIC_KEY_BYTES], [u8; SECRET_KEY_BYTES]) {
    init();
    let mut pk = [0u8; PUBLIC_KEY_BYTES];
    let mut sk = [0u8; SECRET_KEY_BYTES];
    // SAFETY: buffers have the sizes libsodium expects.
    let rc = unsafe { ffi::crypto_sign_keypair(pk.as_mut_ptr(), sk.as_mut_ptr()) };
    assert_eq!(rc, 0, "crypto_sign_keypair failed");
    (pk, sk)
}

/// The Ed25519 key pair for a 32-byte seed: `(public, secret)`.
pub(crate) fn sign_seed_keypair(
    seed: &[u8; SEED_BYTES],
) -> ([u8; PUBLIC_KEY_BYTES], [u8; SECRET_KEY_BYTES]) {
    init();
    let mut pk = [0u8; PUBLIC_KEY_BYTES];
    let mut sk = [0u8; SECRET_KEY_BYTES];
    // SAFETY: buffers have the sizes libsodium expects.
    let rc =
        unsafe { ffi::crypto_sign_seed_keypair(pk.as_mut_ptr(), sk.as_mut_ptr(), seed.as_ptr()) };
    assert_eq!(rc, 0, "crypto_sign_seed_keypair failed");
    (pk, sk)
}

/// `crypto_sign_detached`: an Ed25519 signature of `msg`.
pub(crate) fn sign_detached(msg: &[u8], sk: &[u8; SECRET_KEY_BYTES]) -> [u8; SIGNATURE_BYTES] {
    init();
    let mut sig = [0u8; SIGNATURE_BYTES];
    let mut len = 0u64;
    // SAFETY: sig is 64 bytes, msg is valid for msg.len() bytes, sk is 64 bytes.
    let rc = unsafe {
        ffi::crypto_sign_detached(
            sig.as_mut_ptr(),
            &mut len,
            msg.as_ptr(),
            msg.len() as u64,
            sk.as_ptr(),
        )
    };
    assert!(
        rc == 0 && len == SIGNATURE_BYTES as u64,
        "crypto_sign_detached failed"
    );
    sig
}

/// `crypto_sign_verify_detached`: whether `sig` is a valid signature of `msg`
/// by `pk`.
pub(crate) fn verify_detached(
    sig: &[u8; SIGNATURE_BYTES],
    msg: &[u8],
    pk: &[u8; PUBLIC_KEY_BYTES],
) -> bool {
    init();
    // SAFETY: sig is 64 bytes, msg is valid for msg.len() bytes, pk is 32 bytes.
    let rc = unsafe {
        ffi::crypto_sign_verify_detached(sig.as_ptr(), msg.as_ptr(), msg.len() as u64, pk.as_ptr())
    };
    rc == 0
}

/// Overwrite `buf` with zeros in a way the compiler cannot optimise away.
pub(crate) fn memzero(buf: &mut [u8]) {
    init();
    // SAFETY: buf is valid for buf.len() bytes.
    unsafe { ffi::sodium_memzero(buf.as_mut_ptr().cast(), buf.len()) };
}

/// Lower-case hex encoding.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}

/// Decode hex (either case). `None` on odd length or a non-hex character.
pub(crate) fn from_hex(s: &str) -> Option<Vec<u8>> {
    fn nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    b.chunks(2)
        .map(|p| Some(nibble(p[0])? << 4 | nibble(p[1])?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake2b_vector() {
        // BLAKE2b-256 of the empty string.
        assert_eq!(
            to_hex(&blake2b_256(b"")),
            "0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8"
        );
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(
            from_hex(&to_hex(&[0, 1, 0xab, 0xff])).unwrap(),
            [0, 1, 0xab, 0xff]
        );
        assert!(from_hex("abc").is_none());
        assert!(from_hex("zz").is_none());
    }
}
