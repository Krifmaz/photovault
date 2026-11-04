//! Camera-folder backup (contract v1.2): the album a device folder backs up
//! into, created exactly as Ente's mobile app does
//! (`CollectionsService.getOrCreateForPath`): type `folder`, the name and
//! `attributes.encryptedPath` both the folder name secretbox'd with the new
//! collection key, `attributes.version` 1. An owned `folder` collection with
//! the same name is reused, so a phone that backed up with the Photovault
//! (Ente) app keeps adding to the same albums.

use crate::client::Inner;
use crate::crypto;
use crate::model::{CollectionResponse, RemoteMagicMetadata};
use crate::{Core, CoreError, Result, run};
use reqwest::Method;
use serde::Serialize;

/// Ente's `CollectionType.folder`.
pub(crate) const FOLDER_KIND: &str = "folder";

/// `POST /collections` with device-folder attributes (`ente.CollectionAttributes`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateFolderCollectionRequest {
    encrypted_key: String,
    key_decryption_nonce: String,
    encrypted_name: String,
    name_decryption_nonce: String,
    #[serde(rename = "type")]
    kind: String,
    attributes: FolderAttributes,
    #[serde(skip_serializing_if = "Option::is_none")]
    magic_metadata: Option<RemoteMagicMetadata>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderAttributes {
    encrypted_path: String,
    path_decryption_nonce: String,
    version: i64,
}

/// The folder name as Ente's apps show it: no surrounding whitespace, never empty.
fn folder_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(CoreError::storage("backup folder without a name"));
    }
    Ok(n.to_string())
}

impl Inner {
    pub(crate) async fn backup_album(&self, name: &str) -> Result<i64> {
        let name = folder_name(name)?;
        if let Some(c) = self
            .all_collections()?
            .into_iter()
            .find(|c| c.owned && c.kind == FOLDER_KIND && c.name == name)
        {
            return Ok(c.id);
        }
        let s = self.session()?;
        let api = self.api()?;
        let key = crypto::generate_key();
        let (enc_key, key_nonce) = crypto::secretbox_seal(&key, &s.master_key)?;
        // Ente encrypts the path once and sends it as both the name and the path.
        let (enc_path, path_nonce) = crypto::secretbox_seal(name.as_bytes(), &key)?;
        let (enc_path, path_nonce) = (crypto::b64(&enc_path), crypto::b64(&path_nonce));
        let res: CollectionResponse = api
            .json(
                Method::POST,
                "/collections",
                &CreateFolderCollectionRequest {
                    encrypted_key: crypto::b64(&enc_key),
                    key_decryption_nonce: crypto::b64(&key_nonce),
                    encrypted_name: enc_path.clone(),
                    name_decryption_nonce: path_nonce.clone(),
                    kind: FOLDER_KIND.into(),
                    attributes: FolderAttributes {
                        encrypted_path: enc_path,
                        path_decryption_nonce: path_nonce,
                        version: 1,
                    },
                    magic_metadata: None,
                },
            )
            .await?;
        let rec = crate::library::decrypt_collection(&res.collection, &s)?;
        self.save_collection(&rec)?;
        Ok(rec.id)
    }
}

#[uniffi::export]
impl Core {
    /// [contract v1.2] The album that the device folder `folder_name` backs up
    /// into: the signed-in user's own `folder` collection of that name, created
    /// as Ente's apps create it when missing. Pass the id to `enqueue_upload`.
    pub async fn backup_album(&self, folder_name: String) -> Result<i64, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.backup_album(&folder_name).await }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names() {
        assert_eq!(folder_name("  Camera ").unwrap(), "Camera");
        assert!(folder_name("   ").is_err());
    }

    #[test]
    fn request_matches_ente() {
        let req = CreateFolderCollectionRequest {
            encrypted_key: "k".into(),
            key_decryption_nonce: "kn".into(),
            encrypted_name: "p".into(),
            name_decryption_nonce: "pn".into(),
            kind: FOLDER_KIND.into(),
            attributes: FolderAttributes {
                encrypted_path: "p".into(),
                path_decryption_nonce: "pn".into(),
                version: 1,
            },
            magic_metadata: None,
        };
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            serde_json::json!({
                "encryptedKey": "k", "keyDecryptionNonce": "kn",
                "encryptedName": "p", "nameDecryptionNonce": "pn",
                "type": "folder",
                "attributes": {"encryptedPath": "p", "pathDecryptionNonce": "pn", "version": 1}
            })
        );
    }
}
