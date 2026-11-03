//! Runtime schema probes and portable locking, used where a query touches
//! tables owned by another workstream that may not exist in every build
//! (e.g. `families`, `passkeys`).

use crate::db::{Db, Engine};

/// Whether `table` exists in the current database/schema.
pub async fn has_table(db: &Db, table: &str) -> bool {
    let sql = match db.engine {
        Engine::Postgres => {
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = $1"
        }
        Engine::Sqlite => "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = $1",
    };
    sqlx::query_as::<_, (i64,)>(sql)
        .bind(table)
        .fetch_one(&db.pool)
        .await
        .map(|(n,)| n > 0)
        .unwrap_or(false)
}

/// Whether `table.column` exists.
pub async fn has_column(db: &Db, table: &str, column: &str) -> bool {
    let sql = match db.engine {
        Engine::Postgres => {
            "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2"
        }
        Engine::Sqlite => "SELECT COUNT(*) FROM pragma_table_info($1) WHERE name = $2",
    };
    sqlx::query_as::<_, (i64,)>(sql)
        .bind(table)
        .bind(column)
        .fetch_one(&db.pool)
        .await
        .map(|(n,)| n > 0)
        .unwrap_or(false)
}

/// ` FOR UPDATE` on Postgres, nothing on SQLite (whose writers are serialised by
/// [`lock_user`]'s early write).
pub fn for_update(engine: Engine) -> &'static str {
    match engine {
        Engine::Postgres => " FOR UPDATE",
        Engine::Sqlite => "",
    }
}

/// Lock the `users` row inside a transaction (museum's `FOR NO KEY UPDATE`).
/// On SQLite a no-op write takes the database write lock up front, so later
/// reads-then-writes in the same transaction cannot fail with `SQLITE_BUSY`.
/// Returns false when the user does not exist.
pub async fn lock_user(
    conn: &mut sqlx::AnyConnection,
    engine: Engine,
    user_id: i64,
) -> Result<bool, sqlx::Error> {
    match engine {
        Engine::Postgres => {
            let row: Option<(i64,)> =
                sqlx::query_as("SELECT user_id FROM users WHERE user_id = $1 FOR NO KEY UPDATE")
                    .bind(user_id)
                    .fetch_optional(&mut *conn)
                    .await?;
            Ok(row.is_some())
        }
        Engine::Sqlite => {
            let r = sqlx::query("UPDATE users SET user_id = user_id WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *conn)
                .await?;
            Ok(r.rows_affected() > 0)
        }
    }
}

/// Start a transaction that writes. On SQLite this is `BEGIN IMMEDIATE` so the
/// write lock is taken before any read (no read→write upgrade deadlocks).
pub async fn begin_write(db: &Db) -> Result<sqlx::Transaction<'static, sqlx::Any>, sqlx::Error> {
    match db.engine {
        Engine::Postgres => db.pool.begin().await,
        Engine::Sqlite => db.pool.begin_with("BEGIN IMMEDIATE").await,
    }
}
