//! Ente's encryption scheme, via libsodium (`libsodium-sys-stable`).
//!
//! OWNER: crypto/api agent. Signatures here are the contract other modules
//! compile against; keep them stable.

use zeroize::{Zeroize, ZeroizeOnDrop};

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
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, CryptoError>;

/// A 32-byte symmetric key that is wiped from memory on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey(pub [u8; 32]);

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(..)")
    }
}

/// Output of [`encrypt_blob`]: ciphertext plus secretstream header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedBlob {
    /// Ciphertext (single secretstream message tagged FINAL).
    pub encrypted_data: Vec<u8>,
    /// The 24-byte secretstream header.
    pub decryption_header: Vec<u8>,
}

/// Initialise libsodium. Safe to call many times.
pub fn init() -> Result<()> {
    todo!()
}

/// Ente `encryptBlob`: one secretstream (XChaCha20-Poly1305) message tagged FINAL.
pub fn encrypt_blob(_data: &[u8], _key: &SecretKey) -> Result<EncryptedBlob> {
    todo!()
}

/// Ente `decryptBlob`: inverse of [`encrypt_blob`].
pub fn decrypt_blob(_data: &[u8], _header: &[u8], _key: &SecretKey) -> Result<Vec<u8>> {
    todo!()
}
