//! SRP-6a client, wire-compatible with Ente (4096-bit group, SHA-256, the
//! `fast-srp-hap` / `go-srp` conventions Ente uses).
//!
//! STUB: filled in by the crypto work item.

/// An in-progress SRP login.
pub struct SrpClient {
    _private: (),
}

impl SrpClient {
    /// Start a session. `identity` is the `srpUserID` (UUID string), `salt`
    /// the decoded `srpSalt`, `password` the 16-byte login sub-key from
    /// [`crate::crypto::derive_login_key`].
    pub fn new(identity: &[u8], salt: &[u8], password: &[u8]) -> Self {
        todo!("{identity:?}{salt:?}{password:?}")
    }

    /// The public ephemeral `A`, big-endian, padded to the group size.
    pub fn public_a(&self) -> Vec<u8> {
        todo!()
    }

    /// Process the server's `B` and return the client proof `M1`.
    pub fn compute_m1(&mut self, server_b: &[u8]) -> Result<Vec<u8>, crate::crypto::CryptoError> {
        todo!("{server_b:?}")
    }

    /// Check the server proof `M2` (call after [`Self::compute_m1`]).
    pub fn verify_m2(&self, m2: &[u8]) -> Result<(), crate::crypto::CryptoError> {
        todo!("{m2:?}")
    }
}
