# photovault-core interface (contract v1)

Adopted from the native thread's proposal (`/mnt/project-files/native/core-api-proposal.md`) with the
amendments marked **[contract]**. Crate `core/` (`photovault-core`, lib `photovault_core`,
crate-type `cdylib`, `staticlib`, `lib`), workspace member. UniFFI 0.32 with proc-macros; the generated
Kotlin lives in package `app.photovault.core`. The Rust signatures below are the contract: the native
app (and its mock) code against exactly these names and types.

Rules:
* Crypto is Ente's, byte for byte: libsodium (`libsodium-sys-stable`), Argon2id KEK derivation with the
  user's `memLimit`/`opsLimit`, XSalsa20-Poly1305 secretbox for keys, XChaCha20-Poly1305 secretstream for
  file/thumbnail/metadata, sealed boxes for shared collection keys, SRP-6a with the 4096-bit group and
  SHA-256 exactly as Ente's clients (`server/tests/vectors/` must pass). No custom crypto, no shortcuts.
* All network/disk calls are `async` (Kotlin `suspend`), run on a tokio runtime owned by core.
  TLS: rustls + ring. Sync calls only read the local store and must return in < 5 ms for a page of 200.
* The local store (SQLite, `rusqlite` bundled) never holds plaintext keys; v1 wraps the master key with
  secretbox under a device key in app-private storage, later Android Keystore.
* **[contract]** Base URL selection follows `contracts/server-info.md` (home/public switching).
* **[contract]** `storage_usage()` uses `GET /photovault/usage`, falling back to `/users/details/v2`.

```rust
#[derive(uniffi::Error)] pub enum CoreError {
  Network { message: String },
  Unreachable { address: String },          // UI: "can't reach {address}, are you on your home Wi-Fi?"
  NotPhotovault { address: String },        // [contract] answers but not /photovault/info nor /ping
  Unauthorized, IncorrectPassword, IncorrectCode,
  Server { status: u16, message: String },
  Crypto { message: String }, Storage { message: String }, NotLoggedIn, Cancelled,
}

#[derive(uniffi::Record)] pub struct ServerInfo {
  name: String, version: String, public_url: String, home_url: Option<String>,
  signups_allowed: bool,                    // [contract]
  is_photovault: bool,                      // [contract] false = stock Ente server (/ping only)
}
#[derive(uniffi::Record)] pub struct StorageUsage {
  used_bytes: u64, limit_bytes: Option<u64>, // None = unlimited
  free_bytes: Option<u64>,                  // [contract]
}
#[derive(uniffi::Record)] pub struct Account { user_id: i64, email: String, server_url: String }
#[derive(uniffi::Enum)] pub enum LoginStep {
  Done { account: Account }, EmailOtt,
  TwoFactorTotp { session_id: String },
  Passkey { session_id: String, url: String },
  NeedsRecoveryKey,
}
#[derive(uniffi::Enum)] pub enum FileKind { Image, Video, LivePhoto, Unknown }
#[derive(uniffi::Record)] pub struct TimelineItem {
  id: i64, collection_id: i64, kind: FileKind, taken_at_us: i64,
  width: u32, height: u32, duration_ms: Option<u32>, has_stream: bool, title: String,
}
#[derive(uniffi::Record)] pub struct Collection {
  id: i64, name: String, kind: String, cover_file_id: Option<i64>, count: u64, updated_at_us: i64,
}
#[derive(uniffi::Record)] pub struct SyncProgress { collections: u32, files: u64, done: bool }
#[derive(uniffi::Record)] pub struct UploadStatus {
  pending: u32, uploading: u32, failed: u32, bytes_done: u64, bytes_total: u64,
}

#[uniffi::export(with_foreign)] pub trait SyncListener: Send + Sync { fn on_progress(&self, p: SyncProgress); }

#[uniffi::export] pub fn discover_servers(timeout_ms: u32) -> Vec<String>;   // best effort; Android uses NSD
#[uniffi::export] pub async fn fetch_server_info(address: String) -> Result<ServerInfo, CoreError>;
#[uniffi::export] pub fn parse_connect_qr(payload: String) -> Option<String>; // [contract] -> server address

#[derive(uniffi::Object)] pub struct Core;
#[uniffi::export] impl Core {
  #[uniffi::constructor] fn new(data_dir: String, cache_dir: String) -> Result<Arc<Self>, CoreError>;
  // servers (survive logout)
  fn saved_servers(&self) -> Vec<ServerInfo>;
  async fn add_server(&self, address: String) -> Result<ServerInfo, CoreError>;
  fn remove_server(&self, public_url: String) -> Result<(), CoreError>;          // [contract]
  fn select_server(&self, public_url: String) -> Result<(), CoreError>;
  fn active_base_url(&self) -> Option<String>;
  async fn refresh_network(&self) -> Option<String>;  // [contract] re-run home/public choice on network change
  // auth
  async fn login(&self, email: String, password: String) -> Result<LoginStep, CoreError>;
  async fn send_email_ott(&self, email: String) -> Result<(), CoreError>;
  async fn verify_email_ott(&self, email: String, code: String, password: String) -> Result<LoginStep, CoreError>;
  async fn verify_totp(&self, session_id: String, code: String, password: String) -> Result<LoginStep, CoreError>;
  async fn recover_with_key(&self, recovery_key: String, new_password: String) -> Result<Account, CoreError>;
  fn current_account(&self) -> Option<Account>;
  async fn logout(&self) -> Result<(), CoreError>;
  async fn storage_usage(&self) -> Result<StorageUsage, CoreError>;
  // library
  async fn sync(&self, listener: Option<Arc<dyn SyncListener>>) -> Result<SyncProgress, CoreError>;
  fn timeline_count(&self) -> u64;
  fn timeline_page(&self, offset: u64, limit: u32) -> Vec<TimelineItem>;   // newest first, local only
  fn collections(&self) -> Vec<Collection>;
  fn collection_page(&self, collection_id: i64, offset: u64, limit: u32) -> Vec<TimelineItem>;
  async fn thumbnail(&self, file_id: i64) -> Result<Vec<u8>, CoreError>;  // decrypted, disk cache encrypted at rest
  async fn download(&self, file_id: i64, dest_path: String) -> Result<String, CoreError>;
  async fn stream_playlist(&self, file_id: i64) -> Result<Option<String>, CoreError>;
  // upload queue (resumable, survives process death)
  fn enqueue_upload(&self, path: String, collection_id: Option<i64>) -> Result<(), CoreError>;
  async fn run_uploads(&self) -> Result<UploadStatus, CoreError>;
  fn upload_status(&self) -> UploadStatus;
}
```

Tests the core track hands back with: SRP and key-derivation against `server/tests/vectors/`, decrypting
files/thumbnails/metadata produced by Ente's web client, a full sign-in + sync + upload + download round
trip against the real server (`cargo test -p photovault-core --features it`), and `uniffi-bindgen` Kotlin
generation in CI. Additive changes (new methods/fields) need a contracts commit; renames do not happen in v1.
