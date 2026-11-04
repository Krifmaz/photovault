//! JSON shapes of Ente's API, as museum (Ente's Go server) defines them in
//! `server/ente/*.go` and the apps parse them in `web/packages/media` and
//! `web/packages/accounts`.
//!
//! Copied from `importers/src/ente/model.rs` (branch `claude/project-thread-cp9x81`,
//! commit 1c62c2c) and extended with what a full client reads: recovery-key
//! attributes, SRP setup/update, file objects, user details. Only the fields
//! the core reads or writes are modelled; unknown fields are ignored.

use serde::{Deserialize, Serialize};

/// `GET /users/srp/attributes` response body.
#[derive(Debug, Clone, Deserialize)]
pub struct SrpAttributesResponse {
    /// The attributes.
    pub attributes: SrpAttributes,
}

/// A user's SRP attributes (`ente.GetSRPAttributesResponse`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SrpAttributes {
    /// SRP identity (a UUID string).
    #[serde(rename = "srpUserID")]
    pub srp_user_id: String,
    /// Base64 SRP salt.
    pub srp_salt: String,
    /// Argon2id memory limit for the key-encryption key.
    pub mem_limit: u64,
    /// Argon2id ops limit for the key-encryption key.
    pub ops_limit: u64,
    /// Base64 salt for the key-encryption key.
    pub kek_salt: String,
    /// Whether email OTP is required as a second factor (then SRP is skipped).
    #[serde(default, rename = "isEmailMFAEnabled")]
    pub is_email_mfa_enabled: bool,
}

/// `POST /users/srp/create-session` request.
#[derive(Debug, Clone, Serialize)]
pub struct CreateSrpSessionRequest<'a> {
    /// SRP identity.
    #[serde(rename = "srpUserID")]
    pub srp_user_id: &'a str,
    /// Base64 client public value `A`.
    #[serde(rename = "srpA")]
    pub srp_a: String,
}

/// `POST /users/srp/create-session` response.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateSrpSessionResponse {
    /// Session id (UUID).
    #[serde(rename = "sessionID")]
    pub session_id: String,
    /// Base64 server public value `B`.
    #[serde(rename = "srpB")]
    pub srp_b: String,
}

/// `POST /users/srp/verify-session` request.
#[derive(Debug, Clone, Serialize)]
pub struct VerifySrpSessionRequest<'a> {
    /// SRP identity.
    #[serde(rename = "srpUserID")]
    pub srp_user_id: &'a str,
    /// Session id from create-session.
    #[serde(rename = "sessionID")]
    pub session_id: &'a str,
    /// Base64 client proof `M1`.
    #[serde(rename = "srpM1")]
    pub srp_m1: String,
}

/// `POST /users/ott` request.
#[derive(Debug, Clone, Serialize)]
pub struct SendOttRequest<'a> {
    /// Account email.
    pub email: &'a str,
    /// Always `"login"` here.
    pub purpose: &'a str,
}

/// `POST /users/verify-email` request.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyEmailRequest<'a> {
    /// Account email.
    pub email: &'a str,
    /// The code from the email.
    pub ott: &'a str,
}

/// `POST /users/two-factor/verify` request.
#[derive(Debug, Clone, Serialize)]
pub struct TwoFactorVerifyRequest<'a> {
    /// `twoFactorSessionID` from the previous step.
    #[serde(rename = "sessionID")]
    pub session_id: &'a str,
    /// The TOTP code.
    pub code: &'a str,
}

