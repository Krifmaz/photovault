//! Shared helpers for the encrypted-entity endpoint groups (`/user-entity`,
//! `/authenticator`, `/contacts`) and the legacy endpoints: identifiers,
//! request extraction and small schema probes.
//!
//! Every payload these groups store is ciphertext produced by the apps; the
//! server only keeps it, timestamps it and serves diffs.

pub mod http;
pub mod ids;
pub mod schema;
