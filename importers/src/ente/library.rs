//! The library index used for de-duplication: every file the user owns, by
//! content hash, with the collections it is in and its private magic
//! metadata. Built from `GET /collections/v2/diff` for every owned
//! collection, as the apps sync (`pullCollectionFiles`).

use super::api::Api;
use super::collections::Collections;
use super::metadata::{MagicMetadata, decrypt_json, decrypt_magic, metadata_hash};
use super::model::DiffResponse;
use crate::crypto;
use anyhow::Context as _;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// One file of the library.
#[derive(Debug, Clone)]
pub struct LibraryFile {
    /// File id.
    pub id: i64,
    /// Decrypted file key.
    pub key: Vec<u8>,
    /// Ente `fileType`.
    pub file_type: i64,
    /// Owned collections the file is in.
    pub collections: BTreeSet<i64>,
    /// Private magic metadata (visibility), if any.
    pub magic: Option<MagicMetadata>,
}

/// Index of the user's files by content hash.
#[derive(Debug, Default)]
pub struct Library {
    files: HashMap<i64, LibraryFile>,
    by_hash: HashMap<String, i64>,
}

impl Library {
    /// Page through the diff of every owned collection and index the files
    /// the user owns. Files that fail to decrypt are logged and skipped.
    pub async fn load(api: &Api, collections: &Collections, user_id: i64) -> anyhow::Result<Self> {
        let mut lib = Self::default();
        for c in collections.all() {
            let mut since = 0i64;
            loop {
                let page: DiffResponse = api
                    .get_json(
                        "/collections/v2/diff",
                        &[
                            ("collectionID", c.id.to_string()),
                            ("sinceTime", since.to_string()),
                        ],
                    )
                    .await
                    .with_context(|| format!("fetching files of collection {}", c.id))?;
                if page.diff.is_empty() {
                    break;
                }
                for f in &page.diff {
                    since = since.max(f.updation_time);
                    if f.is_deleted {
                        if let Some(lf) = lib.files.get_mut(&f.id) {
                            lf.collections.remove(&c.id);
                        }
                        continue;
                    }
                    if f.owner_id != user_id {
                        continue;
                    }
                    if let Some(lf) = lib.files.get_mut(&f.id) {
                        lf.collections.insert(c.id);
                        continue;
                    }
                    match decrypt_entry(f, &c.key) {
                        Ok((key, metadata, magic)) => {
                            let file_type = metadata
                                .get("fileType")
                                .and_then(Value::as_i64)
                                .unwrap_or(-1);
                            let lf = LibraryFile {
                                id: f.id,
                                key,
                                file_type,
                                collections: BTreeSet::from([c.id]),
                                magic,
                            };
                            lib.insert(metadata_hash(&metadata), lf);
                        }
                        Err(e) => {
                            tracing::warn!(file_id = f.id, collection_id = c.id, error = %format!("{e:#}"), "skipping file that does not decrypt")
                        }
                    }
                }
                if !page.has_more {
                    break;
                }
            }
        }
        // A file that left every collection (e.g. moved to trash) is gone.
        let gone: Vec<i64> = lib
            .files
            .values()
            .filter(|f| f.collections.is_empty())
            .map(|f| f.id)
            .collect();
        for id in gone {
            lib.files.remove(&id);
        }
        lib.by_hash.retain(|_, id| lib.files.contains_key(id));
        tracing::info!(
            files = lib.files.len(),
            hashes = lib.by_hash.len(),
            "indexed library"
        );
        Ok(lib)
    }

    /// Add a file; the first file seen for a hash keeps it.
    pub fn insert(&mut self, hash: Option<String>, file: LibraryFile) {
        if let Some(h) = hash {
            self.by_hash.entry(h).or_insert(file.id);
        }
        self.files.insert(file.id, file);
    }

    /// The file with content hash `hash`.
    pub fn by_hash(&self, hash: &str) -> Option<&LibraryFile> {
        self.by_hash.get(hash).and_then(|id| self.files.get(id))
    }

    /// Mutable access by file id.
    pub fn get_mut(&mut self, id: i64) -> Option<&mut LibraryFile> {
        self.files.get_mut(&id)
    }

    /// Number of indexed files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

type Decrypted = (Vec<u8>, Value, Option<MagicMetadata>);

fn decrypt_entry(f: &super::model::RemoteFile, collection_key: &[u8]) -> anyhow::Result<Decrypted> {
    let key = crypto::secretbox_open(
        &crypto::unb64(&f.encrypted_key)?,
        &crypto::unb64(&f.key_decryption_nonce)?,
        collection_key,
    )
    .context("decrypting file key")?;
    let md = f.metadata.as_ref().context("file without metadata")?;
    let metadata = decrypt_json(
        md.encrypted_data.as_deref().unwrap_or_default(),
        &md.decryption_header,
        &key,
    )
    .context("decrypting file metadata")?;
    let magic = f
        .magic_metadata
        .as_ref()
        .map(|m| decrypt_magic(m, &key))
        .transpose()
        .context("decrypting file magic metadata")?;
    Ok((key, metadata, magic))
}
