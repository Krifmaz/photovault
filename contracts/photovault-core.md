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
  secretbox under a device key. **[contract v1.1]** Apps pass that key to `Core::with_device_key` and keep
  it wrapped by the platform keystore (Android Keystore AES-GCM); `Core::new` (legacy `device.key` file in
  the data dir) remains for tests and tools.
* **[contract v1.1]** A plain-http `homeUrl` is only used (it carries sign-in and the session token) after
  the user allowed it per server (`set_home_url_allowed`); an `https://` home URL needs no approval.
  Authenticated requests always go to the signed-in account's own server, whatever server is selected.
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
  // [contract v1.1] the store is sealed under `device_key` (32 bytes, kept wrapped by the platform keystore)
  #[uniffi::constructor] fn with_device_key(data_dir: String, cache_dir: String, device_key: Vec<u8>) -> Result<Arc<Self>, CoreError>;
  // servers (survive logout)
  fn saved_servers(&self) -> Vec<ServerInfo>;
  async fn add_server(&self, address: String) -> Result<ServerInfo, CoreError>;
  fn remove_server(&self, public_url: String) -> Result<(), CoreError>;          // [contract]
  fn select_server(&self, public_url: String) -> Result<(), CoreError>;
  fn active_base_url(&self) -> Option<String>;
  async fn refresh_network(&self) -> Option<String>;  // [contract] re-run home/public choice on network change
  fn home_url_allowed(&self, public_url: String) -> bool;                                    // [contract v1.1]
  fn set_home_url_allowed(&self, public_url: String, allowed: bool) -> Result<(), CoreError>; // [contract v1.1]
  // auth
  async fn login(&self, email: String, password: String) -> Result<LoginStep, CoreError>;
  async fn send_email_ott(&self, email: String) -> Result<(), CoreError>;
  async fn verify_email_ott(&self, email: String, code: String, password: String) -> Result<LoginStep, CoreError>;
  async fn verify_totp(&self, session_id: String, code: String, password: String) -> Result<LoginStep, CoreError>;
  // [contract v1.1] after Passkey: GET /users/two-factor/passkeys/get-token; returns Passkey again until verified
  async fn verify_passkey(&self, session_id: String, password: String) -> Result<LoginStep, CoreError>;
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

## v1.2: search, faces, memories

Additive. Code in `core/src/ml/` (one `#[uniffi::export] impl Core` block per file); Rust types are in
`core/src/ml/mod.rs`. Everything uses Ente's own data, so it is shared with the stock Ente apps and the
compute node both ways:

* **`mldata`** (face + CLIP indexes per file): Ente's format exactly (`core/src/ml/mldata.rs`, ported from
  the compute node; fixtures in `core/tests/ml_fixtures/`): JSON, gzipped, encrypted with the file key as
  one secretstream blob, `PUT /files/data` with `type = "mldata"`; pulled with `/files/data/status-diff`
  and `/files/data/fetch`. Unknown keys are kept on rewrite. Versions: face 1, CLIP 1. `client` =
  `app.photovault.native/<version>`. Flags: `RUNTIME_RUST` (+ `WEBGPU` when used).
* **Models** come only from the user's server (`GET /photovault/models/<file>`, PR #14), verified against
  the size and SHA-256 Ente pins. Inference is Ente's `rust/crates/ml` (`ente-ml`) on ONNX Runtime (cargo
  feature `ml`; the app ships Ente's ONNX Runtime AAR, `ort` loads `libonnxruntime.so` dynamically).
* **People** are `cgroup` user entities (`/user-entity/...`, per-type entity key wrapped with the master
  key), clustering follows Ente's algorithm and thresholds; location tags are `location` entities.
* The ML on/off switch is Ente's remote flag `faceSearchEnabled` (`/remote-store`).
* Local tables `ml_data`, `ml_local`, `ml_entities` (`core/src/ml/store.rs`) hold sealed JSON only.
* `on_this_day(now_us, utc_offset_secs, limit)` (map/widgets track) stays; `memories` is the richer API.

