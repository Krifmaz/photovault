//! Guest SDK for Photovault WebAssembly plugins.
//!
//! Photovault plugins are [Extism](https://extism.org) modules that run on the
//! user's own devices or compute node, **never on the server**. They see
//! decrypted metadata (and, if granted, thumbnails or originals) through a
//! small host API, and every call is checked against the permissions the user
//! approved when installing the plugin.
//!
//! This crate is the Rust side of that API:
//!
//! * [`host`]: typed functions for every host call (`list_files`,
//!   [`host::files`] to walk the whole library, `ensure_album`,
//!   `add_to_album`, `trash_files`, `export_file`, `kv_get`/`kv_set`,
//!   `progress`, `read_thumbnail`, `read_original`, ...);
//! * [`photovault_plugin!`]: exports your hook functions as `on_upload`,
//!   `on_schedule` and `on_demand`;
//! * [`HookInputExt`]: typed settings and event helpers;
//! * [`mod@log`] macros ([`info!`], [`warn!`], ...), [`http`] for plugins granted
//!   network hosts, [`path`] for safe export paths, [`time`] for Ente's
//!   microsecond timestamps;
//! * `testing` (native targets only): a `testing::MockHost` so plugin logic
//!   is unit-tested with plain `cargo test`.
//!
//! The wire types live in the [`abi`] module (re-exported from
//! `photovault-plugin-manifest`). Plugins in other languages can implement
//! the same ABI with any Extism PDK.
//!
//! ```
//! use photovault_plugin_pdk::prelude::*;
//!
//! fn tag_videos(input: &HookInput) -> Result<HookOutput> {
//!     let Some(file) = input.uploaded_file()? else { return Ok(HookOutput::default()) };
//!     if file.file_type != FileType::Video {
//!         return Ok(HookOutput::default());
//!     }
//!     pdk::host::add_tags(file.id, &["video"])?;
//!     Ok(HookOutput { summary: Some(format!("Tagged {}", file.title)), changed: 1 })
//! }
//!
//! photovault_plugin_pdk::photovault_plugin! { on_upload: tag_videos }
//! ```
//!
//! Photovault is based on Ente (ente.com).

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod backend;
pub mod hook;
pub mod host;
pub mod http;
pub mod log;
pub mod path;
pub mod time;

#[cfg(not(target_arch = "wasm32"))]
pub mod testing;

/// The manifest crate, for `plugin.toml` parsing and permissions.
pub use photovault_plugin_manifest as manifest;
/// Host/guest wire types (requests, responses, hook input and output).
pub use photovault_plugin_manifest::abi;
/// Declarative settings fields and values.
pub use photovault_plugin_manifest::settings;
pub use photovault_plugin_manifest::{
    API_VERSION, Manifest, Permission, PermissionSet, ReadScope, WriteScope,
};

pub use hook::{HookFn, HookInputExt};

/// The error type of hook functions: any error converts into it with `?`
/// (it is [`anyhow::Error`]). Host failures arrive as [`abi::HostError`],
/// which tests can recover with `err.downcast_ref::<HostError>()`.
pub type Error = anyhow::Error;

/// `Result<T, Error>`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

pub use anyhow::{Context, anyhow, bail, ensure};

/// Everything a typical plugin needs: `use photovault_plugin_pdk::prelude::*;`.
pub mod prelude {
    pub use crate as pdk;
    pub use crate::abi::{
        AlbumId, AlbumInfo, AlbumKind, ErrorCode, ExportResult, FileId, FileInfo, FileQuery,
        FileType, HookEvent, HookInput, HookOutput, HostError, Runner,
    };
    pub use crate::{Context, Error, HookInputExt, Result, anyhow, bail, ensure};
}
