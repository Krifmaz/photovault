//! JSON types of the Ente API, field-for-field as museum sends and expects them.
//!
//! Everything here is ciphertext or metadata the server already has; plaintext
//! types live in [`crate::library`].

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

fn default_true() -> bool {
    true
}

/// `GET /users/srp/attributes` → `attributes`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SrpAttributes {
    /// SRP identity (a UUID string).
    #[serde(rename = "srpUserID")]
    pub srp_user_id: String,
    /// SRP salt (base64).
    pub srp_salt: String,
    /// Argon2id memory limit for the KEK.
    pub mem_limit: u64,
    /// Argon2id passes for the KEK.
    pub ops_limit: u32,
    /// KEK salt (base64).
    pub kek_salt: String,
    /// Whether an email code is required before the password.
    #[serde(rename = "isEmailMFAEnabled", default = "default_true")]
    pub is_email_mfa_enabled: bool,
}

/// The account's wrapped keys (`keyAttributes`).
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyAttributes {
    /// Argon2id salt for the KEK (base64).
    pub kek_salt: String,
    /// Master key, secretbox-encrypted with the KEK (base64).
    pub encrypted_key: String,
    /// Nonce for `encrypted_key` (base64).
    pub key_decryption_nonce: String,
    /// X25519 public key (base64).
    pub public_key: String,
    /// X25519 secret key, secretbox-encrypted with the master key (base64).
    pub encrypted_secret_key: String,
    /// Nonce for `encrypted_secret_key` (base64).
    pub secret_key_decryption_nonce: String,
    /// Argon2id memory limit.
    pub mem_limit: u64,
    /// Argon2id passes.
    pub ops_limit: u32,
    /// Master key encrypted with the recovery key (unused by the node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_key_encrypted_with_recovery_key: Option<String>,
    /// Nonce for the above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_key_decryption_nonce: Option<String>,
    /// Recovery key encrypted with the master key (unused by the node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_key_encrypted_with_master_key: Option<String>,
    /// Nonce for the above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_key_decryption_nonce: Option<String>,
}

impl std::fmt::Debug for KeyAttributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyAttributes")
            .field("mem_limit", &self.mem_limit)
            .field("ops_limit", &self.ops_limit)
            .finish_non_exhaustive()
    }
}

/// Response of `verify-session`, `verify-email` and `two-factor/verify`.
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    /// User id.
    #[serde(default)]
    pub id: i64,
    /// Wrapped keys (absent for accounts that never set a password).
    #[serde(default)]
    pub key_attributes: Option<KeyAttributes>,
    /// Token sealed to the account's public key (base64).
    #[serde(default)]
    pub encrypted_token: Option<String>,
    /// Plain token, only for accounts without keys yet.
    #[serde(default)]
    pub token: Option<String>,
    /// Set when TOTP is required.
    #[serde(rename = "twoFactorSessionID", default)]
    pub two_factor_session_id: Option<String>,
    /// Newer name for the TOTP session id.
    #[serde(rename = "twoFactorSessionIDV2", default)]
    pub two_factor_session_id_v2: Option<String>,
    /// Set when a passkey is required.
    #[serde(rename = "passkeySessionID", default)]
    pub passkey_session_id: Option<String>,
    /// Server SRP proof (base64), on `verify-session`.
    #[serde(default)]
    pub srp_m2: Option<String>,
    /// Accounts app URL for passkey verification.
    #[serde(default)]
    pub accounts_url: Option<String>,
}

impl std::fmt::Debug for AuthResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthResponse")
            .field("id", &self.id)
            .field("has_key_attributes", &self.key_attributes.is_some())
            .field("totp", &self.totp_session_id().is_some())
            .field("passkey", &self.passkey_required())
            .finish_non_exhaustive()
    }
}

impl AuthResponse {
    /// The TOTP session id, if a TOTP code is required.
    pub fn totp_session_id(&self) -> Option<&str> {
        self.two_factor_session_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.two_factor_session_id_v2
                    .as_deref()
                    .filter(|s| !s.is_empty())
            })
    }

    /// Whether a passkey is required.
    pub fn passkey_required(&self) -> bool {
        self.passkey_session_id
            .as_deref()
            .is_some_and(|s| !s.is_empty())
    }
}

