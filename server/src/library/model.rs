//! JSON shapes, field for field like museum's `ente` package (names, casing, `omitempty`).

use serde::{Deserialize, Serialize};

fn is_zero(v: &i64) -> bool {
    *v == 0
}

/// `ente.MagicMetadata`. Stored as this JSON in `TEXT` columns.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MagicMetadata {
    /// Version, bumped by the server on every write.
    #[serde(skip_serializing_if = "is_zero")]
    pub version: i64,
    /// Number of keys in the decrypted JSON.
    #[serde(skip_serializing_if = "is_zero")]
    pub count: i64,
    /// Encrypted data (base64).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub data: String,
    /// Decryption header (base64).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub header: String,
}

impl MagicMetadata {
    /// gin `binding:"required"` on every field: none may be zero/empty.
    pub fn is_valid(&self) -> bool {
        self.version != 0 && self.count != 0 && !self.data.is_empty() && !self.header.is_empty()
    }
    /// Parse a stored column.
    pub fn parse(s: Option<&str>) -> Option<Self> {
        s.and_then(|s| serde_json::from_str(s).ok())
    }
    /// Serialise for storage.
    pub fn to_db(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

/// `ente.FileAttributes`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FileAttributes {
    /// Object key (file and thumbnail).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub object_key: String,
    /// Inline encrypted data (metadata).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub encrypted_data: String,
    /// Decryption header.
    pub decryption_header: String,
    /// Size as reported by the client.
    pub size: i64,
}

/// `ente.FileInfo`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FileInfo {
    /// File object size.
    #[serde(rename = "fileSize", skip_serializing_if = "is_zero")]
    pub file_size: i64,
    /// Thumbnail object size.
    #[serde(rename = "thumbSize", skip_serializing_if = "is_zero")]
    pub thumb_size: i64,
}

/// `ente.File`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct File {
    /// File id.
    pub id: i64,
    /// Owner.
    #[serde(rename = "ownerID")]
    pub owner_id: i64,
    /// Collection this view of the file belongs to.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// Collection owner (`null` when unknown).
    #[serde(rename = "collectionOwnerID")]
    pub collection_owner_id: Option<i64>,
    /// When the file was added to the collection.
    #[serde(rename = "collectionAddedAt", skip_serializing_if = "Option::is_none")]
    pub collection_added_at: Option<i64>,
    /// File key encrypted with the collection key.
    #[serde(rename = "encryptedKey")]
    pub encrypted_key: String,
    /// Nonce for `encryptedKey`.
    #[serde(rename = "keyDecryptionNonce")]
    pub key_decryption_nonce: String,
    /// The file object.
    pub file: FileAttributes,
    /// The thumbnail object.
    pub thumbnail: FileAttributes,
    /// Encrypted metadata.
    pub metadata: FileAttributes,
    /// Removed from this collection.
    #[serde(rename = "isDeleted")]
    pub is_deleted: bool,
    /// Last change (microseconds).
    #[serde(rename = "updationTime")]
    pub updation_time: i64,
    /// Private magic metadata.
    #[serde(rename = "magicMetadata", skip_serializing_if = "Option::is_none")]
    pub magic_metadata: Option<MagicMetadata>,
    /// Public magic metadata.
    #[serde(rename = "pubMagicMetadata", skip_serializing_if = "Option::is_none")]
    pub pub_magic_metadata: Option<MagicMetadata>,
    /// Object sizes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<FileInfo>,
    /// Pending collection action (`REMOVE`, `DELETE_SUGGESTED`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Who set `action`.
    #[serde(rename = "actionUser", skip_serializing_if = "Option::is_none")]
    pub action_user: Option<i64>,
}

/// `ente.CollectionUser`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectionUser {
    /// User id.
    pub id: i64,
    /// Email (decrypted), or `""`.
    pub email: String,
    /// Always `""` (museum never fills it).
    pub name: String,
    /// Role (`""` for the owner).
    pub role: String,
}

/// `ente.CollectionAttributes`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CollectionAttributes {
    /// Encrypted device folder path.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub encrypted_path: String,
    /// Nonce for the path.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path_decryption_nonce: String,
    /// Version.
    pub version: i64,
}

/// `ente.PublicURL`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicUrl {
    /// Shareable URL.
    pub url: String,
    /// Device limit (0 = unlimited).
    pub device_limit: i64,
    /// Expiry (0 = never).
    pub valid_till: i64,
    /// Downloads allowed.
    pub enable_download: bool,
    /// Uploads by viewers allowed.
    pub enable_collect: bool,
    /// Comments allowed.
    pub enable_comment: bool,
    /// Password set.
    pub password_enabled: bool,
    /// Password KDF nonce.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    /// KDF memory limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_limit: Option<i64>,
    /// KDF ops limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ops_limit: Option<i64>,
    /// Joining via link allowed.
    pub enable_join: bool,
    /// Minimum participant role that may see the link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_role: Option<String>,
}

