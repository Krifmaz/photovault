# photovault-core UniFFI interface (proposal from the native thread)

Crate: `core/` (package `photovault-core`, lib name `photovault_core`, crate-type cdylib+staticlib+lib), workspace member.
Kotlin package: `app.photovault.core` (uniffi.toml `package_name`). UniFFI 0.32, proc-macros (no UDL).
Native app: `native/android/` Gradle project, applicationId `app.photovault.native`, name "Photovault Native".
All network/disk calls are `async` (exported to Kotlin as `suspend`), run on a tokio runtime inside core. TLS: rustls + ring. Crypto: libsodium-sys-stable.

```rust
#[derive(uniffi::Error)] pub enum CoreError {
  Network{message:String}, Unreachable{address:String},    // "can't reach 10.0.1.7:8095"
  Unauthorized, IncorrectPassword, IncorrectCode, Server{status:u16,message:String},
  Crypto{message:String}, Storage{message:String}, NotLoggedIn, Cancelled }

#[derive(uniffi::Record)] pub struct ServerInfo { name:String, version:String, public_url:String, home_url:Option<String> } // GET /photovault/info
#[derive(uniffi::Record)] pub struct StorageUsage { used_bytes:u64, limit_bytes:Option<u64> }        // None = unlimited
#[derive(uniffi::Record)] pub struct Account { user_id:i64, email:String, server_url:String }
#[derive(uniffi::Enum)]  pub enum LoginStep {
  Done{account:Account}, EmailOtt, TwoFactorTotp{session_id:String},
  Passkey{session_id:String, url:String}, NeedsRecoveryKey }   // key attrs need recovery
#[derive(uniffi::Enum)]  pub enum FileKind { Image, Video, LivePhoto, Unknown }
#[derive(uniffi::Record)] pub struct TimelineItem { id:i64, collection_id:i64, kind:FileKind,
  taken_at_us:i64, width:u32, height:u32, duration_ms:Option<u32>, has_stream:bool, title:String }
#[derive(uniffi::Record)] pub struct Collection { id:i64, name:String, kind:String, cover_file_id:Option<i64>, count:u64, updated_at_us:i64 }
#[derive(uniffi::Record)] pub struct SyncProgress { collections:u32, files:u64, done:bool }
#[derive(uniffi::Record)] pub struct UploadStatus { pending:u32, uploading:u32, failed:u32, bytes_done:u64, bytes_total:u64 }

#[uniffi::export(with_foreign)] pub trait SyncListener: Send+Sync { fn on_progress(&self, p:SyncProgress); }
#[uniffi::export] pub fn discover_servers(timeout_ms:u32) -> Vec<String>;   // mDNS _photovault._tcp (best effort; Android NSD may be used instead)
#[uniffi::export] pub async fn fetch_server_info(address:String) -> Result<ServerInfo,CoreError>;

#[derive(uniffi::Object)] pub struct Core;
#[uniffi::export] impl Core {
  #[uniffi::constructor] fn new(data_dir:String, cache_dir:String) -> Result<Arc<Self>,CoreError>; // opens encrypted local store
  // servers (survive logout)
  fn saved_servers(&self) -> Vec<ServerInfo>; async fn add_server(&self, address:String) -> Result<ServerInfo,CoreError>;
  fn select_server(&self, public_url:String) -> Result<(),CoreError>; fn active_base_url(&self) -> Option<String>; // home/public auto-switch
  // auth
  async fn login(&self, email:String, password:String) -> Result<LoginStep,CoreError>;   // SRP, falls back to email OTT if no SRP
  async fn send_email_ott(&self, email:String) -> Result<(),CoreError>;
  async fn verify_email_ott(&self, email:String, code:String, password:String) -> Result<LoginStep,CoreError>;
  async fn verify_totp(&self, session_id:String, code:String, password:String) -> Result<LoginStep,CoreError>;
  async fn recover_with_key(&self, recovery_key:String /* 24-word mnemonic or hex */, new_password:String) -> Result<Account,CoreError>;
  fn current_account(&self) -> Option<Account>; async fn logout(&self) -> Result<(),CoreError>;
  async fn storage_usage(&self) -> Result<StorageUsage,CoreError>;
  // library
  async fn sync(&self, listener:Option<Arc<dyn SyncListener>>) -> Result<SyncProgress,CoreError>;
  fn timeline_count(&self) -> u64; fn timeline_page(&self, offset:u64, limit:u32) -> Vec<TimelineItem>; // newest first, local DB only (fast)
  fn collections(&self) -> Vec<Collection>; fn collection_page(&self, collection_id:i64, offset:u64, limit:u32) -> Vec<TimelineItem>;
  async fn thumbnail(&self, file_id:i64) -> Result<Vec<u8>,CoreError>;      // decrypted JPEG/WebP bytes, disk-cached (encrypted at rest)
  async fn download(&self, file_id:i64, dest_path:String) -> Result<String,CoreError>; // decrypted original, returns path
  async fn stream_playlist(&self, file_id:i64) -> Result<Option<String>,CoreError>;  // Ente streamable-video HLS, local decrypted m3u8 path
  // upload queue (resumable)
  fn enqueue_upload(&self, path:String, collection_id:Option<i64>) -> Result<(),CoreError>;
  async fn run_uploads(&self) -> Result<UploadStatus,CoreError>; fn upload_status(&self) -> UploadStatus;
}
```
Notes: Kotlin calls everything off the main thread (suspend + Dispatchers.IO for sync fns). Coil fetcher calls `thumbnail(id)`.
Local store: SQLite (rusqlite bundled) with fields encrypted under a device key; master key kept in Android Keystore-wrapped blob passed in later (v1: secretbox under a key file in app-private storage).