/// A user's key attributes (`ente.KeyAttributes`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyAttributes {
    /// Base64 salt for the key-encryption key.
    pub kek_salt: String,
    /// Base64 master key, secretbox'd with the key-encryption key.
    pub encrypted_key: String,
    /// Nonce for [`Self::encrypted_key`].
    pub key_decryption_nonce: String,
    /// Base64 X25519 public key.
    pub public_key: String,
    /// Base64 secret key, secretbox'd with the master key.
    pub encrypted_secret_key: String,
    /// Nonce for [`Self::encrypted_secret_key`].
    pub secret_key_decryption_nonce: String,
    /// Argon2id memory limit.
    pub mem_limit: u64,
    /// Argon2id ops limit.
    pub ops_limit: u64,
    /// Master key, secretbox'd with the recovery key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_key_encrypted_with_recovery_key: Option<String>,
    /// Nonce for [`Self::master_key_encrypted_with_recovery_key`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_key_decryption_nonce: Option<String>,
    /// Recovery key, secretbox'd with the master key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_key_encrypted_with_master_key: Option<String>,
    /// Nonce for [`Self::recovery_key_encrypted_with_master_key`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_key_decryption_nonce: Option<String>,
}

/// What museum answers after a login step (`ente.EmailAuthorizationResponse`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationResponse {
    /// User id.
    #[serde(default)]
    pub id: i64,
    /// Key attributes; absent while a second factor is pending.
    #[serde(default)]
    pub key_attributes: Option<KeyAttributes>,
    /// Auth token sealed to the user's public key.
    #[serde(default)]
    pub encrypted_token: Option<String>,
    /// Plain token (only for accounts that have not set up keys yet).
    #[serde(default)]
    pub token: Option<String>,
    /// Set when TOTP is required.
    #[serde(default, rename = "twoFactorSessionID")]
    pub two_factor_session_id: Option<String>,
    /// Set when a passkey is required.
    #[serde(default, rename = "passkeySessionID")]
    pub passkey_session_id: Option<String>,
    /// Where the passkey ceremony happens.
    #[serde(default)]
    pub accounts_url: Option<String>,
    /// Base64 server proof `M2` (SRP only).
    #[serde(default, rename = "srpM2")]
    pub srp_m2: Option<String>,
}

/// Encrypted magic metadata as stored on the server (`ente.MagicMetadata`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMagicMetadata {
    /// Version; the server bumps it on every update and rejects updates
    /// whose version differs from the stored one.
    pub version: i64,
    /// Number of non-null keys in the JSON.
    #[serde(default)]
    pub count: i64,
    /// Base64 secretstream ciphertext of the JSON.
    pub data: String,
    /// Base64 secretstream header.
    pub header: String,
}

/// A collection owner or sharee.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CollectionUser {
    /// User id.
    pub id: i64,
    /// Email (owner and sharees).
    #[serde(default)]
    pub email: String,
    /// Participant role of a sharee (`VIEWER`, `COLLABORATOR`, `ADMIN`).
    #[serde(default)]
    pub role: String,
}

/// A public link of a collection (`ente.PublicURL`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePublicUrl {
    /// The link, without the key fragment.
    #[serde(default)]
    pub url: String,
    /// 0 = unlimited.
    #[serde(default)]
    pub device_limit: i64,
    /// Expiry, µs; 0 = never.
    #[serde(default)]
    pub valid_till: i64,
    /// Viewers may download originals.
    #[serde(default)]
    pub enable_download: bool,
    /// Viewers may add photos.
    #[serde(default)]
    pub enable_collect: bool,
    /// Signed-in viewers may join the album.
    #[serde(default)]
    pub enable_join: bool,
    /// The link asks for a password.
    #[serde(default)]
    pub password_enabled: bool,
}

