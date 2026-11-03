//! Photovault plugin host runtime.
//!
//! Plugins are WebAssembly modules (Extism) that run on the user's own
//! devices or compute node, **never on the server**, against the library the
//! app already holds decrypted. This crate is everything the app or compute
//! node needs to run them safely:
//!
//! * [`package`]: the signed `.pvplugin` container (Ed25519 via libsodium),
//!   keys and fingerprints;
//! * [`library`]: the [`Library`] trait the embedder implements over its
//!   decrypted local state (plus an in-memory one behind the `testing`
//!   feature);
//! * [`runtime`]: loading a verified package into a sandbox (memory, time and
//!   fuel limits, no file system, HTTP only to granted hosts) and running its
//!   hooks, with every host call checked against the *current* grants;
//! * [`registry`]: the on-device plugin store with trust-on-first-use
//!   publisher keys, grants, settings and updates;
//! * [`scheduler`]: which scheduled hooks are due, and failure-isolated
//!   dispatch of upload and scheduled hooks.
//!
//! The manifest, permission model and ABI types come from
//! [`photovault_plugin_manifest`], re-exported as [`manifest`].
//!
//! Photovault is based on Ente (ente.com).

#![warn(missing_docs)]

mod crypto;
mod fsutil;
pub mod kv;
pub mod library;
pub mod package;
pub mod registry;
pub mod runtime;
pub mod scheduler;

pub use photovault_plugin_manifest as manifest;

pub use fsutil::{BadPath, MAX_EXPORT_PATH_BYTES, sanitize_export_path};
pub use library::Library;
#[cfg(feature = "testing")]
pub use library::{LibraryWrite, MemoryLibrary};
pub use package::{Package, PackageError, PublicKey, SigningKey};
pub use registry::{
    InstallRequest, InstalledPlugin, KeyStatus, PermissionRequest, Registry, RegistryError,
    UpdateReport,
};
pub use runtime::{
    HostLimits, InstanceEnv, PluginInstance, Policy, PolicySource, ProgressFn, ProgressUpdate,
    RuntimeError, SharedPolicy,
};
pub use scheduler::{Dispatcher, HookRun};
