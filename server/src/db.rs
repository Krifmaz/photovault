//! Database access: one [`sqlx::AnyPool`] over Postgres or SQLite.
//!
//! # Portability rules for SQL in this crate
//! * Use `$1, $2, …` placeholders, and make each placeholder's **first**
//!   appearance follow ascending order (SQLite numbers named params by first use).
//! * Column types: `BIGINT`, `TEXT`, `BOOLEAN`. Store times as microseconds
//!   since the epoch in `BIGINT` (museum's convention), JSON as `TEXT`.
//! * Use `INSERT … RETURNING` for generated ids; `ON CONFLICT … DO UPDATE` is fine on both.
//! * Migrations live in `migrations/postgres` and `migrations/sqlite` with identical file names.

use sqlx::AnyPool;
use sqlx::any::{AnyPoolOptions, install_default_drivers};

/// Which engine the pool talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// PostgreSQL (multi-user).
    Postgres,
    /// SQLite (single-user / small installs).
    Sqlite,
}

/// Database handle shared through [`crate::state::AppState`].
#[derive(Clone, Debug)]
pub struct Db {
    /// The pool.
    pub pool: AnyPool,
    /// Engine behind the pool.
    pub engine: Engine,
}

static PG_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
static SQLITE_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

impl Db {
    /// Connect to `url` (`postgres://…` or `sqlite://…`).
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        install_default_drivers();
        let engine = if url.starts_with("postgres") {
            Engine::Postgres
        } else {
            Engine::Sqlite
        };
        let max = if engine == Engine::Sqlite { 4 } else { 16 };
        let pool = AnyPoolOptions::new()
            .max_connections(max)
            .after_connect(move |conn, _| {
                Box::pin(async move {
                    if engine == Engine::Sqlite {
                        use sqlx::Executor;
                        conn.execute("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 10000; PRAGMA synchronous = NORMAL;").await?;
                    }
                    Ok(())
                })
            })
            .connect(url)
            .await?;
        Ok(Self { pool, engine })
    }

    /// Run pending migrations for the active engine.
    pub async fn migrate(&self, url: &str) -> anyhow::Result<()> {
        match self.engine {
            Engine::Postgres => {
                let pool = sqlx::PgPool::connect(url).await?;
                PG_MIGRATOR.run(&pool).await?;
                pool.close().await;
            }
            Engine::Sqlite => {
                let pool = sqlx::SqlitePool::connect(url).await?;
                SQLITE_MIGRATOR.run(&pool).await?;
                pool.close().await;
            }
        }
        Ok(())
    }
}

/// Current time in microseconds since the Unix epoch (museum's timestamp unit).
pub fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}
