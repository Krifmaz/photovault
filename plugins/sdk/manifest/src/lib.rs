//! Photovault plugin manifest, permission model and host/guest ABI.
//!
//! This crate is the contract between three parties:
//!
//! * the **plugin author**, who writes `plugin.toml` ([`Manifest`]) and uses the
//!   guest SDK (`photovault-plugin-pdk`), which speaks the [`abi`] types;
//! * the **host runtime** (`photovault-plugin-host`), which runs plugins on the
//!   user's devices or compute node, never on the server;
//! * the **apps**, which show the [`Permission`] list on install and render the
//!   declarative [`settings`] page.
//!
//! It has no Wasm runtime or crypto dependencies, so it compiles both natively
//! and for `wasm32-unknown-unknown`.
//!
//! Photovault is based on Ente (ente.com).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod abi;
mod manifest;
mod permission;
pub mod settings;

pub use manifest::{
    API_VERSION, Hooks, Limits, Manifest, ManifestError, OnDemandAction, PluginInfo, Schedule,
};
pub use permission::{Permission, PermissionSet, ReadScope, WriteScope, valid_host_pattern};