/// `POST /users/srp/create-session` request.
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSrpSessionRequest {
    /// SRP identity.
    #[serde(rename = "srpUserID")]
    pub srp_user_id: String,
    /// Client public value `A` (base64, padded to 512 bytes).
    #[serde(rename = "srpA")]
    pub srp_a: String,
}

/// `POST /users/srp/create-session` response.
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSrpSessionResponse {
    /// Session id.
    #[serde(rename = "sessionID")]
    pub session_id: String,
    /// Server public value `B` (base64).
    #[serde(rename = "srpB")]
    pub srp_b: String,
}

/// `POST /users/srp/verify-session` request.
#[derive(Debug, Serialize, Deserialize)]
pub struct VerifySrpSessionRequest {
    /// SRP identity.
    #[serde(rename = "srpUserID")]
    pub srp_user_id: String,
    /// Session id from create-session.
    #[serde(rename = "sessionID")]
    pub session_id: String,
    /// Client proof `M1` (base64).
    #[serde(rename = "srpM1")]
    pub srp_m1: String,
}

// ---------------------------------------------------------------------------
// Collections and files
// ---------------------------------------------------------------------------

/// A user in a collection (`owner`, `sharees`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CollectionUser {
    /// User id.
    pub id: i64,
    /// Email, if the server shares it.
    #[serde(default)]
    pub email: String,
    /// Role (`OWNER`, `VIEWER`, `COLLABORATOR`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Magic metadata as sent by museum (`magicMetadata`, `pubMagicMetadata`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MagicMetadata {
    /// Version, bumped on each edit.
    #[serde(default)]
    pub version: i64,
    /// Number of keys.
    #[serde(default)]
    pub count: i64,
    /// Encrypted JSON (base64 blob).
    pub data: String,
    /// Blob header (base64).
    pub header: String,
}

/// An element of `GET /collections/v2` → `collections`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCollection {
    /// Collection id.
    pub id: i64,
    /// Owner.
    pub owner: CollectionUser,
    /// Collection key: secretbox with the master key (owned) or sealed box (shared).
    #[serde(default)]
    pub encrypted_key: String,
    /// Nonce for `encrypted_key` (owned collections only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_decryption_nonce: Option<String>,
    /// Legacy plaintext name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Name, secretbox with the collection key (base64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_name: Option<String>,
    /// Nonce for `encrypted_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_decryption_nonce: Option<String>,
    /// `album`, `folder`, `favorites`, `uncategorized`.
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Sharees.
    #[serde(default)]
    pub sharees: Vec<CollectionUser>,
    /// Last change, microseconds since epoch.
    #[serde(default)]
    pub updation_time: i64,
    /// Whether the collection was deleted.
    #[serde(default)]
    pub is_deleted: bool,
    /// Owner-private magic metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magic_metadata: Option<MagicMetadata>,
    /// Public magic metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_magic_metadata: Option<MagicMetadata>,
    /// Per-sharee magic metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_magic_metadata: Option<MagicMetadata>,
}

/// `GET /collections/v2` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionsResponse {
    /// Collections changed since `sinceTime`.
    #[serde(default)]
    pub collections: Vec<RemoteCollection>,
}

/// Object attributes of a file part (`file`, `thumbnail`, `metadata`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileAttributes {
    /// Inline ciphertext (base64), only for `metadata`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_data: Option<String>,
    /// Secretstream header (base64).
    #[serde(default)]
    pub decryption_header: String,
    /// Object size in bytes, when the server reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
}

/// `info` on a file.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    /// Encrypted original size.
    #[serde(default)]
    pub file_size: i64,
    /// Encrypted thumbnail size.
    #[serde(default)]
    pub thumb_size: i64,
}

/// An element of `GET /collections/v2/diff` → `diff`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFile {
    /// File id.
    pub id: i64,
    /// File owner.
    #[serde(rename = "ownerID", default)]
    pub owner_id: i64,
    /// The collection this entry belongs to.
    #[serde(rename = "collectionID", default)]
    pub collection_id: i64,
    /// File key, secretbox with the collection key (base64).
    #[serde(default)]
    pub encrypted_key: String,
    /// Nonce for `encrypted_key`.
    #[serde(default)]
    pub key_decryption_nonce: String,
    /// Original.
    #[serde(default)]
    pub file: FileAttributes,
    /// Thumbnail.
    #[serde(default)]
    pub thumbnail: FileAttributes,
    /// Metadata (inline encrypted JSON).
    #[serde(default)]
    pub metadata: FileAttributes,
    /// Removed from this collection (or trashed / deleted).
    #[serde(default)]
    pub is_deleted: bool,
    /// Last change, microseconds since epoch.
    #[serde(default)]
    pub updation_time: i64,
    /// Owner-private magic metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magic_metadata: Option<MagicMetadata>,
    /// Public magic metadata (dimensions, edited time, `sv`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_magic_metadata: Option<MagicMetadata>,
    /// Sizes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<FileInfo>,
}

