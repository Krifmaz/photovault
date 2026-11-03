//! Server settings stored in the database (`photovault_settings`), changed with the
//! admin CLI while the server runs. A stored value overrides the configuration.

use crate::db::Db;
use crate::state::AppState;

/// Key for the sign-up switch (`photovault admin signups on|off`).
pub const SIGNUPS: &str = "allow_signups";

/// Read a setting.
pub async fn get(db: &Db, key: &str) -> Option<String> {
    sqlx::query_as::<_, (String,)>("SELECT value FROM photovault_settings WHERE key = $1")
        .bind(key)
        .fetch_optional(&db.pool)
        .await
        .ok()
        .flatten()
        .map(|r| r.0)
}

/// Store a setting.
pub async fn set(db: &Db, key: &str, value: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO photovault_settings (key, value) VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE SET value = $2",
    )
    .bind(key)
    .bind(value)
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Whether new accounts may sign up: the admin CLI's switch, else `PHOTOVAULT_ALLOW_SIGNUPS`.
pub async fn signups_allowed(state: &AppState) -> bool {
    match get(&state.db, SIGNUPS).await.as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => state.config.allow_signups,
    }
}
