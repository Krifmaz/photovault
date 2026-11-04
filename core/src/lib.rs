//! photovault-core: the client core of Photovault's native apps.
//!
//! Implements `contracts/photovault-core.md` (contract v1) and exports it
//! with UniFFI (Kotlin package `app.photovault.core`). Everything a client
//! needs that touches keys lives here: Ente's end-to-end encryption scheme
//! byte for byte (libsodium; Argon2id KEK, secretbox key wrapping,
//! secretstream file/thumbnail/metadata encryption, sealed boxes, SRP-6a),
//! login with SRP / email code / TOTP / recovery key, the Ente API, sync into
//! an encrypted local store, thumbnails, downloads and a resumable upload
//! queue. An account works the same here and in the stock Ente apps.
//!
//! Threading: every `async` function runs on a tokio runtime owned by the
//! core (see [`runtime`]); the synchronous functions only read the local
//! store and its in-memory index.

#![warn(missing_docs)]

mod albums;
pub mod api;
mod auth;
mod client;
pub mod crypto;
mod library;
pub mod media;
pub mod metadata;
pub mod model;
mod server;
pub mod srp;
mod store;
mod upload;

use std::future::Future;
use std::sync::{Arc, OnceLock};

uniffi::setup_scaffolding!();

pub use crate::albums::{
    AlbumDetails, PublicLink, PublicLinkUpdate, ShareRole, Sharee, TrashEntry,
};
pub use crate::server::{normalize_address, parse_connect_qr_url};

/// Errors of the core, as the apps see them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    /// A network failure other than "cannot connect" (reset, timeout, TLS, bad answer).
    #[error("network error: {message}")]
    Network {
        /// Details.
        message: String,
    },
    /// The server cannot be reached at all. UI: "can't reach {address}, are you on your home Wi-Fi?"
    #[error("can't reach {address}")]
    Unreachable {
        /// The address that was tried.
        address: String,
    },
    /// Something answers at the address, but neither `/photovault/info` nor `/ping`.
    #[error("{address} is not a Photovault server")]
    NotPhotovault {
        /// The address that was tried.
        address: String,
    },
    /// The session token was rejected (logged out elsewhere).
    #[error("unauthorized")]
    Unauthorized,
    /// Wrong email or password.
    #[error("incorrect password")]
    IncorrectPassword,
    /// Wrong or expired code (email code, TOTP, recovery key).
    #[error("incorrect code")]
    IncorrectCode,
    /// Any other non-success answer.
    #[error("server error {status}: {message}")]
    Server {
        /// HTTP status.
        status: u16,
        /// Server message.
        message: String,
    },
    /// Decryption or encryption failed.
    #[error("crypto error: {message}")]
    Crypto {
        /// Details.
        message: String,
    },
    /// Local store or file system error.
    #[error("storage error: {message}")]
    Storage {
        /// Details.
        message: String,
    },
    /// The call needs a signed-in account.
    #[error("not logged in")]
    NotLoggedIn,
    /// The call was cancelled.
    #[error("cancelled")]
    Cancelled,
}

impl CoreError {
    pub(crate) fn crypto(m: impl Into<String>) -> Self {
        Self::Crypto { message: m.into() }
    }
    pub(crate) fn storage(m: impl std::fmt::Display) -> Self {
        Self::Storage {
            message: m.to_string(),
        }
    }
    /// A transport failure (connection reset, timeout, TLS). Only for
    /// problems a retry can fix: malformed answers are [`CoreError::protocol`].
    pub(crate) fn network(m: impl std::fmt::Display) -> Self {
        Self::Network {
            message: m.to_string(),
        }
    }
    /// The server answered, but with something unusable (unparseable JSON,
    /// missing fields). Reported as `Server { status: 0 }`: retrying the
    /// same request will not help.
    pub(crate) fn protocol(m: impl std::fmt::Display) -> Self {
        Self::Server {
            status: 0,
            message: m.to_string(),
        }
    }
    /// HTTP status of a server answer, if this is one.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Server { status, .. } => Some(*status),
            Self::Unauthorized => Some(401),
            _ => None,
        }
    }
}

impl From<crypto::CryptoError> for CoreError {
    fn from(e: crypto::CryptoError) -> Self {
        Self::crypto(e.to_string())
    }
}
/// Local (de)serialisation (the store, request bodies). Server answers are
/// mapped to [`CoreError::protocol`] and decrypted payloads to
/// [`CoreError::Crypto`] where they are parsed.
impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::storage(format!("bad JSON: {e}"))
    }
}
impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::storage(e)
    }
}
impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        Self::storage(e)
    }
}

