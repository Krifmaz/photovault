//! JSON shapes shared by the public-link and cast endpoints, serialised exactly
//! like museum's `ente.File`, `ente.Collection`, `ente.PublicURL` and `ente.MemoryShare`
//! (field names, `omitempty` and `null` behaviour included).

use serde::{Deserialize, Serialize};

fn is_zero(v: &i64) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// museum `ente.MagicMetadata` (stored as JSON text).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MagicMetadata {
    /// Version.
    #[serde(skip_serializing_if = "is_zero")]
    pub version: i64,
    /// Number of keys.
    #[serde(skip_serializing_if = "is_zero")]
    pub count: i64,
    /// Encrypted payload.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub data: String,
    /// Decryption header.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub header: String,
}

impl MagicMetadata {
    /// Parse a stored JSON column (NULL/invalid → `None`).
    pub fn from_db(s: Option<String>) -> Option<Self> {
        s.and_then(|s| serde_json::from_str(&s).ok())
    }
}

/// museum `ente.FileInfo`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FileInfo {
    /// Size of the file object.
    #[serde(rename = "fileSize", skip_serializing_if = "is_zero")]
    pub file_size: i64,
    /// Size of the thumbnail object.
    #[serde(rename = "thumbSize", skip_serializing_if = "is_zero")]
    pub thumb_size: i64,
}

impl FileInfo {
    /// Parse a stored JSON column.
    pub fn from_db(s: Option<String>) -> Option<Self> {
        s.and_then(|s| serde_json::from_str(&s).ok())
    }
}

/// museum `ente.FileAttributes`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct FileAttributes {
    /// Object key (only in upload requests/responses).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub object_key: String,
    /// Inline encrypted data (metadata).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub encrypted_data: String,
    /// Decryption header.
    pub decryption_header: String,
    /// Size in bytes.
    pub size: i64,
}

/// museum `ente.File`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct File {
    /// File id.
    pub id: i64,
    /// Owner.
    #[serde(rename = "ownerID")]
    pub owner_id: i64,
    /// Collection (for diff rows).
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// Collection owner (`null` when unknown).
    #[serde(rename = "collectionOwnerID")]
    pub collection_owner_id: Option<i64>,
    /// When the file was added to the collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection_added_at: Option<i64>,
    /// File key encrypted with the collection key.
    pub encrypted_key: String,
    /// Nonce for `encrypted_key`.
    pub key_decryption_nonce: String,
    /// File object attributes.
    pub file: FileAttributes,
    /// Thumbnail object attributes.
    pub thumbnail: FileAttributes,
    /// Metadata attributes.
    pub metadata: FileAttributes,
    /// Removed from the collection.
    pub is_deleted: bool,
    /// Version (microseconds).
    pub updation_time: i64,
    /// Private magic metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magic_metadata: Option<MagicMetadata>,
    /// Public magic metadata.
    #[serde(rename = "pubMagicMetadata", skip_serializing_if = "Option::is_none")]
    pub pub_magic_metadata: Option<MagicMetadata>,
    /// Object sizes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<FileInfo>,
    /// Pending collection action (stripped from public/cast diffs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// User who triggered `action`.
    #[serde(rename = "actionUser", skip_serializing_if = "Option::is_none")]
    pub action_user: Option<i64>,
}

impl File {
    /// museum `scrubDeletedFiles`: deleted entries keep only identifiers and version.
    pub fn scrubbed(&self) -> File {
        File {
            id: self.id,
            owner_id: self.owner_id,
            collection_id: self.collection_id,
            collection_owner_id: self.collection_owner_id,
            metadata: FileAttributes {
                encrypted_data: "-".into(),
                ..Default::default()
            },
            is_deleted: true,
            updation_time: self.updation_time,
            ..Default::default()
        }
    }

    /// museum `GetPublicDiff`/`GetCastDiff` post-processing for one entry.
    pub fn for_public_diff(mut self) -> File {
        self.magic_metadata = None;
        if let Some(a) = &self.action
            && !self.is_deleted
            && (a == "REMOVE" || a == "DELETE_SUGGESTED")
        {
            self.is_deleted = true;
        }
        self.action = None;
        self.action_user = None;
        if self.metadata.encrypted_data == "-" && !self.is_deleted {
            tracing::warn!(file_id = self.id, "stale collection_file found");
            self.is_deleted = true;
        }
        if self.is_deleted {
            self.scrubbed()
        } else {
            self
        }
    }
}

/// museum `ente.CollectionUser`.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct CollectionUser {
    /// User id.
    pub id: i64,
    /// Email (empty on public views).
    pub email: String,
    /// Deprecated name.
    pub name: String,
    /// Role (empty for the owner on public views).
    pub role: String,
}

/// museum `ente.CollectionAttributes`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CollectionAttributes {
    /// Encrypted folder path.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub encrypted_path: String,
    /// Nonce for the path.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path_decryption_nonce: String,
    /// Version.
    pub version: i64,
}

/// museum `ente.PublicURL` (the limited form exposed to link viewers).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PublicUrl {
    /// Link URL (empty for viewers).
    pub url: String,
    /// Device limit.
    pub device_limit: i64,
    /// Expiry (0 = never).
    pub valid_till: i64,
    /// Downloads allowed.
    pub enable_download: bool,
    /// Uploads ("collect") allowed.
    pub enable_collect: bool,
    /// Comments allowed.
    pub enable_comment: bool,
    /// Password protected.
    pub password_enabled: bool,
    /// KDF nonce for the password.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    /// KDF memory limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_limit: Option<i64>,
    /// KDF ops limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ops_limit: Option<i64>,
    /// Joining the album allowed.
    pub enable_join: bool,
    /// Minimum role to see the link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_role: Option<String>,
}

/// museum `ente.Collection`.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    /// Collection id.
    pub id: i64,
    /// Owner.
    pub owner: CollectionUser,
    /// Collection key, encrypted for the owner.
    pub encrypted_key: String,
    /// Nonce for `encrypted_key`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key_decryption_nonce: String,
    /// Legacy plaintext name.
    pub name: String,
    /// Encrypted name.
    pub encrypted_name: String,
    /// Nonce for the name.
    pub name_decryption_nonce: String,
    /// album / folder / favorites / uncategorized.
    #[serde(rename = "type")]
    pub kind: String,
    /// Attributes.
    pub attributes: CollectionAttributes,
    /// Sharees (`null` on public views).
    pub sharees: Option<Vec<CollectionUser>>,
    /// Public links.
    #[serde(rename = "publicURLs")]
    pub public_urls: Option<Vec<PublicUrl>>,
    /// Version.
    pub updation_time: i64,
    /// When shared with the viewer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_at: Option<i64>,
    /// Deleted.
    #[serde(skip_serializing_if = "is_false")]
    pub is_deleted: bool,
    /// Private magic metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magic_metadata: Option<MagicMetadata>,
    /// App (photos/locker).
    pub app: String,
    /// Public magic metadata.
    #[serde(rename = "pubMagicMetadata", skip_serializing_if = "Option::is_none")]
    pub pub_magic_metadata: Option<MagicMetadata>,
    /// Sharee magic metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_magic_metadata: Option<MagicMetadata>,
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
        let v = serde_json::to_value(f.for_public_diff()).unwrap();
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
