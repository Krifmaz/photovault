//! Helpers for integration tests: an in-process server on SQLite (or Postgres
//! via `PHOTOVAULT_TEST_DATABASE_URL`) with a temporary local object store.

use crate::config::{Config, StorageConfig};
use crate::state::AppState;

/// A running test server.
pub struct TestServer {
    /// Base URL, e.g. `http://127.0.0.1:12345`.
    pub url: String,
    /// State (direct DB access in tests).
    pub state: AppState,
    /// Keeps the temp dir alive.
    pub _dir: tempdir::TempDir,
}

/// Minimal temp dir so the library has no dev-only dependency.
pub mod tempdir {
    /// Directory removed on drop.
    pub struct TempDir(pub std::path::PathBuf);
    impl TempDir {
        /// Create a unique temp dir.
        pub fn new() -> std::io::Result<Self> {
            let p = std::env::temp_dir().join(format!(
                "photovault-test-{}",
                hex::encode(crate::crypto::random_bytes(8))
            ));
            std::fs::create_dir_all(&p)?;
            Ok(Self(p))
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

impl TestServer {
    /// Start a server on a random port. `tweak` may adjust the config.
    pub async fn start_with(tweak: impl FnOnce(&mut Config)) -> anyhow::Result<Self> {
        let dir = tempdir::TempDir::new()?;
        let db_url = match std::env::var("PHOTOVAULT_TEST_DATABASE_URL") {
            // A fresh database per test server, so parallel tests never share state.
            Ok(base) if base.starts_with("postgres") => {
                use sqlx::Executor;
                let name = format!("pv_test_{}", hex::encode(crate::crypto::random_bytes(6)));
                let admin = sqlx::PgPool::connect(&base).await?;
                admin
                    .execute(format!("CREATE DATABASE {name}").as_str())
                    .await?;
                admin.close().await;
                let mut u = url::Url::parse(&base)?;
                u.set_path(&format!("/{name}"));
                u.to_string()
            }
            _ => format!("sqlite://{}/db.sqlite?mode=rwc", dir.0.display()),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let mut config = Config {
            public_url: url.clone(),
            database_url: db_url.clone(),
            storage: StorageConfig::Local {
                path: dir.0.join("objects"),
                public_url: None,
            },
            log_format: "pretty".into(),
            allow_signups: true,
            config_dir: dir.0.join("config"),
            ..Config::default()
        };
        tweak(&mut config);
        let state = crate::state::build_state(config).await?;
        let app = crate::app(state.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            url,
            state,
            _dir: dir,
        })
    }

    /// Start with default config.
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_with(|_| {}).await
    }
}
