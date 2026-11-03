//! Files, collections, sharing, trash and file data: the photo library.
//!
//! Route modules (`routes::{files, file_data, trash, collections, collection_actions}`) are
//! thin HTTP adapters over these repositories, which other workstreams reuse:
//! * [`files`]: file rows, access checks (`can_access_files`, `accessible_object`),
//!   create/update/trash.
//! * [`collections`]: collection rows, roles and `access`, sharing, membership changes,
//!   the file diff.
//! * [`links`]: public album links (`public_collection_tokens`) and file links
//!   (`public_file_tokens`), including password-JWT verification.
//! * [`trash`], [`actions`], [`filedata`]: trash, collection actions, derived file data.
//! * [`objects`], [`usage`]: staged uploads, the work queue, quota checks.
//! * [`model`]: museum's JSON shapes; [`errors`]: museum's error responses;
//!   [`http`]: lenient extractors; [`sql`]: Postgres/SQLite portability helpers;
//!   [`users`]: email lookup and decryption.
//!
//! Call [`spawn_background`] once at startup to run trash expiry, queue processing and
//! object cleanup.

pub mod actions;
pub mod collections;
pub mod errors;
pub mod filedata;
pub mod files;
pub mod http;
pub mod links;
pub mod model;
pub mod objects;
pub mod sql;
pub mod tasks;
pub mod trash;
pub mod usage;
pub mod users;

pub use tasks::spawn_background;