/// Result alias.
pub type Result<T, E = CoreError> = std::result::Result<T, E>;

/// A server, as `/photovault/info` describes it (contracts/server-info.md).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct ServerInfo {
    /// Display name; empty for a stock Ente-compatible server.
    pub name: String,
    /// Server version; empty for a stock server.
    pub version: String,
    /// Public URL, no trailing slash. Saved servers are keyed by it.
    pub public_url: String,
    /// LAN URL, used when it answers (home/public switching).
    pub home_url: Option<String>,
    /// Whether "Create account" should be offered.
    pub signups_allowed: bool,
    /// False for a stock Ente-compatible server (answers only `/ping`).
    pub is_photovault: bool,
}

/// Storage numbers (contracts/storage-usage.md).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StorageUsage {
    /// Bytes used.
    pub used_bytes: u64,
    /// Limit; `None` = unlimited.
    pub limit_bytes: Option<u64>,
    /// Free bytes; `None` = unlimited.
    pub free_bytes: Option<u64>,
}

/// The signed-in account.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct Account {
    /// User id.
    pub user_id: i64,
    /// Email.
    pub email: String,
    /// Public URL of the account's server.
    pub server_url: String,
}

/// What a login step needs next.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum LoginStep {
    /// Signed in.
    Done {
        /// The account.
        account: Account,
    },
    /// A code was emailed: call `verify_email_ott`.
    EmailOtt,
    /// A TOTP code is needed: call `verify_totp`.
    TwoFactorTotp {
        /// Second-factor session.
        session_id: String,
    },
    /// A passkey is needed; open `url` in a browser, then call `verify_passkey`.
    Passkey {
        /// Passkey session.
        session_id: String,
        /// Where the passkey ceremony happens.
        url: String,
    },
    /// The first factors passed but no password was given: call `recover_with_key`.
    NeedsRecoveryKey,
}

/// Kind of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FileKind {
    /// A still image.
    Image,
    /// A video.
    Video,
    /// A Live Photo (image + video bundle).
    LivePhoto,
    /// Anything else.
    Unknown,
}

/// One entry of the timeline or of an album.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TimelineItem {
    /// File id (stable: use it as the list key).
    pub id: i64,
    /// A collection the file is in (the album for `collection_page`).
    pub collection_id: i64,
    /// Kind.
    pub kind: FileKind,
    /// Capture time (edited time if set), µs since the epoch.
    pub taken_at_us: i64,
    /// Width in pixels, 0 if unknown.
    pub width: u32,
    /// Height in pixels, 0 if unknown.
    pub height: u32,
    /// Video duration.
    pub duration_ms: Option<u32>,
    /// A streamable (HLS) version exists.
    pub has_stream: bool,
    /// File name (edited name if set).
    pub title: String,
}

/// An album or other collection.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Collection {
    /// Collection id.
    pub id: i64,
    /// Decrypted name.
    pub name: String,
    /// `album`, `folder`, `favorites` or `uncategorized`.
    pub kind: String,
    /// Newest file, for the cover.
    pub cover_file_id: Option<i64>,
    /// Number of files.
    pub count: u64,
    /// Last change, µs since the epoch.
    pub updated_at_us: i64,
}

/// Progress of a sync.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncProgress {
    /// Collections synced so far.
    pub collections: u32,
    /// Files in the local library.
    pub files: u64,
    /// The sync finished.
    pub done: bool,
}

/// State of the upload queue.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct UploadStatus {
    /// Waiting.
    pub pending: u32,
    /// In progress.
    pub uploading: u32,
    /// Failed (retried by the next `run_uploads`).
    pub failed: u32,
    /// Bytes uploaded in the current queue.
    pub bytes_done: u64,
    /// Bytes in the current queue.
    pub bytes_total: u64,
}

/// Receives sync progress (implemented in Kotlin).
#[uniffi::export(with_foreign)]
pub trait SyncListener: Send + Sync {
    /// Called after each collection and at the end.
    fn on_progress(&self, p: SyncProgress);
}

/// The tokio runtime that runs all network and disk work of the core.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("photovault-core")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Run CPU-heavy synchronous work (index rebuilds, SRP modpows, cache
/// walks) without stalling the async worker it is called from: on the
/// core's multi-threaded runtime the worker hands its other tasks to the
/// rest of the pool for the duration (`block_in_place`). Elsewhere (unit
/// tests on a current-thread runtime, plain threads) `f` just runs.
pub(crate) fn offload<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