impl RemoteFile {
    /// Whether this entry means "not live in this collection" (Ente marks
    /// deleted entries with `isDeleted` and, in old servers, metadata `"-"`).
    pub fn is_removed(&self) -> bool {
        self.is_deleted || self.metadata.encrypted_data.as_deref() == Some("-")
    }
}

/// `GET /collections/v2/diff` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDiff {
    /// Changed entries, ordered by `updationTime`.
    #[serde(default)]
    pub diff: Vec<RemoteFile>,
    /// Whether another page follows.
    #[serde(default)]
    pub has_more: bool,
}

// ---------------------------------------------------------------------------
// File data
// ---------------------------------------------------------------------------

/// One `mldata` / `vid_preview` record (`/files/data/fetch`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileData {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Type (`mldata`, `vid_preview`).
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Blob ciphertext (base64), encrypted with the file key.
    pub encrypted_data: String,
    /// Blob header (base64).
    pub decryption_header: String,
    /// Server update time (absent on older museums).
    #[serde(default)]
    pub updated_at: Option<i64>,
}

/// `POST /files/data/fetch` response.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FilesDataResponse {
    /// Records found.
    #[serde(default)]
    pub data: Vec<RemoteFileData>,
    /// Files without data yet.
    #[serde(rename = "pendingIndexFileIDs", default)]
    pub pending_index_file_ids: Vec<i64>,
    /// Files whose lookup failed.
    #[serde(rename = "errFileIDs", default)]
    pub err_file_ids: Vec<i64>,
}

/// One entry of `POST /files/data/status-diff`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileDataStatus {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Owner.
    #[serde(rename = "userID", default)]
    pub user_id: i64,
    /// Type (`mldata`, `vid_preview`, `img_preview`).
    #[serde(rename = "type")]
    pub kind: String,
    /// Whether the data was deleted.
    #[serde(default)]
    pub is_deleted: bool,
    /// Preview object id, for preview types.
    #[serde(rename = "objectID", default)]
    pub object_id: Option<String>,
    /// Object size.
    #[serde(default)]
    pub size: i64,
    /// Server update time; the cursor for the next request.
    pub updated_at: i64,
}

/// `POST /files/data/status-diff` response.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StatusDiffResponse {
    /// Changes; empty or null at the end.
    #[serde(default)]
    pub diff: Option<Vec<FileDataStatus>>,
}

/// `PUT /files/data` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutFileDataRequest {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Type (`mldata`).
    #[serde(rename = "type")]
    pub kind: String,
    /// Blob ciphertext (base64).
    pub encrypted_data: String,
    /// Blob header (base64).
    pub decryption_header: String,
    /// Format version, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    /// `updatedAt` of the remote record this write replaces (0 for none):
    /// the server rejects the write if the record changed since.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated_at: Option<i64>,
}

/// Preview upload URLs (`/files/data/preview-upload-url` and friends).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewUploadUrl {
    /// Object id to pass to `PUT /files/video-data`.
    #[serde(rename = "objectID")]
    pub object_id: String,
    /// Single-part upload URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Multipart part URLs, in order.
    #[serde(rename = "partURLs", default, skip_serializing_if = "Option::is_none")]
    pub part_urls: Option<Vec<String>>,
    /// Multipart completion URL.
    #[serde(
        rename = "completeURL",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub complete_url: Option<String>,
}

/// `PUT /files/video-data` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutVideoDataRequest {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Object id from the preview upload URL.
    #[serde(rename = "objectID")]
    pub object_id: String,
    /// Uploaded (encrypted) segment object size.
    #[serde(rename = "objectSize")]
    pub object_size: i64,
    /// Encrypted playlist JSON (base64 blob).
    pub playlist: String,
    /// Playlist blob header (base64).
    #[serde(rename = "playlistHeader")]
    pub playlist_header: String,
    /// Format version, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
}
