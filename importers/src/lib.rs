//! Photovault importers.
//!
//! Import an [Immich](https://immich.app) library or an iCloud Photos export
//! into Photovault (or any server that speaks Ente's API). Every byte that
//! leaves the importer is encrypted on this machine with Ente's own scheme
//! (libsodium: Argon2id, SRP, per-file and per-album keys, encrypted metadata),
//! so the server only ever sees ciphertext.
//!
//! Based on Ente (ente.com).
//!
//! # Layout
//! - [`crypto`] and [`srp`]: libsodium wrappers and the SRP-6a client.
//! - [`ente`]: Ente client API (login, collections, upload, dedup).
//! - [`media`]: file type detection, hashing, thumbnails, Live Photo bundles.
//! - [`item`]: the source-neutral [`item::ImportItem`] every importer yields.
//! - [`immich`] and [`icloud`]: the two sources.
//! - [`state`]: the resume journal, so re-running an import is safe.
//! - [`run`]: the pipeline that feeds items from a source into the uploader.

#![warn(missing_docs)]

pub mod crypto;
pub mod ente;
pub mod icloud;
pub mod immich;
pub mod item;
pub mod media;
pub mod run;
pub mod srp;
pub mod state;
pub mod tls;