/// A collection as `GET /collections/v2` and `POST /collections` return it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCollection {
    /// Collection id.
    pub id: i64,
    /// Owner.
    pub owner: CollectionUser,
    /// Collection key: secretbox'd with the master key (owner) or sealed box
    /// to the user's public key (sharee).
    #[serde(default)]
    pub encrypted_key: String,
    /// Nonce for an owner's [`Self::encrypted_key`].
    #[serde(default)]
    pub key_decryption_nonce: Option<String>,
    /// Legacy plaintext name.
    #[serde(default)]
    pub name: Option<String>,
    /// Name, secretbox'd with the collection key.
    #[serde(default)]
    pub encrypted_name: Option<String>,
    /// Nonce for [`Self::encrypted_name`].
    #[serde(default)]
    pub name_decryption_nonce: Option<String>,
    /// `album`, `folder`, `favorites` or `uncategorized`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Server time of the last change, µs.
    #[serde(default)]
    pub updation_time: i64,
    /// Tombstone.
    #[serde(default)]
    pub is_deleted: bool,
    /// Owner's private magic metadata (subType, visibility, order).
    #[serde(default)]
    pub magic_metadata: Option<RemoteMagicMetadata>,
    /// People the collection is shared with (owner's view).
    #[serde(default)]
    pub sharees: Option<Vec<CollectionUser>>,
    /// Public links.
    #[serde(default, rename = "publicURLs")]
    pub public_urls: Option<Vec<RemotePublicUrl>>,
}

/// `POST /collections` request.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCollectionRequest {
    /// Collection key secretbox'd with the master key.
    pub encrypted_key: String,
    /// Its nonce.
    pub key_decryption_nonce: String,
    /// Name secretbox'd with the collection key.
    pub encrypted_name: String,
    /// Its nonce.
    pub name_decryption_nonce: String,
    /// Collection type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Private magic metadata, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magic_metadata: Option<RemoteMagicMetadata>,
}

/// `{"collection": …}` wrapper.
#[derive(Debug, Clone, Deserialize)]
pub struct CollectionResponse {
    /// The collection.
    pub collection: RemoteCollection,
}

/// `{"collections": […]}` wrapper.
#[derive(Debug, Clone, Deserialize)]
pub struct CollectionsResponse {
    /// The collections (including tombstones).
    pub collections: Vec<RemoteCollection>,
}

/// Header/data pair of an uploaded or inline encrypted object.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAttributes {
    /// Object key in the bucket (file and thumbnail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_key: Option<String>,
    /// Inline base64 ciphertext (metadata).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_data: Option<String>,
    /// Base64 secretstream header.
    pub decryption_header: String,
    /// Ciphertext size (file and thumbnail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// A file as `GET /collections/v2/diff` and `POST /files` return it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFile {
    /// File id.
    pub id: i64,
    /// Owner.
    #[serde(default, rename = "ownerID")]
    pub owner_id: i64,
    /// The collection this entry belongs to.
    #[serde(default, rename = "collectionID")]
    pub collection_id: i64,
    /// File key secretbox'd with the collection key.
    #[serde(default)]
    pub encrypted_key: String,
    /// Its nonce.
    #[serde(default)]
    pub key_decryption_nonce: String,
    /// The file object (absent on tombstones).
    #[serde(default)]
    pub file: Option<FileAttributes>,
    /// The thumbnail object (absent on tombstones).
    #[serde(default)]
    pub thumbnail: Option<FileAttributes>,
    /// Encrypted metadata (absent on tombstones).
    #[serde(default)]
    pub metadata: Option<FileAttributes>,
    /// Public magic metadata (w, h, editedTime, editedName, caption).
    #[serde(default)]
    pub pub_magic_metadata: Option<RemoteMagicMetadata>,
    /// Plaintext sizes, when the uploader reported them.
    #[serde(default)]
    pub info: Option<FileInfo>,
    /// Private magic metadata (visibility).
    #[serde(default)]
    pub magic_metadata: Option<RemoteMagicMetadata>,
    /// Tombstone: the file left this collection.
    #[serde(default)]
    pub is_deleted: bool,
    /// Server time of the last change, µs.
    #[serde(default)]
    pub updation_time: i64,
}

/// `GET /collections/v2/diff` response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffResponse {
    /// Changed files.
    pub diff: Vec<RemoteFile>,
    /// More pages follow.
    #[serde(default)]
    pub has_more: bool,
}

