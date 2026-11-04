//! The core's state: local store, session, active server, library index.
//! Login lives in [`crate::auth`], sync and reads in [`crate::library`],
//! uploads in [`crate::upload`].

use crate::api::Api;
use crate::library::Index;
use crate::model::KeyAttributes;
use crate::store::Store;
use crate::{Account, CoreError, Result, ServerInfo, server};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

/// kv key of the active server's public URL (plain).
pub(crate) const KV_ACTIVE_SERVER: &str = "active_server";
/// kv key prefix of "the user allows this server's plain-http home URL"
/// (`home_allowed:<public url>` = `1`, plain).
pub(crate) const KV_HOME_ALLOWED: &str = "home_allowed:";
/// kv key of the sealed [`Session`].
pub(crate) const KV_SESSION: &str = "session";

/// The secrets of the signed-in account. Stored sealed under the device key.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub account: Account,
    /// `X-Auth-Token`.
    pub token: String,
    pub master_key: Vec<u8>,
    pub secret_key: Vec<u8>,
    pub public_key: Vec<u8>,
    pub key_attributes: KeyAttributes,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        crate::crypto::wipe(&mut self.master_key);
        crate::crypto::wipe(&mut self.secret_key);
    }
}

/// Live progress of the upload in flight.
#[derive(Debug, Default)]
pub(crate) struct LiveUpload {
    pub row: Option<i64>,
    pub bytes_done: u64,
}

/// Where requests go now: the active server's public URL and the base URL
/// chosen for it (home or public). Kept together so a request for one
/// server can never pick up another server's base.
#[derive(Debug, Clone)]
pub(crate) struct Base {
    pub public: String,
    pub url: String,
}

impl Base {
    fn public(public: &str) -> Self {
        Self {
            public: public.to_string(),
            url: public.to_string(),
        }
    }
}

/// Everything behind [`crate::Core`].
pub struct Inner {
    pub(crate) cache_dir: PathBuf,
    store: Mutex<Store>,
    pub(crate) session: RwLock<Option<Arc<Session>>>,
    pub(crate) pending: Mutex<Option<crate::auth::Pending>>,
    /// A create-account flow between its steps (`crate::signup`).
    pub(crate) signup: Mutex<Option<crate::signup::PendingSignup>>,
    base: RwLock<Option<Base>>,
    pub(crate) index: RwLock<Arc<Index>>,
    pub(crate) sync_lock: tokio::sync::Mutex<()>,
    pub(crate) upload_lock: tokio::sync::Mutex<()>,
    pub(crate) live_upload: Mutex<LiveUpload>,
    /// Search, faces and memories state.
    pub(crate) ml: crate::ml::MlState,
}

impl Inner {
    /// Open the store and restore the session and index.
    /// `device_key`: the 32-byte key that seals the store, supplied (and
    /// kept wrapped) by the platform; `None` uses the legacy `device.key`
    /// file (tests, desktop tools).
    pub fn open(
        data_dir: PathBuf,
        cache_dir: PathBuf,
        device_key: Option<Vec<u8>>,
    ) -> Result<Arc<Self>> {
        crate::crypto::init();
        std::fs::create_dir_all(&cache_dir)?;
        let (store, reset) = Store::open_with(&data_dir, device_key)?;
        let ml = crate::ml::MlState::new(&data_dir);
        if reset {
            // The store was sealed under a key we no longer have: its account
            // data is unreadable. Drop the encrypted object cache with it.
            for d in ["thumbs", "files", crate::ml::faces::CROP_CACHE_DIR] {
                let _ = std::fs::remove_dir_all(cache_dir.join(d));
            }
        }
        let session: Option<Session> = store.get_sealed(KV_SESSION)?;
        let active = store.get_plain(KV_ACTIVE_SERVER)?;
        let inner = Arc::new(Self {
            cache_dir,
            store: Mutex::new(store),
            session: RwLock::new(session.map(Arc::new)),
            pending: Mutex::new(None),
            signup: Mutex::new(None),
            base: RwLock::new(active.as_deref().map(Base::public)),
            index: RwLock::new(Arc::new(Index::default())),
            sync_lock: tokio::sync::Mutex::new(()),
            upload_lock: tokio::sync::Mutex::new(()),
            live_upload: Mutex::new(LiveUpload::default()),
            ml,
        });
        inner.rebuild_index()?;
        if active.is_some() {
            let me = inner.clone();
            crate::runtime().spawn(async move {
                me.refresh_network().await;
            });
        }
        Ok(inner)
    }

    /// Lock the store.
    pub(crate) fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The current session.
    pub(crate) fn session(&self) -> Result<Arc<Session>> {
        self.session
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or(CoreError::NotLoggedIn)
    }

    /// A client for the active base URL, without a token.
    pub(crate) fn anon_api(&self) -> Result<Api> {
        let base = self.active_base_url().ok_or_else(|| CoreError::Network {
            message: "no server selected".into(),
        })?;
        Ok(Api::new(&base))
    }