```rust
#[derive(uniffi::Record)] pub struct MlStatus { enabled: bool, inference_available: bool, models_ready: bool,
  model_bytes_done: u64, model_bytes_total: u64, files_total: u64, files_indexed: u64, files_pending: u64,
  running: bool, last_error: Option<String> }
#[uniffi::export(with_foreign)] pub trait MlListener: Send + Sync { fn on_status(&self, status: MlStatus); }
#[derive(uniffi::Enum)] pub enum SearchKind { Magic, Person, Album, Date, Place, FileName, Caption, FileType }
#[derive(uniffi::Record)] pub struct SearchSuggestion { kind: SearchKind, key: String, label: String, count: u64,
  cover_file_id: Option<i64>, cover_face_id: Option<String> }
#[derive(uniffi::Record)] pub struct FaceRef { face_id: String, file_id: i64, x: f32, y: f32, width: f32,
  height: f32, score: f32, blur: f32 }                       // box normalised to [0, 1]
#[derive(uniffi::Record)] pub struct Person { id: String,    // cgroup id, or "cluster:<id>" when unnamed
  name: Option<String>, is_hidden: bool, file_count: u64, cover: Option<FaceRef>, birth_date: Option<String> }
#[derive(uniffi::Record)] pub struct FileFace { face: FaceRef, person_id: Option<String>, person_name: Option<String> }
#[derive(uniffi::Enum)] pub enum MemoryKind { OnThisDay, People, Trip, Time, Clip, Filler }
#[derive(uniffi::Record)] pub struct Memory { id: String, kind: MemoryKind, title: String, subtitle: Option<String>,
  items: Vec<TimelineItem>, cover_file_id: i64, seen: bool, person_id: Option<String> }

impl Core {
  // ML control (index.rs)
  fn ml_status(&self) -> MlStatus;
  async fn set_ml_enabled(&self, enabled: bool) -> Result<MlStatus, CoreError>;
  async fn sync_ml(&self, listener: Option<Arc<dyn MlListener>>) -> Result<MlStatus, CoreError>;     // after each sync
  async fn run_ml_indexing(&self, max_files: u32, listener: Option<Arc<dyn MlListener>>) -> Result<MlStatus, CoreError>;
  // search (search.rs)
  fn search_suggestions(&self, query: String, limit: u32) -> Vec<SearchSuggestion>;
  fn search_discovery(&self) -> Vec<SearchSuggestion>;
  async fn search_results(&self, kind: SearchKind, key: String) -> Result<Vec<TimelineItem>, CoreError>;
  async fn magic_search(&self, query: String, limit: u32) -> Result<Vec<TimelineItem>, CoreError>;
  // people (faces.rs)
  fn people(&self, include_hidden: bool, min_files: u32) -> Vec<Person>;
  fn person(&self, person_id: String) -> Option<Person>;
  fn person_files(&self, person_id: String) -> Vec<TimelineItem>;
  fn file_faces(&self, file_id: i64) -> Vec<FileFace>;
  fn person_suggestions(&self, person_id: String) -> Vec<Person>;
  async fn face_thumbnail(&self, face_id: String) -> Result<Vec<u8>, CoreError>;           // square JPEG
  async fn name_person(&self, person_id: String, name: String) -> Result<Person, CoreError>;
  async fn merge_people(&self, into_id: String, from_id: String) -> Result<Person, CoreError>;
  async fn answer_suggestion(&self, person_id: String, cluster_id: String, accept: bool) -> Result<(), CoreError>;
  async fn set_person_hidden(&self, person_id: String, hidden: bool) -> Result<(), CoreError>;
  async fn set_person_cover(&self, person_id: String, face_id: String) -> Result<(), CoreError>;
  async fn set_person_birth_date(&self, person_id: String, birth_date: Option<String>) -> Result<(), CoreError>;
  async fn remove_faces_from_person(&self, person_id: String, face_ids: Vec<String>) -> Result<(), CoreError>;
  async fn delete_person(&self, person_id: String) -> Result<(), CoreError>;
  // memories (memories.rs)
  fn memories(&self, now_us: i64, utc_offset_secs: i32) -> Vec<Memory>;
  fn mark_memory_seen(&self, memory_id: String) -> Result<(), CoreError>;
}
```