/// `ente.Collection`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Collection {
    /// Collection id.
    pub id: i64,
    /// Owner.
    pub owner: CollectionUser,
    /// Collection key (encrypted with the owner's master key, or sealed to a sharee).
    #[serde(rename = "encryptedKey")]
    pub encrypted_key: String,
    /// Nonce for `encryptedKey` (absent for sharees).
    #[serde(
        rename = "keyDecryptionNonce",
        skip_serializing_if = "String::is_empty"
    )]
    pub key_decryption_nonce: String,
    /// Legacy plaintext name.
    pub name: String,
    /// Encrypted name.
    #[serde(rename = "encryptedName")]
    pub encrypted_name: String,
    /// Nonce for the name.
    #[serde(rename = "nameDecryptionNonce")]
    pub name_decryption_nonce: String,
    /// `album`, `folder`, `favorites`, `uncategorized`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Attributes.
    pub attributes: CollectionAttributes,
    /// Sharees (`null` when not loaded).
    pub sharees: Option<Vec<CollectionUser>>,
    /// Active public links (`null` when not loaded).
    #[serde(rename = "publicURLs")]
    pub public_urls: Option<Vec<PublicUrl>>,
    /// Last change (microseconds).
    #[serde(rename = "updationTime")]
    pub updation_time: i64,
    /// When the viewer was given access.
    #[serde(rename = "sharedAt", skip_serializing_if = "Option::is_none")]
    pub shared_at: Option<i64>,
    /// Deleted.
    #[serde(rename = "isDeleted", skip_serializing_if = "std::ops::Not::not")]
    pub is_deleted: bool,
    /// Owner-private magic metadata.
    #[serde(rename = "magicMetadata", skip_serializing_if = "Option::is_none")]
    pub magic_metadata: Option<MagicMetadata>,
    /// App (`photos`, `locker`, `auth`).
    pub app: String,
    /// Public magic metadata.
    #[serde(rename = "pubMagicMetadata", skip_serializing_if = "Option::is_none")]
    pub pub_magic_metadata: Option<MagicMetadata>,
    /// The viewer's own (sharee) magic metadata.
    #[serde(
        rename = "sharedMagicMetadata",
        skip_serializing_if = "Option::is_none"
    )]
    pub shared_magic_metadata: Option<MagicMetadata>,
}

/// Participant roles (`ente.CollectionParticipantRole`).
pub mod role {
    /// Read-only sharee.
    pub const VIEWER: &str = "VIEWER";
    /// Sharee who can add files.
    pub const COLLABORATOR: &str = "COLLABORATOR";
    /// Sharee who can also manage participants and remove files.
    pub const ADMIN: &str = "ADMIN";
    /// The owner.
    pub const OWNER: &str = "OWNER";
    /// Anything else.
    pub const UNKNOWN: &str = "UNKNOWN";

    /// `ConvertStringToCollectionParticipantRole`.
    pub fn normalize(s: &str) -> &'static str {
        match s {
            VIEWER => VIEWER,
            OWNER => OWNER,
            COLLABORATOR => COLLABORATOR,
            ADMIN => ADMIN,
            _ => UNKNOWN,
        }
    }
    /// Rank for `Satisfies`.
    pub fn rank(s: &str) -> i32 {
        match s {
            VIEWER => 1,
            COLLABORATOR => 2,
            ADMIN => 3,
            OWNER => 4,
            _ => 0,
        }
    }
    /// `role.Satisfies(min)`.
    pub fn satisfies(role: &str, min: Option<&str>) -> bool {
        min.is_none_or(|m| rank(role) >= rank(m))
    }
    /// `CanAdd`.
    pub fn can_add(role: &str) -> bool {
        matches!(role, OWNER | COLLABORATOR | ADMIN)
    }
    /// `IsValidShareRole`.
    pub fn is_valid_share_role(role: &str) -> bool {
        matches!(role, VIEWER | COLLABORATOR | ADMIN | OWNER)
    }
}

/// `ente.FilterPublicURLsForRole`.
pub fn filter_public_urls_for_role(urls: Vec<PublicUrl>, role: &str) -> Vec<PublicUrl> {
    let role = if role == role::UNKNOWN {
        role::VIEWER
    } else {
        role
    };
    urls.into_iter()
        .filter(|u| role::satisfies(role, u.min_role.as_deref()))
        .collect()
}

/// Collection file actions.
pub mod action {
    /// An admin removed the owner's file from a shared album.
    pub const REMOVE: &str = "REMOVE";
    /// An admin suggested that the owner delete the file.
    pub const DELETE_SUGGESTED: &str = "DELETE_SUGGESTED";
}

/// A collection file item in requests (`ente.CollectionFileItem`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CollectionFileItem {
    /// File id.
    pub id: i64,
    /// File key encrypted with the collection key.
    pub encrypted_key: String,
    /// Nonce.
    pub key_decryption_nonce: String,
}

impl CollectionFileItem {
    /// gin `binding:"required"`.
    pub fn has_required(&self) -> bool {
        self.id != 0 && !self.encrypted_key.is_empty() && !self.key_decryption_nonce.is_empty()
    }
}