/// Aborts the spawned task when the caller's future is dropped (a cancelled
/// Kotlin coroutine).
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Run `fut` on the core's runtime and await it from any executor.
pub(crate) async fn run<T: Send + 'static>(
    fut: impl Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    let mut h = AbortOnDrop(runtime().spawn(fut));
    match (&mut h.0).await {
        Ok(r) => r,
        Err(e) if e.is_cancelled() => Err(CoreError::Cancelled),
        Err(e) => Err(CoreError::storage(format!("task failed: {e}"))),
    }
}

/// Look for Photovault servers on the LAN (`_photovault._tcp`) for up to
/// `timeout_ms`. Best effort: returns addresses to pass to `add_server`
/// (the home URL when advertised). Android may use NSD instead.
#[uniffi::export]
pub fn discover_servers(timeout_ms: u32) -> Vec<String> {
    server::discover(timeout_ms)
}

/// Describe the server at `address` (`/photovault/info`, falling back to
/// `/ping` for a stock Ente-compatible server).
#[uniffi::export]
pub async fn fetch_server_info(address: String) -> Result<ServerInfo, CoreError> {
    run(async move { server::fetch_server_info(&address).await }).await
}

/// The server address in a "Connect" QR code
/// (`photovault://connect?url=…&home=…&name=…`, or a bare http(s) URL).
#[uniffi::export]
pub fn parse_connect_qr(payload: String) -> Option<String> {
    parse_connect_qr_url(&payload)
}

/// The client core: one per app process.
#[derive(uniffi::Object)]
pub struct Core {
    inner: Arc<client::Inner>,
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core").finish_non_exhaustive()
    }
}

#[uniffi::export]
impl Core {
    /// Open (or create) the local store in `data_dir` (app-private) and the
    /// encrypted cache in `cache_dir`.
    #[uniffi::constructor]
    ///
    /// The store is sealed under a `device.key` file in `data_dir`; apps
    /// should use [`Core::with_device_key`] instead.
    pub fn new(data_dir: String, cache_dir: String) -> Result<Arc<Self>, CoreError> {
        let inner = client::Inner::open(data_dir.into(), cache_dir.into(), None)?;
        Ok(Arc::new(Self { inner }))
    }

    /// Like [`Core::new`], but the store is sealed under `device_key`
    /// (32 bytes), which the app keeps wrapped by the platform keystore and
    /// never writes in the clear. A store left by [`Core::new`] is re-sealed
    /// under it and its `device.key` deleted. If the store was sealed under
    /// a different key (keystore reset), its account data is dropped and
    /// the user signs in again; saved servers are kept.
    #[uniffi::constructor]
    pub fn with_device_key(
        data_dir: String,
        cache_dir: String,
        device_key: Vec<u8>,
    ) -> Result<Arc<Self>, CoreError> {
        let inner = client::Inner::open(data_dir.into(), cache_dir.into(), Some(device_key))?;
        Ok(Arc::new(Self { inner }))
    }

    // ---- servers (survive logout) ----

    /// Saved servers, oldest first.
    pub fn saved_servers(&self) -> Vec<ServerInfo> {
        self.inner.saved_servers()
    }

