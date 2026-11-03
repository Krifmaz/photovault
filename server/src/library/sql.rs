//! Small portability helpers for the library's SQL (Postgres and SQLite through `AnyPool`).
//!
//! * [`ids_list`] builds `$n, $n+1, …` placeholder lists for `IN (…)` (the Any driver has no arrays).
//! * Row getters ([`get_bool`] etc.) accept SQLite's integer booleans.
//! * [`begin`] opens a write transaction. On SQLite it serialises writers inside the
//!   process and takes the write lock up front, so a transaction never fails half-way
//!   with `SQLITE_BUSY` when it upgrades from read to write.

use crate::db::{Db, Engine};
use crate::error::ApiError;
use sqlx::any::{AnyRow, AnyTypeInfoKind};
use sqlx::{Any, Row, Transaction, TypeInfo, ValueRef};

/// Placeholder list `"$start, $start+1, …"` for `n` values (`n` must be > 0).
pub fn ids_list(start: usize, n: usize) -> String {
    (start..start + n)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `" FOR UPDATE"` on Postgres, empty on SQLite (which locks the whole database instead).
pub fn for_update(db: &Db) -> &'static str {
    match db.engine {
        Engine::Postgres => " FOR UPDATE",
        Engine::Sqlite => "",
    }
}

/// Map a sqlx error to museum's behaviour: `sql.ErrNoRows` → 404 `{}`, anything else → 500.
pub fn db_err(e: sqlx::Error) -> ApiError {
    match e {
        sqlx::Error::RowNotFound => ApiError::status(axum::http::StatusCode::NOT_FOUND),
        e => ApiError::internal(e),
    }
}

/// Read a boolean column (Postgres `BOOLEAN` or SQLite integer).
pub fn get_bool(row: &AnyRow, col: &str) -> Result<bool, sqlx::Error> {
    match row.try_get::<bool, _>(col) {
        Ok(b) => Ok(b),
        Err(_) => row.try_get::<i64, _>(col).map(|v| v != 0),
    }
}

/// Read a nullable boolean column.
pub fn get_opt_bool(row: &AnyRow, col: &str) -> Result<Option<bool>, sqlx::Error> {
    if is_null(row, col)? {
        return Ok(None);
    }
    get_bool(row, col).map(Some)
}

/// Read a `BIGINT` column. Integer-like values of other widths (counts, sums) are accepted.
pub fn get_i64(row: &AnyRow, col: &str) -> Result<i64, sqlx::Error> {
    match row.try_get::<i64, _>(col) {
        Ok(v) => Ok(v),
        Err(e) => match row.try_get::<i32, _>(col) {
            Ok(v) => Ok(v as i64),
            Err(_) => match row.try_get::<f64, _>(col) {
                Ok(v) => Ok(v as i64),
                Err(_) => Err(e),
            },
        },
    }
}

/// Read a nullable `BIGINT` column.
pub fn get_opt_i64(row: &AnyRow, col: &str) -> Result<Option<i64>, sqlx::Error> {
    if is_null(row, col)? {
        return Ok(None);
    }
    get_i64(row, col).map(Some)
}

/// Read a `TEXT` column.
pub fn get_str(row: &AnyRow, col: &str) -> Result<String, sqlx::Error> {
    row.try_get::<String, _>(col)
}

/// Read a nullable `TEXT` column.
pub fn get_opt_str(row: &AnyRow, col: &str) -> Result<Option<String>, sqlx::Error> {
    row.try_get::<Option<String>, _>(col)
}

fn is_null(row: &AnyRow, col: &str) -> Result<bool, sqlx::Error> {
    let v = row.try_get_raw(col)?;
    Ok(v.is_null() || v.type_info().kind() == AnyTypeInfoKind::Null || v.type_info().is_null())
}

/// A write transaction. Commit with [`WriteTx::commit`]; dropping it rolls back.
pub struct WriteTx {
    /// The underlying transaction.
    pub tx: Transaction<'static, Any>,
    _guard: Option<tokio::sync::MutexGuard<'static, ()>>,
}

static SQLITE_WRITER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl WriteTx {
    /// Commit.
    pub async fn commit(self) -> Result<(), sqlx::Error> {
        self.tx.commit().await
    }

    /// The transaction's connection, for passing to helpers that take `&mut AnyConnection`.
    pub fn conn(&mut self) -> &mut sqlx::AnyConnection {
        &mut self.tx
    }
}

impl std::ops::Deref for WriteTx {
    type Target = Transaction<'static, Any>;
    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl std::ops::DerefMut for WriteTx {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tx
    }
}

/// Begin a write transaction (see module docs for the SQLite locking strategy).
pub async fn begin(db: &Db) -> Result<WriteTx, sqlx::Error> {
    let guard = match db.engine {
        Engine::Sqlite => Some(SQLITE_WRITER.lock().await),
        Engine::Postgres => None,
    };
    let mut tx = db.pool.begin().await?;
    if db.engine == Engine::Sqlite {
        // A write statement takes SQLite's RESERVED lock now (waiting via busy_timeout),
        // instead of failing on a lock upgrade later in the transaction.
        sqlx::query("UPDATE queue SET item = item WHERE 1 = 0")
            .execute(&mut *tx)
            .await?;
    }
    Ok(WriteTx { tx, _guard: guard })
}

/// `GREATEST(a, b)` spelled for the active engine.
pub fn greatest(db: &Db, a: &str, b: &str) -> String {
    match db.engine {
        Engine::Postgres => format!("GREATEST({a}, {b})"),
        Engine::Sqlite => format!("MAX({a}, {b})"),
    }
}
