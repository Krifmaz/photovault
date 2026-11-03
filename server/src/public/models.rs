//! JSON shapes of the public-link and cast endpoints. Files and collections are the
//! library's [`crate::library::model`] types (museum's `ente.File`, `ente.Collection`,
//! `ente.PublicURL`); this module adds the public-diff post-processing and museum's
//! `ente.MemoryShare`.

use serde::Serialize;

pub use crate::library::model::{
    Collection, CollectionAttributes, CollectionUser, File, FileAttributes, FileInfo,
    MagicMetadata, PublicUrl,
};

fn is_zero(v: &i64) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// museum `GetPublicDiff`/`GetCastDiff` post-processing for one entry: strip private
/// metadata and pending actions (a pending `REMOVE`/`DELETE_SUGGESTED` reads as deleted),
/// and scrub deleted entries.
pub fn for_public_diff(mut f: File) -> File {
    f.magic_metadata = None;
    if let Some(a) = &f.action
        && !f.is_deleted
        && (a == "REMOVE" || a == "DELETE_SUGGESTED")
    {
        f.is_deleted = true;
    }
    f.action = None;
    f.action_user = None;
    if f.metadata.encrypted_data == "-" && !f.is_deleted {
        tracing::warn!(file_id = f.id, "stale collection_file found");
        f.is_deleted = true;
    }
    crate::library::collections::scrub_deleted(f)
}

/// The limited `publicURLs` form museum's `GetPublicCollection` exposes to link viewers.
pub fn limited_public_urls(urls: Option<Vec<PublicUrl>>) -> Vec<PublicUrl> {
    urls.unwrap_or_default()
        .into_iter()
        .map(|u| PublicUrl {
            enable_download: u.enable_download,
            enable_collect: u.enable_collect,
            enable_comment: u.enable_comment,
            password_enabled: u.password_enabled,
            nonce: u.nonce,
            mem_limit: u.mem_limit,
            ops_limit: u.ops_limit,
            enable_join: u.enable_join,
            ..Default::default()
        })
        .collect()
}

/// museum `ente.MemoryShare`.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryShare {
    /// Share id.
    pub id: i64,
    /// Owner (never serialised).
    #[serde(skip)]
    pub user_id: i64,
    /// `share` or `lane`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Client-side memory identifier hash.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub memory_hash: String,
    /// Encrypted metadata.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub metadata_cipher: String,
    /// Nonce for metadata.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub metadata_nonce: String,
    /// Share key, encrypted.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub encrypted_key: String,
    /// Nonce for the share key.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key_decryption_nonce: String,
    /// Public access token.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub access_token: String,
    /// Deleted.
    #[serde(skip_serializing_if = "is_false")]
    pub is_deleted: bool,
    /// Created, microseconds.
    pub created_at: i64,
    /// Updated, microseconds.
    #[serde(skip_serializing_if = "is_zero")]
    pub updated_at: i64,
    /// Public URL.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubbed_file_shape() {
        let f = File {
            id: 3,
            owner_id: 1,
            collection_id: 2,
            encrypted_key: "k".into(),
            is_deleted: true,
            updation_time: 9,
            ..Default::default()
        };
        let v = serde_json::to_value(for_public_diff(f)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"id":3,"ownerID":1,"collectionID":2,"collectionOwnerID":null,
                "encryptedKey":"","keyDecryptionNonce":"",
                "file":{"decryptionHeader":"","size":0},"thumbnail":{"decryptionHeader":"","size":0},
                "metadata":{"encryptedData":"-","decryptionHeader":"","size":0},
                "isDeleted":true,"updationTime":9})
        );
    }
}
