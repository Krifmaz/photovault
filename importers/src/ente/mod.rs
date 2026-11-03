//! Client side of Ente's API: login, collections, encrypted upload and the
//! library index used for de-duplication.
//!
//! The importer is an ordinary Ente client. It speaks Ente's API and uses
//! Ente's encryption scheme unchanged, so it works against Photovault and
//! against Ente's own server alike:
//!
//! * [`auth`]: SRP login (or email code), TOTP, key decryption.
//! * [`collections`]: albums, Favorites, the default hidden collection.
//! * [`library`]: the content-hash index of existing files.
//! * [`upload`]: streaming encryption and (multipart) object upload.
//! * [`metadata`]: file metadata and magic metadata formats.
//!
//! How an [`ImportItem`] maps onto Ente, following what the apps do:
//!
//! * The file is created in its first album (or [`UploadOptions::default_album`])
//!   and added to the others with `POST /collections/add-files`.
//! * Favourite: added to the user's `favorites` collection.
//! * Archived: private magic metadata `{visibility: 1}`, set with
//!   `PUT /files/magic-metadata` after the upload, as the apps archive files.
//! * Hidden: the file goes only into the default hidden collection (the apps'
//!   "hide" moves a file there out of its albums), not into albums or
//!   Favorites.
//! * Description: public magic metadata `caption`; width/height as `w`/`h`.
//! * Live Photo: the apps' zip bundle as file content, `fileType` 2 and
//!   `hash` = `imageHash:videoHash`.
//! * Duplicate (same content hash as a file already in the library): nothing
//!   is uploaded, but the existing file is added to missing albums,
//!   Favourites or the hidden collection, and archived if the item is.

pub mod api;
pub mod auth;
pub mod collections;
pub mod library;
pub mod metadata;
pub mod model;
pub mod upload;

use crate::crypto;
use crate::item::{GeoPoint, ImportItem, MediaKind};
use crate::media;
use anyhow::{Context as _, bail};
use collections::Collections;
use library::{Library, LibraryFile};
use metadata::{FileMetadata, LIVE_PHOTO_HASH_SEPARATOR, MagicMetadata, VISIBILITY_ARCHIVED};
use model::*;
use reqwest::Method;
use std::collections::BTreeSet;
use std::path::Path;

pub use api::RetryPolicy;
pub use auth::Session;

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
    api: api::Api,
    session: Session,
    opts: UploadOptions,
    collections: Collections,
    library: Library,
    objects: upload::ObjectUploader,
}

impl std::fmt::Debug for Uploader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Uploader")
            .field("user_id", &self.session.user_id)
            .field("collections", &self.collections.all().len())
            .field("files", &self.library.len())
            .finish_non_exhaustive()
    }
}

/// Where an item has to end up.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Targets {
    /// Album names, in order; the first is where a new file is created.
    albums: Vec<String>,
    /// Add to Favorites.
    favorite: bool,
    /// Put into the default hidden collection (instead of albums/favorites).
    hidden: bool,
    /// Archive.
    archived: bool,
}

impl Uploader {
    /// Log in (SRP, prompting for a TOTP code via `totp` if 2FA is on, or for
    /// an email code via `otp` if the account has no SRP set up or uses email
    /// 2FA), load the user's collections and the content hashes of existing
    /// files.
    pub async fn connect(
        creds: Credentials,
        opts: UploadOptions,
        totp: &(dyn Fn() -> anyhow::Result<String> + Sync),
        otp: &(dyn Fn() -> anyhow::Result<String> + Sync),
    ) -> anyhow::Result<Self> {
        Self::connect_with_retry(creds, opts, totp, otp, RetryPolicy::default()).await
    }

