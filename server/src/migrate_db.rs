//! One-time copy of a PostgreSQL database into a new SQLite database
//! (`photovault admin migrate-to-sqlite`).
//!
//! The target gets the normal SQLite migrations, then every table is copied
//! row by row with foreign keys off, and row counts are compared. The source is
//! only read. Object files are untouched: they do not depend on the database
//! engine. The server keys must stay the same; [`migrate`] writes the current
//! ones to `keys.toml` next to the new database when that file does not exist.

use sqlx::{Column, Connection, Row, TypeInfo};

/// One table's result.
#[derive(Debug)]
pub struct Copied {
    /// Table name.
    pub table: String,
    /// Rows copied.
    pub rows: u64,
}

enum Val {
    Null,
    Int(i64),
    Bool(bool),
    Text(String),
    Bytes(Vec<u8>),
    Real(f64),
}

fn read(row: &sqlx::postgres::PgRow, i: usize) -> anyhow::Result<Val> {
    let col = &row.columns()[i];
    let ty = col.type_info().name().to_ascii_uppercase();
    macro_rules! get {
        ($t:ty, $wrap:expr) => {
            match row.try_get::<Option<$t>, _>(i)? {
                None => Val::Null,
                Some(v) => $wrap(v),
            }
        };
    }
    Ok(match ty.as_str() {
        "INT8" | "BIGINT" => get!(i64, Val::Int),
        "INT4" | "INT" | "INTEGER" => get!(i32, |v: i32| Val::Int(i64::from(v))),
        "INT2" | "SMALLINT" => get!(i16, |v: i16| Val::Int(i64::from(v))),
        "BOOL" | "BOOLEAN" => get!(bool, Val::Bool),
        "BYTEA" => get!(Vec<u8>, Val::Bytes),
        "FLOAT8" | "FLOAT4" | "DOUBLE PRECISION" | "REAL" => get!(f64, Val::Real),
        "JSON" | "JSONB" => get!(serde_json::Value, |v: serde_json::Value| Val::Text(
            v.to_string()
        )),
        _ => get!(String, Val::Text),
    })
}

/// Copy `from` (postgres://…) into the fresh SQLite database `to` (sqlite://…).
pub async fn copy(from: &str, to: &str) -> anyhow::Result<Vec<Copied>> {
    anyhow::ensure!(
        from.starts_with("postgres"),
        "--from must be a postgres:// URL"
    );
    anyhow::ensure!(to.starts_with("sqlite:"), "--to must be a sqlite:// URL");
    let target = crate::db::Db::connect(to).await?;
    target.migrate(to).await?;
    let (users,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&target.pool)
        .await?;
    anyhow::ensure!(
        users == 0,
        "the SQLite database {to} already has accounts; use a new file"
    );
    target.pool.close().await;

    let mut pg = sqlx::PgConnection::connect(from).await?;
    let mut lite = sqlx::SqliteConnection::connect(to).await?;
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut lite)
        .await?;

    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
         AND name <> '_sqlx_migrations' ORDER BY name",
    )
    .fetch_all(&mut lite)
    .await?;
    let mut out = vec![];
    for (table,) in tables {
        let in_pg: Option<(String,)> = sqlx::query_as(
            "SELECT table_name::text FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = $1",
        )
        .bind(&table)
        .fetch_optional(&mut pg)
        .await?;
        if in_pg.is_none() {
            tracing::warn!("table {table} is not in the Postgres database; left empty");
            continue;
        }
        let cols: Vec<(String,)> = sqlx::query_as(
            "SELECT column_name::text FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
        )
        .bind(&table)
        .fetch_all(&mut pg)
        .await?;
        let lite_cols: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as(&format!("PRAGMA table_info(\"{table}\")"))
                .fetch_all(&mut lite)
                .await?;
        let names: Vec<String> = cols
            .into_iter()
            .map(|c| c.0)
            .filter(|c| lite_cols.iter().any(|l| &l.1 == c))
            .collect();
        if names.is_empty() {
            continue;
        }
        let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
        let select = format!("SELECT {} FROM \"{table}\"", quoted.join(", "));
        let placeholders: Vec<String> = (1..=names.len()).map(|i| format!("${i}")).collect();
        let insert = format!(
            "INSERT INTO \"{table}\" ({}) VALUES ({})",
            quoted.join(", "),
            placeholders.join(", ")
        );
        let rows = sqlx::query(&select).fetch_all(&mut pg).await?;
        let mut tx = lite.begin().await?;
        for row in &rows {
            let mut q = sqlx::query(&insert);
            for (i, name) in names.iter().enumerate() {
                q = match read(row, i).map_err(|e| anyhow::anyhow!("{table}.{name}: {e}"))? {
                    Val::Null => q.bind(None::<String>),
                    Val::Int(v) => q.bind(v),
                    Val::Bool(v) => q.bind(v),
                    Val::Text(v) => q.bind(v),
                    Val::Bytes(v) => q.bind(v),
                    Val::Real(v) => q.bind(v),
                };
            }
            q.execute(&mut *tx)
                .await
                .map_err(|e| anyhow::anyhow!("insert into {table}: {e}"))?;
        }
        tx.commit().await?;
        let (n,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM \"{table}\""))
            .fetch_one(&mut lite)
            .await?;
        anyhow::ensure!(
            n as usize == rows.len(),
            "{table}: copied {} rows but SQLite has {n}",
            rows.len()
        );
        out.push(Copied {
            table,
            rows: rows.len() as u64,
        });
    }
    let bad: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&mut lite)
        .await
        .unwrap_or_default();
    anyhow::ensure!(
        bad.is_empty(),
        "foreign key check failed after the copy: {} problems (first: {})",
        bad.len(),
        bad[0].0
    );
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut lite)
        .await?;
    lite.close().await?;
    pg.close().await?;
    Ok(out)
}