/// `POST /files` request (`PostEnteFileRequest` in the web app).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFileRequest {
    /// Collection to create the file in.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// File key secretbox'd with the collection key.
    pub encrypted_key: String,
    /// Its nonce.
    pub key_decryption_nonce: String,
    /// The uploaded file object.
    pub file: FileAttributes,
    /// The uploaded thumbnail object.
    pub thumbnail: FileAttributes,
    /// Encrypted metadata JSON.
    pub metadata: FileAttributes,
    /// Public magic metadata, when it has at least one key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pub_magic_metadata: Option<RemoteMagicMetadata>,
}

/// One file in `POST /collections/add-files`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionFileItem {
    /// File id.
    pub id: i64,
    /// File key secretbox'd with the target collection's key.
    pub encrypted_key: String,
    /// Its nonce.
    pub key_decryption_nonce: String,
}

/// `POST /collections/add-files` request.
#[derive(Debug, Clone, Serialize)]
pub struct AddFilesRequest {
    /// Target collection.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// Files with their re-encrypted keys.
    pub files: Vec<CollectionFileItem>,
}

/// One entry of `PUT /files/magic-metadata`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMagicMetadata {
    /// File id.
    pub id: i64,
    /// The new magic metadata, carrying the version currently stored.
    pub magic_metadata: RemoteMagicMetadata,
}

/// `PUT /files/magic-metadata` request.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMagicMetadataRequest {
    /// Updates.
    pub metadata_list: Vec<UpdateMagicMetadata>,
}

/// A presigned single-object upload URL.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadUrl {
    /// Object key to report in `POST /files`.
    pub object_key: String,
    /// Where to `PUT` the bytes.
    pub url: String,
}

/// `GET /files/upload-urls` response.
#[derive(Debug, Clone, Deserialize)]
pub struct UploadUrlsResponse {
    /// The URLs.
    pub urls: Vec<UploadUrl>,
}

/// `POST /files/upload-url` request (checksum-protected upload).
#[derive(Debug, Clone, Serialize)]
pub struct UploadUrlRequest {
    /// Exact ciphertext length.
    #[serde(rename = "contentLength")]
    pub content_length: u64,
    /// Base64 MD5 of the ciphertext.
    #[serde(rename = "contentMD5")]
    pub content_md5: String,
}

/// Presigned URLs for a multipart upload.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultipartUploadUrls {
    /// Object key to report in `POST /files`.
    pub object_key: String,
    /// One `PUT` URL per part, in order.
    #[serde(rename = "partURLs")]
    pub part_urls: Vec<String>,
    /// Where to `POST` the completion XML.
    #[serde(rename = "completeURL")]
    pub complete_url: String,
}

/// `GET /files/multipart-upload-urls` response.
#[derive(Debug, Clone, Deserialize)]
pub struct MultipartUploadUrlsResponse {
    /// The URLs.
    pub urls: MultipartUploadUrls,
}

/// `POST /files/multipart-upload-url` request. The importer uses the
/// "deferred checksum" form the web app also uses: no `partMd5s` up front,
/// a `Content-MD5` header on every part instead.
#[derive(Debug, Clone, Serialize)]
pub struct MultipartUploadUrlRequest {
    /// Total ciphertext length.
    #[serde(rename = "contentLength")]
    pub content_length: u64,
    /// Length of every part but the last.
    #[serde(rename = "partLength")]
    pub part_length: u64,
}

/// Plaintext sizes reported at upload (`ente.FileInfo`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    /// Original size in bytes.
    #[serde(default)]
    pub file_size: i64,
    /// Thumbnail size in bytes.
    #[serde(default)]
    pub thumb_size: i64,
}

/// `POST /users/srp/setup` request.
#[derive(Debug, Clone, Serialize)]
pub struct SetupSrpRequest {
    /// New SRP identity (UUID).
    #[serde(rename = "srpUserID")]
    pub srp_user_id: String,
    /// Base64 salt.
    #[serde(rename = "srpSalt")]
    pub srp_salt: String,
    /// Base64 verifier.
    #[serde(rename = "srpVerifier")]
    pub srp_verifier: String,
    /// Base64 client public value `A`.
    #[serde(rename = "srpA")]
    pub srp_a: String,
}