    /// [`Self::connect`] with a custom retry policy for every request.
    pub async fn connect_with_retry(
        creds: Credentials,
        opts: UploadOptions,
        totp: &(dyn Fn() -> anyhow::Result<String> + Sync),
        otp: &(dyn Fn() -> anyhow::Result<String> + Sync),
        retry: RetryPolicy,
    ) -> anyhow::Result<Self> {
        let mut api = api::Api::new(&creds.endpoint)?;
        api.set_retry_policy(retry);
        let session = auth::login(&api, &creds.email, &creds.password, totp, otp)
            .await
            .context("login failed")?;
        api.set_token(session.token.clone());
        let collections = Collections::load(&api, &session).await?;
        let library = Library::load(&api, &collections, session.user_id).await?;
        Ok(Self {
            api,
            session,
            opts,
            collections,
            library,
            objects: upload::ObjectUploader::new(),
        })
    }

    /// The logged-in account's id.
    pub fn user_id(&self) -> i64 {
        self.session.user_id
    }

    /// Number of files currently in the de-duplication index.
    pub fn library_len(&self) -> usize {
        self.library.len()
    }

    /// Encrypt and upload one item, then put it in its albums, favourites,
    /// hidden album and archive state.
    pub async fn import(&mut self, item: &ImportItem) -> anyhow::Result<UploadOutcome> {
        if item.kind == MediaKind::LivePhoto && item.live_video.is_none() {
            bail!("Live Photo {} has no video part", item.source_id);
        }
        let hash = content_hash(item).await?;
        let targets = targets(item, &self.opts);

        if let Some(existing) = self.library.by_hash(&hash).map(|f| f.id) {
            if self.opts.dry_run {
                tracing::info!(file = %item.primary.file_name, existing, "dry run: duplicate");
                return Ok(UploadOutcome::DryRun);
            }
            self.apply_targets(existing, &targets).await?;
            tracing::debug!(file = %item.primary.file_name, existing, "duplicate");
            return Ok(UploadOutcome::Duplicate(existing));
        }

        let prepared = prepare(item, hash.clone(), &self.opts).await?;
        if self.opts.dry_run {
            // Run the whole encryption pipeline, discarding the output.
            let key = crypto::generate_key();
            let mut r = upload::EncryptingReader::open(prepared.content_path(), &key).await?;
            while r.next_chunk().await?.is_some() {}
            crypto::encrypt_blob(&prepared.thumbnail, &key)?;
            metadata::encrypt_json(&prepared.metadata, &key)?;
            tracing::info!(file = %item.primary.file_name, ?targets, "dry run: would upload");
            return Ok(UploadOutcome::DryRun);
        }

        let collection_ids = self.resolve(&targets).await?;
        let (&first, rest) = collection_ids
            .split_first()
            .context("item has no target collection")?;
        let (id, key) = self.create_file(first, &prepared).await?;
        self.library.insert(
            Some(hash),
            LibraryFile {
                id,
                key,
                file_type: prepared.metadata.file_type,
                collections: BTreeSet::from([first]),
                magic: None,
            },
        );
        for &c in rest {
            self.add_to_collection(id, c).await?;
        }
        if targets.archived {
            self.archive(id).await?;
        }
        tracing::debug!(file = %item.primary.file_name, id, "uploaded");
        Ok(UploadOutcome::Uploaded(id))
    }