    /// Look up and save the server at `address`; it becomes the active
    /// server if none is selected.
    pub async fn add_server(&self, address: String) -> Result<ServerInfo, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.add_server(&address).await }).await
    }

    /// Forget a saved server.
    pub fn remove_server(&self, public_url: String) -> Result<(), CoreError> {
        self.inner.remove_server(&public_url)
    }

    /// Make a saved server the active one (re-checks home/public in the background).
    pub fn select_server(&self, public_url: String) -> Result<(), CoreError> {
        self.inner.select_server(&public_url)
    }

    /// The base URL requests go to now (home or public URL of the active server).
    pub fn active_base_url(&self) -> Option<String> {
        self.inner.active_base_url()
    }

    /// Whether the user allowed the plain-http home URL of this saved server.
    /// An `https://` home URL needs no approval (its certificate is checked).
    pub fn home_url_allowed(&self, public_url: String) -> bool {
        self.inner.home_url_allowed(&public_url)
    }

    /// Allow (or stop allowing) requests, including sign-in and the session
    /// token, to go to this server's plain-http home URL when it answers.
    /// Off by default: on another network the same private address can be a
    /// different machine.
    pub fn set_home_url_allowed(&self, public_url: String, allowed: bool) -> Result<(), CoreError> {
        self.inner.set_home_url_allowed(&public_url, allowed)
    }

    /// Re-run the home/public choice (call on network change). Returns the new base URL.
    pub async fn refresh_network(&self) -> Option<String> {
        let inner = self.inner.clone();
        run(async move { Ok(inner.refresh_network().await) })
            .await
            .ok()
            .flatten()
    }

    // ---- auth ----

    /// Sign in with email and password (SRP; email code if the account has
    /// no SRP or uses email 2FA).
    pub async fn login(&self, email: String, password: String) -> Result<LoginStep, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.login(&email, &password).await }).await
    }

    /// Email a login code.
    pub async fn send_email_ott(&self, email: String) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.send_email_ott(&email).await }).await
    }

    /// Check an emailed code. With an empty `password` (forgot password) the
    /// step after the second factor is `NeedsRecoveryKey`.
    pub async fn verify_email_ott(
        &self,
        email: String,
        code: String,
        password: String,
    ) -> Result<LoginStep, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.verify_email_ott(&email, &code, &password).await }).await
    }

    /// Check a TOTP code. With an empty `password`, see `verify_email_ott`.
    pub async fn verify_totp(
        &self,
        session_id: String,
        code: String,
        password: String,
    ) -> Result<LoginStep, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.verify_totp(&session_id, &code, &password).await }).await
    }

    /// After `Passkey`: once the passkey ceremony in the browser finished
    /// (the `photovault://passkey` redirect, or the user says so), fetch the
    /// verified session. Returns the same `Passkey` step while it is not
    /// verified yet. With an empty `password`, see `verify_email_ott`.
    pub async fn verify_passkey(
        &self,
        session_id: String,
        password: String,
    ) -> Result<LoginStep, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.verify_passkey(&session_id, &password).await }).await
    }

    /// After `NeedsRecoveryKey`: unlock with the recovery key (24 words or
    /// hex) and set `new_password`, as Ente's apps do.
    pub async fn recover_with_key(
        &self,
        recovery_key: String,
        new_password: String,
    ) -> Result<Account, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.recover_with_key(&recovery_key, &new_password).await }).await
    }

    /// The signed-in account.
    pub fn current_account(&self) -> Option<Account> {
        self.inner.current_account()
    }

    /// Sign out: revoke the token (best effort) and wipe the account's local data.
    pub async fn logout(&self) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.logout().await }).await
    }

    /// Storage used and available.
    pub async fn storage_usage(&self) -> Result<StorageUsage, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.storage_usage().await }).await
    }

    // ---- library ----

    /// Pull collection and file changes into the local store.
    pub async fn sync(
        &self,
        listener: Option<Arc<dyn SyncListener>>,
    ) -> Result<SyncProgress, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.sync(listener).await }).await
    }

    /// Number of files in the timeline.
    pub fn timeline_count(&self) -> u64 {
        self.inner.timeline_count()
    }

    /// A page of the timeline, newest first (local only).
    pub fn timeline_page(&self, offset: u64, limit: u32) -> Vec<TimelineItem> {
        self.inner.timeline_page(offset, limit)
    }

    /// Visible collections, most recently changed first.
    pub fn collections(&self) -> Vec<Collection> {
        self.inner.collections()
    }

    /// A page of one collection, newest first (local only).
    pub fn collection_page(
        &self,
        collection_id: i64,
        offset: u64,
        limit: u32,
    ) -> Vec<TimelineItem> {
        self.inner.collection_page(collection_id, offset, limit)
    }

    /// The decrypted thumbnail (JPEG). Cached on disk, encrypted.
    pub async fn thumbnail(&self, file_id: i64) -> Result<Vec<u8>, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.thumbnail(file_id).await }).await
    }

    /// Download and decrypt the original to `dest_path`; returns the path.
    pub async fn download(&self, file_id: i64, dest_path: String) -> Result<String, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.download(file_id, &dest_path).await }).await
    }

    /// A local, decrypted HLS playlist of the streamable version, if any.
    pub async fn stream_playlist(&self, file_id: i64) -> Result<Option<String>, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.stream_playlist(file_id).await }).await
    }

    // ---- uploads ----

    /// Queue `path` for upload into `collection_id` (or the uncategorized
    /// collection). The queue survives process death.
    pub fn enqueue_upload(
        &self,
        path: String,
        collection_id: Option<i64>,
    ) -> Result<(), CoreError> {
        self.inner.enqueue_upload(&path, collection_id)
    }

    /// Upload everything queued; returns the final status.
    pub async fn run_uploads(&self) -> Result<UploadStatus, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.run_uploads().await }).await
    }

    /// The queue's current state.
    pub fn upload_status(&self) -> UploadStatus {
        self.inner.upload_status()
    }
}
