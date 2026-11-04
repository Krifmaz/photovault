//! Shared application state handed to every handler.

use crate::config::Config;
use crate::db::Db;
use crate::metrics::Metrics;
use crate::storage::ObjectStore;
use std::sync::Arc;

/// Decoded server secrets.
#[derive(Clone)]
pub struct Keys {
    /// HMAC key for JWTs and signed storage URLs.
    pub jwt: Vec<u8>,
    /// secretbox key for emails at rest.
    pub encryption: Vec<u8>,
    /// generichash key for email lookup hashes.
    pub hashing: Vec<u8>,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys(..)")
    }
}

/// Cheaply clonable state (`Arc` inside).
#[derive(Clone, Debug)]
pub struct AppState(pub Arc<Inner>);

/// The actual state.
#[derive(Debug)]
pub struct Inner {
    /// Configuration.
    pub config: Config,
    /// Database.
    pub db: Db,
    /// Object store.
    pub store: Arc<dyn ObjectStore>,
    /// The local store, when that backend is active (serves `/storage/*`).
    pub local_store: Option<crate::storage::local::LocalStore>,
    /// Secrets.
    pub keys: Keys,
    /// Prometheus metrics.
    pub metrics: Metrics,
    /// Cached free space of the storage backend.
    pub capacity: crate::capacity::Cache,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// Build state from config: connect, migrate, open the object store, decode secrets
/// (random ones are generated when empty, which is fine for tests and first runs).
pub async fn build_state(config: Config) -> anyhow::Result<AppState> {
    crate::crypto::init();
    let db = Db::connect(&config.database_url).await?;
    db.migrate(&config.database_url).await?;
    let key = |s: &str, n: usize| -> anyhow::Result<Vec<u8>> {
        if s.is_empty() {
            Ok(crate::crypto::random_bytes(n))
        } else {
            Ok(crate::crypto::unb64(s)?)
        }
    };
    let keys = Keys {
        jwt: key(&config.secrets.jwt, 32)?,
        encryption: key(&config.secrets.encryption, 32)?,
        hashing: key(&config.secrets.hashing, 32)?,
    };
    let store: Arc<dyn crate::storage::ObjectStore> =
        Arc::from(crate::storage::open(&config, &keys.jwt)?);
    let local_store = match &config.storage {
        crate::config::StorageConfig::Local { path, public_url } => {
            Some(crate::storage::local::LocalStore::new(
                path,
                public_url
                    .clone()
                    .unwrap_or_else(|| config.public_url.clone()),
                keys.jwt.clone(),
            )?)
        }
        _ => None,
    };
    Ok(AppState(Arc::new(Inner {
        config,
        db,
        store,
        local_store,
        keys,
        metrics: Default::default(),
        capacity: Default::default(),
    })))
}