    /// Collection ids for `targets`, creating collections as needed.
    async fn resolve(&mut self, targets: &Targets) -> anyhow::Result<Vec<i64>> {
        let (api, session) = (&self.api, &self.session);
        if targets.hidden {
            return Ok(vec![self.collections.default_hidden(api, session).await?]);
        }
        let mut ids = Vec::new();
        for name in &targets.albums {
            let id = self.collections.album(api, session, name).await?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if targets.favorite {
            let id = self.collections.favorites(api, session).await?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// Bring an existing file's collections and archive state up to `targets`.
    async fn apply_targets(&mut self, file_id: i64, targets: &Targets) -> anyhow::Result<()> {
        let ids = self.resolve(targets).await?;
        let have = self
            .library
            .get_mut(file_id)
            .map(|f| f.collections.clone())
            .unwrap_or_default();
        for c in ids {
            if !have.contains(&c) {
                self.add_to_collection(file_id, c).await?;
            }
        }
        let archived = self
            .library
            .get_mut(file_id)
            .and_then(|f| f.magic.as_ref())
            .and_then(|m| m.data.get("visibility"))
            .and_then(serde_json::Value::as_i64)
            == Some(VISIBILITY_ARCHIVED);
        if targets.archived && !archived {
            self.archive(file_id).await?;
        }
        Ok(())
    }

    /// Encrypt and upload the object pieces, then `POST /files`.
    async fn create_file(
        &mut self,
        collection_id: i64,
        p: &Prepared,
    ) -> anyhow::Result<(i64, Vec<u8>)> {
        let collection_key = self
            .collections
            .get(collection_id)
            .context("unknown collection")?
            .key
            .clone();
        let key = crypto::generate_key();
        let file = self
            .objects
            .upload_file(&self.api, p.content_path(), &key)
            .await?;
        let (thumb_ct, thumb_header) = crypto::encrypt_blob(&p.thumbnail, &key)?;
        let thumbnail = self
            .objects
            .upload_blob(&self.api, thumb_ct, &thumb_header)
            .await
            .context("uploading thumbnail")?;
        let (md_data, md_header) = metadata::encrypt_json(&p.metadata, &key)?;
        let pub_magic_metadata = if p.public_magic.is_empty() {
            None
        } else {
            Some(metadata::encrypt_magic(&p.public_magic, 1, &key)?)
        };
        let (enc_key, nonce) = crypto::secretbox_seal(&key, &collection_key)?;
        let res: RemoteFile = self
            .api
            .json(
                Method::POST,
                "/files",
                &CreateFileRequest {
                    collection_id,
                    encrypted_key: crypto::b64(&enc_key),
                    key_decryption_nonce: crypto::b64(&nonce),
                    file: file.attributes(),
                    thumbnail: thumbnail.attributes(),
                    metadata: FileAttributes {
                        object_key: None,
                        encrypted_data: Some(md_data),
                        decryption_header: md_header,
                        size: None,
                    },
                    pub_magic_metadata,
                },
            )
            .await
            .context("creating file")?;
        Ok((res.id, key))
    }

    /// `POST /collections/add-files` for one file.
    async fn add_to_collection(&mut self, file_id: i64, collection_id: i64) -> anyhow::Result<()> {
        let collection_key = self
            .collections
            .get(collection_id)
            .context("unknown collection")?
            .key
            .clone();
        let file = self.library.get_mut(file_id).context("unknown file")?;
        let (enc_key, nonce) = crypto::secretbox_seal(&file.key, &collection_key)?;
        self.api
            .json_no_content(
                Method::POST,
                "/collections/add-files",
                &AddFilesRequest {
                    collection_id,
                    files: vec![CollectionFileItem {
                        id: file_id,
                        encrypted_key: crypto::b64(&enc_key),
                        key_decryption_nonce: crypto::b64(&nonce),
                    }],
                },
            )
            .await
            .with_context(|| format!("adding file {file_id} to collection {collection_id}"))?;
        if let Some(f) = self.library.get_mut(file_id) {
            f.collections.insert(collection_id);
        }
        Ok(())
    }

    /// Set `visibility: 1` in the file's private magic metadata, keeping its
    /// other keys and sending the version currently stored (as
    /// `updateFilesPrivateMagicMetadata` does).
    async fn archive(&mut self, file_id: i64) -> anyhow::Result<()> {
        let file = self.library.get_mut(file_id).context("unknown file")?;
        let current = file.magic.clone().unwrap_or(MagicMetadata {
            version: 1,
            data: Default::default(),
        });
        let mut data = current.data.clone();
        data.insert("visibility".into(), VISIBILITY_ARCHIVED.into());
        let remote = metadata::encrypt_magic(&data, current.version.max(1), &file.key)?;
        self.api
            .json_no_content(
                Method::PUT,
                "/files/magic-metadata",
                &UpdateMagicMetadataRequest {
                    metadata_list: vec![UpdateMagicMetadata {
                        id: file_id,
                        magic_metadata: remote,
                    }],
                },
            )
            .await
            .with_context(|| format!("archiving file {file_id}"))?;
        if let Some(f) = self.library.get_mut(file_id) {
            // The server stores version + 1.
            f.magic = Some(MagicMetadata {
                version: current.version.max(1) + 1,
                data,
            });
        }
        Ok(())
    }
}

/// Work out albums and flags for `item` from the options.
fn targets(item: &ImportItem, opts: &UploadOptions) -> Targets {
    fn push(albums: &mut Vec<String>, name: &str) {
        let name = name.trim();
        if !name.is_empty() && !albums.iter().any(|a| a == name) {
            albums.push(name.to_string());
        }
    }
    let mut albums: Vec<String> = Vec::new();
    for a in &item.albums {
        push(&mut albums, a);
    }
    if opts.people == PeopleMode::Albums {
        for p in &item.people {
            if !p.trim().is_empty() {
                push(
                    &mut albums,
                    &format!("{}{}", opts.people_album_prefix, p.trim()),
                );
            }
        }
    }
    if albums.is_empty() {
        push(&mut albums, &opts.default_album);
    }
    Targets {
        albums,
        favorite: item.favorite,
        hidden: item.hidden,
        archived: item.archived,
    }
}

/// Everything about an item that does not depend on keys.
struct Prepared {
    metadata: FileMetadata,
    public_magic: serde_json::Map<String, serde_json::Value>,
    thumbnail: Vec<u8>,
    /// For Live Photos, the zip bundle (deleted on drop).
    bundle: Option<tempfile::NamedTempFile>,
    primary: std::path::PathBuf,
}

impl Prepared {
    /// What to upload as the file content.
    fn content_path(&self) -> &Path {
        self.bundle
            .as_ref()
            .map_or(self.primary.as_path(), |b| b.path())
    }
}

/// Ente's content hash of an item (`imageHash:videoHash` for Live Photos).
async fn content_hash(item: &ImportItem) -> anyhow::Result<String> {
    let primary = item.primary.path.clone();
    let video = item.live_video.as_ref().map(|v| v.path.clone());
    let kind = item.kind;
    tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let h = media::hash_file(&primary)?;
        match (kind, video) {
            (MediaKind::LivePhoto, Some(v)) => Ok(format!(
                "{h}{LIVE_PHOTO_HASH_SEPARATOR}{}",
                media::hash_file(&v)?
            )),
            _ => Ok(h),
        }
    })
    .await?
    .with_context(|| format!("hashing {}", item.primary.path.display()))
}

/// Metadata, thumbnail and (for Live Photos) the bundle of an item.
async fn prepare(
    item: &ImportItem,
    hash: String,
    opts: &UploadOptions,
) -> anyhow::Result<Prepared> {
    let mtime_us = file_mtime_us(&item.primary.path).await;

    let need_embedded =
        item.creation_time_us.is_none() || item.location.is_none() || item.dimensions.is_none();
    let embedded = if need_embedded {
        let p = item.primary.path.clone();
        tokio::task::spawn_blocking(move || media::read_embedded_info(&p))
            .await
            .unwrap_or_default()
    } else {
        media::EmbeddedInfo::default()
    };

    let modification_time = item.modification_time_us.or(mtime_us).unwrap_or(0);
    let creation_time = item
        .creation_time_us
        .or(embedded.creation_time_us)
        .unwrap_or(modification_time);
    let location: Option<GeoPoint> = item
        .location
        .or(embedded.location)
        .filter(|l| l.latitude.is_finite() && l.longitude.is_finite());
    let dimensions = item.dimensions.or(embedded.dimensions);

    let file_type = match item.kind {
        MediaKind::Image => metadata::FILE_TYPE_IMAGE,
        MediaKind::Video => metadata::FILE_TYPE_VIDEO,
        MediaKind::LivePhoto => metadata::FILE_TYPE_LIVE_PHOTO,
    };
    let md = FileMetadata {
        file_type,
        title: item.primary.file_name.clone(),
        creation_time,
        modification_time,
        hash: Some(hash),
        duration: None,
        latitude: location.map(|l| l.latitude),
        longitude: location.map(|l| l.longitude),
    };

    let caption = caption(item, opts);
    let public_magic = metadata::public_magic_data(dimensions, caption.as_deref());

    let thumb_kind = match item.kind {
        MediaKind::LivePhoto => MediaKind::Image,
        k => k,
    };
    let thumbnail =
        media::make_thumbnail(&item.primary, thumb_kind, item.thumbnail.as_deref()).await;

    let bundle = match (&item.kind, &item.live_video) {
        (MediaKind::LivePhoto, Some(video)) => {
            let (image, video) = (item.primary.clone(), video.clone());
            Some(
                tokio::task::spawn_blocking(move || media::live_photo_bundle(&image, &video))
                    .await?
                    .context("bundling Live Photo")?,
            )
        }
        _ => None,
    };

    Ok(Prepared {
        metadata: md,
        public_magic,
        thumbnail,
        bundle,
        primary: item.primary.path.clone(),
    })
}

/// The caption: the description, plus "People: …" in [`PeopleMode::Caption`].
fn caption(item: &ImportItem, opts: &UploadOptions) -> Option<String> {
    let desc = item
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let people: Vec<&str> = item
        .people
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    let people_line = (opts.people == PeopleMode::Caption && !people.is_empty())
        .then(|| format!("People: {}", people.join(", ")));
    match (desc, people_line) {
        (Some(d), Some(p)) => Some(format!("{d}\n{p}")),
        (Some(d), None) => Some(d.to_string()),
        (None, p) => p,
    }
}

async fn file_mtime_us(path: &Path) -> Option<i64> {
    let m = tokio::fs::metadata(path).await.ok()?.modified().ok()?;
    let d = m.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(d.as_micros()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::MediaFile;

    fn opts(people: PeopleMode) -> UploadOptions {
        UploadOptions {
            default_album: "Imported".into(),
            people,
            people_album_prefix: "Person: ".into(),
            dry_run: false,
        }
    }

    fn item() -> ImportItem {
        ImportItem::new(
            "id",
            MediaKind::Image,
            MediaFile {
                path: "/nonexistent".into(),
                file_name: "a.jpg".into(),
            },
        )
    }

    #[test]
    fn caption_combines_description_and_people() {
        let mut i = item();
        assert_eq!(caption(&i, &opts(PeopleMode::Caption)), None);
        i.people = vec!["Alice".into(), " Bob ".into()];
        assert_eq!(
            caption(&i, &opts(PeopleMode::Caption)).as_deref(),
            Some("People: Alice, Bob")
        );
        assert_eq!(caption(&i, &opts(PeopleMode::Albums)), None);
        i.description = Some("Beach".into());
        assert_eq!(
            caption(&i, &opts(PeopleMode::Caption)).as_deref(),
            Some("Beach\nPeople: Alice, Bob")
        );
        assert_eq!(
            caption(&i, &opts(PeopleMode::Skip)).as_deref(),
            Some("Beach")
        );
    }

    #[test]
    fn targets_from_albums_people_and_default() {
        let mut i = item();
        let t = targets(&i, &opts(PeopleMode::Albums));
        assert_eq!(t.albums, vec!["Imported".to_string()]);
        i.albums = vec!["Trip".into(), " Trip ".into(), "".into()];
        i.people = vec!["Alice".into()];
        i.favorite = true;
        i.archived = true;
        let t = targets(&i, &opts(PeopleMode::Albums));
        assert_eq!(
            t.albums,
            vec!["Trip".to_string(), "Person: Alice".to_string()]
        );
        assert!(t.favorite && t.archived && !t.hidden);
        let t = targets(&i, &opts(PeopleMode::Caption));
        assert_eq!(t.albums, vec!["Trip".to_string()]);
    }
}