    /// A client for the signed-in account's server, with the session token.
    /// The token only ever goes to that server: its home URL when that is
    /// the current choice for it, otherwise its public URL, never another
    /// server the user selected meanwhile.
    pub(crate) fn api(&self) -> Result<Api> {
        let s = self.session()?;
        let own = &s.account.server_url;
        let base = self
            .base
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|b| &b.public == own || &b.url == own)
            .map(|b| b.url.clone())
            .unwrap_or_else(|| own.clone());
        Ok(Api::new(&base).with_token(s.token.clone()))
    }

    fn set_base(&self, b: Option<Base>) {
        *self.base.write().unwrap_or_else(|e| e.into_inner()) = b;
    }

    /// Whether the user allowed the plain-http home URL of `public_url`.
    pub fn home_url_allowed(&self, public_url: &str) -> bool {
        self.store()
            .get_plain(&format!("{KV_HOME_ALLOWED}{public_url}"))
            .ok()
            .flatten()
            .is_some_and(|v| v == "1")
    }

    /// Allow (or stop allowing) a plain-http home URL for `public_url`.
    pub fn set_home_url_allowed(self: &Arc<Self>, public_url: &str, allowed: bool) -> Result<()> {
        let k = format!("{KV_HOME_ALLOWED}{public_url}");
        {
            let st = self.store();
            if allowed {
                st.put_plain(&k, "1")?;
            } else {
                st.delete(&k)?;
            }
        }
        if !allowed && self.active_public_url().as_deref() == Some(public_url) {
            // Stop using the home URL right away, before the re-probe finishes.
            self.set_base(Some(Base::public(public_url)));
        }
        let me = self.clone();
        crate::runtime().spawn(async move {
            me.refresh_network().await;
        });
        Ok(())
    }

    /// The public URL of the active server.
    pub(crate) fn active_public_url(&self) -> Option<String> {
        self.store().get_plain(KV_ACTIVE_SERVER).ok().flatten()
    }

    // ---- servers ----

    pub fn saved_servers(&self) -> Vec<ServerInfo> {
        self.store().servers().unwrap_or_default()
    }

    pub async fn add_server(&self, address: &str) -> Result<ServerInfo> {
        let info = server::fetch_server_info(address).await?;
        let make_active = {
            let st = self.store();
            st.save_server(&info)?;
            st.get_plain(KV_ACTIVE_SERVER)?.is_none()
        };
        if make_active {
            self.store().put_plain(KV_ACTIVE_SERVER, &info.public_url)?;
            self.set_base(Some(Base::public(&info.public_url)));
            self.refresh_network().await;
        }
        Ok(info)
    }

    pub fn remove_server(&self, public_url: &str) -> Result<()> {
        let st = self.store();
        st.remove_server(public_url)?;
        if st.get_plain(KV_ACTIVE_SERVER)?.as_deref() == Some(public_url) {
            st.delete(KV_ACTIVE_SERVER)?;
            st.delete(&format!("{KV_HOME_ALLOWED}{public_url}"))?;
            self.set_base(None);
        } else {
            st.delete(&format!("{KV_HOME_ALLOWED}{public_url}"))?;
        }
        Ok(())
    }

    pub fn select_server(self: &Arc<Self>, public_url: &str) -> Result<()> {
        {
            let st = self.store();
            if !st.servers()?.iter().any(|s| s.public_url == public_url) {
                return Err(CoreError::storage(format!("unknown server {public_url}")));
            }
            st.put_plain(KV_ACTIVE_SERVER, public_url)?;
        }
        self.set_base(Some(Base::public(public_url)));
        let me = self.clone();
        crate::runtime().spawn(async move {
            me.refresh_network().await;
        });
        Ok(())
    }

    pub fn active_base_url(&self) -> Option<String> {
        self.base
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|b| b.url.clone())
    }

    pub async fn refresh_network(&self) -> Option<String> {
        let active = self.active_public_url()?;
        let info = self
            .store()
            .servers()
            .ok()?
            .into_iter()
            .find(|s| s.public_url == active)?;
        let allow_http = self.home_url_allowed(&active);
        let mut base = server::choose_base(&info, allow_http).await;
        // The approval may have been withdrawn while we probed (a probe started
        // earlier must not bring the plain-http home URL back).
        if allow_http
            && base != server::trim_url(&info.public_url)
            && base.to_ascii_lowercase().starts_with("http://")
            && !self.home_url_allowed(&active)
        {
            base = server::trim_url(&info.public_url);
        }
        // The user may have switched servers while we probed.
        if self.active_public_url().as_deref() == Some(active.as_str()) {
            self.set_base(Some(Base {
                public: active,
                url: base.clone(),
            }));
            Some(base)
        } else {
            self.active_base_url()
        }
    }

    /// Persist a new session (wiping another account's data first).
    pub(crate) fn set_session(&self, s: Session) -> Result<()> {
        {
            let mut st = self.store();
            let old: Option<Session> = st.get_sealed(KV_SESSION)?;
            if old.is_some_and(|o| {
                o.account.user_id != s.account.user_id
                    || o.account.server_url != s.account.server_url
            }) {
                st.wipe_account()?;
                drop(st);
                self.wipe_cache();
                st = self.store();
            }
            st.put_sealed(KV_SESSION, &s)?;
        }
        *self.session.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(s));
        self.rebuild_index()
    }

    /// Remove the account's cached objects.
    pub(crate) fn wipe_cache(&self) {
        for d in ["thumbs", "files", crate::ml::faces::CROP_CACHE_DIR] {
            let _ = std::fs::remove_dir_all(self.cache_dir.join(d));
        }
    }

    pub fn current_account(&self) -> Option<Account> {
        self.session().ok().map(|s| s.account.clone())
    }
}
