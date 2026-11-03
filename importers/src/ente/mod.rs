//! Client side of Ente's API: login, collections, encrypted upload and the
//! library index used for de-duplication.
//!
//! STUB: the public surface used by [`crate::run`] is fixed; the rest is
//! filled in by the Ente client work item.

use crate::item::ImportItem;

/// How to bring over people names (Ente's people are on-device face clusters,
/// so only names can be imported).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum PeopleMode {
    /// One album per person, e.g. "Alice".
    #[default]
    Albums,
    /// Append "People: Alice, Bob" to the caption.
    Caption,
    /// Do not import people.
    Skip,
}

/// Options for [`Uploader`].
#[derive(Debug, Clone)]
pub struct UploadOptions {
    /// Album for items that belong to no album in the source.
    pub default_album: String,
    /// What to do with people names.
    pub people: PeopleMode,
    /// Prefix for person albums (e.g. "Person: "), empty by default.
    pub people_album_prefix: String,
    /// Encrypt and prepare everything but do not send anything.
    pub dry_run: bool,
}

/// Login details.
#[derive(Debug, Clone)]
pub struct Credentials {
    /// API endpoint, e.g. `https://photos.example.com`.
    pub endpoint: url::Url,
    /// Account email.
    pub email: String,
    /// Account password.
    pub password: String,
}

/// Result of importing one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadOutcome {
    /// Uploaded as a new Ente file with this id.
    Uploaded(i64),
    /// Already in the library (same content hash); only albums/flags updated.
    Duplicate(i64),
    /// Dry run: nothing sent.
    DryRun,
}

/// Uploads [`ImportItem`]s into the logged-in account.
pub struct Uploader {
    _private: (),
}

impl Uploader {
    /// Log in (SRP, prompting for a TOTP code via `totp` if 2FA is on), load
    /// the user's collections and the content hashes of existing files.
    pub async fn connect(
        creds: Credentials,
        opts: UploadOptions,
        totp: &(dyn Fn() -> anyhow::Result<String> + Sync),
    ) -> anyhow::Result<Self> {
        let _ = (creds, opts, totp);
        todo!()
    }

    /// Encrypt and upload one item, then put it in its albums, favourites,
    /// hidden album and archive state.
    pub async fn import(&mut self, item: &ImportItem) -> anyhow::Result<UploadOutcome> {
        todo!("{item:?}")
    }
}
