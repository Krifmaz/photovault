//! The user's collections: listing, decryption, and find-or-create of albums,
//! the Favorites collection and the default hidden collection, the way
//! `web/packages/new/photos/services/collection.ts` does it.
//!
//! * Albums: type `album`, name secretbox'd with the collection key, the
//!   collection key secretbox'd with the master key.
//! * Favorites: the user's one collection of type `favorites`, named
//!   "Favorites" when the importer has to create it.
//! * Hidden: the default hidden collection is an `album` named `.hidden`
//!   whose private magic metadata is `{subType: 1, visibility: 2}`. When
//!   several exist (concurrent clients), the one with the lowest id is used.

use super::api::Api;
use super::auth::Session;
use super::metadata::{SUB_TYPE_DEFAULT_HIDDEN, VISIBILITY_HIDDEN, decrypt_magic, encrypt_magic};
use super::model::*;
use crate::crypto;
use anyhow::Context as _;
use reqwest::Method;
use serde_json::{Map, Value};

/// Name the apps give the Favorites collection.
pub const FAVORITES_NAME: &str = "Favorites";
/// Name the apps give the default hidden collection.
pub const DEFAULT_HIDDEN_NAME: &str = ".hidden";

/// A decrypted collection owned by the user.
#[derive(Debug, Clone)]
pub struct Collection {
    /// Collection id.
    pub id: i64,
    /// Decrypted collection key.
    pub key: Vec<u8>,
    /// Decrypted name.
    pub name: String,
    /// `album`, `folder`, `favorites` or `uncategorized`.
    pub kind: String,
    /// Private magic metadata `subType`.
    pub sub_type: Option<i64>,
    /// Private magic metadata `visibility`.
    pub visibility: Option<i64>,
}

impl Collection {
    /// Whether this is a default hidden collection.
    pub fn is_default_hidden(&self) -> bool {
        self.sub_type == Some(SUB_TYPE_DEFAULT_HIDDEN)
    }
}

/// The user's own collections.
#[derive(Debug, Default)]
pub struct Collections {
    owned: Vec<Collection>,
}

impl Collections {
    /// Fetch and decrypt every collection the user owns. Collections shared
    /// with the user are skipped: the importer never adds to them.
    pub async fn load(api: &Api, session: &Session) -> anyhow::Result<Self> {
        let res: CollectionsResponse = api
            .get_json("/collections/v2", &[("sinceTime", "0".to_string())])
            .await
            .context("listing collections")?;
        let mut owned = Vec::new();
        for c in res.collections {
            if c.is_deleted || c.owner.id != session.user_id {
                continue;
            }
            match decrypt_collection(&c, &session.master_key) {
                Ok(c) => owned.push(c),
                Err(e) => {
                    tracing::warn!(collection_id = c.id, error = %format!("{e:#}"), "skipping collection that does not decrypt")
                }
            }
        }
        owned.sort_by_key(|c| c.id);
        tracing::info!(count = owned.len(), "loaded collections");
        Ok(Self { owned })
    }

    /// All owned collections, by ascending id.
    pub fn all(&self) -> &[Collection] {
        &self.owned
    }

    /// The collection with this id.
    pub fn get(&self, id: i64) -> Option<&Collection> {
        self.owned.iter().find(|c| c.id == id)
    }

    /// An existing album with exactly this name. Visible albums win over
    /// hidden ones; among equals the oldest (lowest id) wins.
    pub fn find_album(&self, name: &str) -> Option<&Collection> {
        let mut candidates: Vec<&Collection> = self
            .owned
            .iter()
            .filter(|c| c.kind == "album" && !c.is_default_hidden() && c.name == name)
            .collect();
        candidates.sort_by_key(|c| (c.visibility == Some(VISIBILITY_HIDDEN), c.id));
        candidates.first().copied()
    }

    /// Find or create the album `name`; returns its id.
    pub async fn album(&mut self, api: &Api, session: &Session, name: &str) -> anyhow::Result<i64> {
        if let Some(c) = self.find_album(name) {
            return Ok(c.id);
        }
        tracing::info!(album = name, "creating album");
        self.create(api, session, name, "album", None).await
    }

    /// Find or create the user's Favorites collection; returns its id.
    pub async fn favorites(&mut self, api: &Api, session: &Session) -> anyhow::Result<i64> {
        if let Some(c) = self.owned.iter().find(|c| c.kind == "favorites") {
            return Ok(c.id);
        }
        tracing::info!("creating Favorites collection");
        self.create(api, session, FAVORITES_NAME, "favorites", None)
            .await
    }

