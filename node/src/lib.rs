//! Photovault compute node.
//!
//! An optional program that runs on a machine the user owns (for example a
//! desktop with a GPU). It signs in to the user's account like any other
//! device, walks the whole library, and uploads derived data **in Ente's own
//! encrypted formats** so that stock Ente apps and Photovault forks pick it up
//! without any change:
//!
//! - `mldata`: gzipped JSON with face and CLIP indexes, encrypted with the
//!   file key as a libsodium secretstream blob ([`ml`]).
//! - `vid_preview`: an encrypted HLS stream plus encrypted playlist JSON
//!   ([`video`]).
//!
//! The node pauses while the machine is busy ([`schedule`]). Without a node,
//! the apps index everything themselves, exactly as upstream.
//!
//! Based on Ente (ente.com).

pub mod api;
pub mod backend;
pub mod cli;
pub mod config;
pub mod crypto;
pub mod library;
pub mod ml;
pub mod schedule;
pub mod state;
pub mod video;
pub mod worker;
