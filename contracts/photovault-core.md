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

## v1.2: signup

Create account, exactly as Ente's apps do it (`rust/crates/accounts/src/signup.rs`, `auth/key_gen.rs`;
the photos app's `UserService.generateKey` / `setAttributes`). Module `core/src/signup.rs`, its own
`#[uniffi::export] impl Core` block. No new records or error variants: failures use the v1 `CoreError`.

```rust
#[uniffi::export] impl Core {
  // POST /users/ott {email, purpose: "signup", mobile: true}. Refused before sending when the active
  // server's ServerInfo.signups_allowed is false (asked fresh from /photovault/info, the saved copy if
  // that does not answer): Server { status: 403 }. Email already registered: Server { status: 409 }.
  async fn signup_send_code(&self, email: String) -> Result<(), CoreError>;
  // POST /users/verify-email. Wrong/expired code: IncorrectCode. An answer with key attributes, a sealed
  // token or a second factor is an existing account: Server { status: 409 }. Keeps {email, id, token}
  // in memory (never on disk) for the next steps.
  async fn signup_verify_code(&self, email: String, code: String) -> Result<(), CoreError>;
  // Makes the keys on the device and returns the recovery key as 24 BIP39 (English) words. Nothing is
  // uploaded. masterKey, recoveryKey: 32 random bytes; masterKeyEncryptedWithRecoveryKey and
  // recoveryKeyEncryptedWithMasterKey: secretbox (+ nonces). KEK: Argon2id over the password with a 16-byte
  // kekSalt, Ente's deriveSensitiveKey: 1 GiB / 4 ops, or 256 MiB / 16 ops when `low_memory` (Ente's
  // low-spec start), halving memory and doubling ops while the derivation fails, down to 128 MiB (the
  // server minimum); mem x ops is always 4 GiB. encryptedKey = secretbox(masterKey, KEK). X25519 key pair;
  // encryptedSecretKey = secretbox(secretKey, masterKey). SRP: login sub-key of the KEK, srpUserID = UUID v4,
  // 16-byte srpSalt, verifier. The same password again returns the same words; another password makes new
  // keys (until they are uploaded).
  async fn signup_prepare(&self, password: String, low_memory: bool) -> Result<String, CoreError>;
  // After the user saved the recovery key: GET /users/session-validity/v2, PUT /users/attributes
  // {keyAttributes} (skipped if the same attributes are already there), POST /users/srp/setup +
  // POST /users/srp/complete (server M2 checked), GET /users/srp/attributes must match (srpUserID, srpSalt,
  // kekSalt, memLimit, opsLimit). Then the account is signed in on this device (as after `login`).
  // Safe to retry after a failure.
  async fn signup_finish(&self) -> Result<Account, CoreError>;
  // The signed-in account's recovery key as 24 words (recoveryKeyEncryptedWithMasterKey opened with the
  // master key). Local only.
  fn recovery_key_words(&self) -> Result<String, CoreError>;
}
```

Apps pass `low_memory = ActivityManager.isLowRamDevice() || totalMem < 4 GiB || availMem < 1.5 GiB` on
Android. Tests: `core/tests/signup.rs` (sign-up through core against the in-process server, SRP login from
a second core and from a hand-rolled SRP client, the key attributes opened with Ente's steps, checked
against Ente's own `vectors/keys.json` + `sealed_token.json`, recovery with the shown words, closed sign-ups).