    /// Find or create the default hidden collection; returns its id.
    pub async fn default_hidden(&mut self, api: &Api, session: &Session) -> anyhow::Result<i64> {
        if let Some(c) = self.owned.iter().find(|c| c.is_default_hidden()) {
            return Ok(c.id);
        }
        tracing::info!("creating default hidden collection");
        let mut mm = Map::new();
        mm.insert("subType".into(), SUB_TYPE_DEFAULT_HIDDEN.into());
        mm.insert("visibility".into(), VISIBILITY_HIDDEN.into());
        self.create(api, session, DEFAULT_HIDDEN_NAME, "album", Some(mm))
            .await
    }

    async fn create(
        &mut self,
        api: &Api,
        session: &Session,
        name: &str,
        kind: &str,
        magic: Option<Map<String, Value>>,
    ) -> anyhow::Result<i64> {
        let key = crypto::generate_key();
        let (enc_key, key_nonce) = crypto::secretbox_seal(&key, &session.master_key)?;
        let (enc_name, name_nonce) = crypto::secretbox_seal(name.as_bytes(), &key)?;
        let magic_metadata = magic
            .as_ref()
            .map(|m| encrypt_magic(m, 1, &key))
            .transpose()?;
        let res: CollectionResponse = api
            .json(
                Method::POST,
                "/collections",
                &CreateCollectionRequest {
                    encrypted_key: crypto::b64(&enc_key),
                    key_decryption_nonce: crypto::b64(&key_nonce),
                    encrypted_name: crypto::b64(&enc_name),
                    name_decryption_nonce: crypto::b64(&name_nonce),
                    kind: kind.to_string(),
                    magic_metadata,
                },
            )
            .await
            .with_context(|| format!("creating collection {name:?}"))?;
        let id = res.collection.id;
        let get = |k: &str| {
            magic
                .as_ref()
                .and_then(|m| m.get(k))
                .and_then(Value::as_i64)
        };
        self.owned.push(Collection {
            id,
            key,
            name: name.to_string(),
            kind: kind.to_string(),
            sub_type: get("subType"),
            visibility: get("visibility"),
        });
        Ok(id)
    }
}

/// Decrypt an owned collection with the master key.
pub fn decrypt_collection(c: &RemoteCollection, master_key: &[u8]) -> anyhow::Result<Collection> {
    let nonce = c
        .key_decryption_nonce
        .as_deref()
        .context("owned collection without keyDecryptionNonce")?;
    let key = crypto::secretbox_open(
        &crypto::unb64(&c.encrypted_key)?,
        &crypto::unb64(nonce)?,
        master_key,
    )
    .context("decrypting collection key")?;
    let name = match (&c.name, &c.encrypted_name, &c.name_decryption_nonce) {
        (Some(n), _, _) if !n.is_empty() => n.clone(),
        (_, Some(en), Some(nn)) => String::from_utf8_lossy(
            &crypto::secretbox_open(&crypto::unb64(en)?, &crypto::unb64(nn)?, &key)
                .context("decrypting collection name")?,
        )
        .into_owned(),
        _ => String::new(),
    };
    let (sub_type, visibility) = match &c.magic_metadata {
        Some(mm) => {
            let m = decrypt_magic(mm, &key).context("decrypting collection magic metadata")?;
            (
                m.data.get("subType").and_then(Value::as_i64),
                m.data.get("visibility").and_then(Value::as_i64),
            )
        }
        None => (None, None),
    };
    Ok(Collection {
        id: c.id,
        key,
        name,
        kind: c.kind.clone(),
        sub_type,
        visibility,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(id: i64, name: &str, kind: &str, sub: Option<i64>, vis: Option<i64>) -> Collection {
        Collection {
            id,
            key: vec![],
            name: name.into(),
            kind: kind.into(),
            sub_type: sub,
            visibility: vis,
        }
    }

    #[test]
    fn find_album_prefers_visible_then_oldest() {
        let c = Collections {
            owned: vec![
                col(1, "Trip", "album", None, Some(VISIBILITY_HIDDEN)),
                col(5, "Trip", "album", None, None),
                col(3, "Trip", "album", None, Some(1)),
                col(2, "Trip", "folder", None, None),
                col(4, ".hidden", "album", Some(1), Some(2)),
            ],
        };
        assert_eq!(c.find_album("Trip").map(|c| c.id), Some(3));
        assert!(c.find_album(".hidden").is_none());
        assert!(c.find_album("trip").is_none());
    }
}
