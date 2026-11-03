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

/// Everything behind [`crate::Core`].
pub struct Inner {
    pub(crate) cache_dir: PathBuf,
    store: Mutex<Store>,
    pub(crate) session: RwLock<Option<Arc<Session>>>,
    pub(crate) pending: Mutex<Option<crate::auth::Pending>>,
    base: RwLock<Option<String>>,
    pub(crate) index: RwLock<Arc<Index>>,
    pub(crate) sync_lock: tokio::sync::Mutex<()>,
    pub(crate) upload_lock: tokio::sync::Mutex<()>,
    pub(crate) live_upload: Mutex<LiveUpload>,
}

impl Inner {
    /// Open the store and restore the session and index.
    pub fn open(data_dir: PathBuf, cache_dir: PathBuf) -> Result<Arc<Self>> {
        crate::crypto::init();
        std::fs::create_dir_all(&cache_dir)?;
        let store = Store::open(&data_dir)?;
        let session: Option<Session> = store.get_sealed(KV_SESSION)?;
        let active = store.get_plain(KV_ACTIVE_SERVER)?;
        let inner = Arc::new(Self {
            cache_dir,
            store: Mutex::new(store),
            session: RwLock::new(session.map(Arc::new)),
            pending: Mutex::new(None),
            base: RwLock::new(active.clone()),
            index: RwLock::new(Arc::new(Index::default())),
            sync_lock: tokio::sync::Mutex::new(()),
            upload_lock: tokio::sync::Mutex::new(()),
            live_upload: Mutex::new(LiveUpload::default()),
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

    /// A client for the active base URL with the session token.
    pub(crate) fn api(&self) -> Result<Api> {
        let s = self.session()?;
        let base = self
            .active_base_url()
            .unwrap_or_else(|| s.account.server_url.clone());
        Ok(Api::new(&base).with_token(s.token.clone()))
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
            *self.base.write().unwrap_or_else(|e| e.into_inner()) = Some(info.public_url.clone());
            let base = server::choose_base(&info).await;
            *self.base.write().unwrap_or_else(|e| e.into_inner()) = Some(base);
        }
        Ok(info)
    }

    pub fn remove_server(&self, public_url: &str) -> Result<()> {
        let st = self.store();
        st.remove_server(public_url)?;
        if st.get_plain(KV_ACTIVE_SERVER)?.as_deref() == Some(public_url) {
            st.delete(KV_ACTIVE_SERVER)?;
            *self.base.write().unwrap_or_else(|e| e.into_inner()) = None;
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
        *self.base.write().unwrap_or_else(|e| e.into_inner()) = Some(public_url.to_string());
        let me = self.clone();
        crate::runtime().spawn(async move {
            me.refresh_network().await;
        });
        Ok(())
    }

    pub fn active_base_url(&self) -> Option<String> {
        self.base.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub async fn refresh_network(&self) -> Option<String> {
        let active = self.active_public_url()?;
        let info = self
            .store()
            .servers()
            .ok()?
            .into_iter()
            .find(|s| s.public_url == active)?;
        let base = server::choose_base(&info).await;
        // The user may have switched servers while we probed.
        if self.active_public_url().as_deref() == Some(active.as_str()) {
            *self.base.write().unwrap_or_else(|e| e.into_inner()) = Some(base.clone());
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
        for d in ["thumbs", "files"] {
            let _ = std::fs::remove_dir_all(self.cache_dir.join(d));
        }
    }

    pub fn current_account(&self) -> Option<Account> {
        self.session().ok().map(|s| s.account.clone())
    }
}