/// `POST /users/srp/setup` response.
#[derive(Debug, Clone, Deserialize)]
pub struct SetupSrpResponse {
    /// Setup id.
    #[serde(rename = "setupID")]
    pub setup_id: String,
    /// Base64 server public value `B`.
    #[serde(rename = "srpB")]
    pub srp_b: String,
}

/// The password-derived part of the key attributes (`ente.UpdateKeysRequest`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatedKeyAttributes {
    /// Base64 KEK salt.
    pub kek_salt: String,
    /// Master key secretbox'd with the new KEK.
    pub encrypted_key: String,
    /// Its nonce.
    pub key_decryption_nonce: String,
    /// Argon2id memory limit.
    pub mem_limit: u64,
    /// Argon2id ops limit.
    pub ops_limit: u64,
}

/// `POST /users/srp/update` request (`ente.UpdateSRPAndKeysRequest`).
#[derive(Debug, Clone, Serialize)]
pub struct UpdateSrpAndKeysRequest {
    /// Setup id from `/users/srp/setup`.
    #[serde(rename = "setupID")]
    pub setup_id: String,
    /// Base64 client proof.
    #[serde(rename = "srpM1")]
    pub srp_m1: String,
    /// New key attributes.
    #[serde(rename = "updatedKeyAttr")]
    pub updated_key_attr: UpdatedKeyAttributes,
    /// Revoke other sessions (Ente's apps send `false` from recovery).
    #[serde(rename = "logOutOtherDevices")]
    pub log_out_other_devices: bool,
}

/// `POST /users/srp/update` response.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSrpAndKeysResponse {
    /// Base64 server proof.
    #[serde(rename = "srpM2", default)]
    pub srp_m2: Option<String>,
}

/// A family member in `/users/details/v2`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FamilyMember {
    /// Bytes used by this member.
    #[serde(default)]
    pub usage: i64,
}

/// `familyData` in `/users/details/v2`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FamilyData {
    /// Members (including the admin).
    #[serde(default)]
    pub members: Vec<FamilyMember>,
    /// The family's storage limit.
    #[serde(default)]
    pub storage: i64,
}

/// `subscription` in `/users/details/v2`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Subscription {
    /// Storage limit in bytes.
    #[serde(default)]
    pub storage: i64,
}

/// `GET /users/details/v2` (the fields the storage card needs).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDetails {
    /// Account email.
    #[serde(default)]
    pub email: String,
    /// Bytes used.
    #[serde(default)]
    pub usage: i64,
    /// Plan.
    #[serde(default)]
    pub subscription: Option<Subscription>,
    /// Bonus storage.
    #[serde(default)]
    pub storage_bonus: i64,
    /// Family plan, if any.
    #[serde(default)]
    pub family_data: Option<FamilyData>,
}

/// `GET /photovault/usage` (contracts/storage-usage.md).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotovaultUsage {
    /// Bytes used.
    pub used_bytes: u64,
    /// Limit; `null` when unlimited.
    #[serde(default)]
    pub limit_bytes: Option<u64>,
    /// Free bytes.
    #[serde(default)]
    pub free_bytes: Option<u64>,
    /// Unlimited storage.
    #[serde(default)]
    pub unlimited: bool,
}

/// `GET /photovault/info` (contracts/server-info.md). Unknown fields ignored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotovaultInfo {
    /// Always `"photovault"`.
    #[serde(default)]
    pub product: String,
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// Server version.
    #[serde(default)]
    pub version: String,
    /// Public URL, no trailing slash.
    #[serde(default)]
    pub public_url: String,
    /// LAN URL.
    #[serde(default)]
    pub home_url: Option<String>,
    /// Whether sign-ups are open.
    #[serde(default)]
    pub signups_allowed: bool,
}
