//! Admin CLI (`photovault admin …`): users, storage quotas, OTP codes, 2FA resets,
//! deletion and admin rights. Commands run against the configured database directly, so
//! they work before any account (or admin session) exists.
//!
//! Users are named by email or numeric id. Nothing here passes input to a shell.

use crate::auth::App;
use crate::db::now_micros;
use crate::state::AppState;
use crate::users;
use clap::Subcommand;

/// Admin subcommands.
#[derive(Subcommand, Debug)]
pub enum AdminCmd {
    /// List users: id, email, storage quota (bytes), usage (bytes), admin flag.
    ListUsers,
    /// Set a user's storage quota, e.g. `set-storage alice@example.org 50GB`.
    SetStorage {
        /// Email or user id.
        user: String,
        /// Bytes, or a number with a KB/MB/GB/TB suffix (binary units).
        size: String,
    },
    /// Show unexpired one-time codes sent to an email (useful without SMTP).
    PendingOtt {
        /// Email address.
        email: String,
    },
    /// Add a one-time code an email can log in with (museum `admin/user/add-ott`).
    AddOtt {
        /// Email address.
        email: String,
        /// Code (at least 6 characters).
        code: String,
        /// App: photos, auth or locker.
        #[arg(long, default_value = "photos")]
        app: String,
        /// Minutes until the code expires.
        #[arg(long, default_value_t = 60)]
        minutes: i64,
    },
    /// Disable TOTP two-factor authentication (and optionally passkeys) for a user.
    Disable2fa {
        /// Email or user id.
        user: String,
        /// Also remove all passkeys.
        #[arg(long)]
        passkeys: bool,
    },
    /// Delete a user's account (data is removed after the recovery window).
    DeleteUser {
        /// Email or user id.
        user: String,
        /// Confirm the deletion.
        #[arg(long)]
        yes: bool,
    },
    /// Grant (or with `--revoke`, remove) access to the `/admin` API.
    MarkAdmin {
        /// Email or user id.
        user: String,
        /// Remove the admin flag instead.
        #[arg(long)]
        revoke: bool,
    },
}

/// Parse `123`, `1.5GB`, `500 MB`, `2TB` (binary units) into bytes.
pub fn parse_size(s: &str) -> anyhow::Result<i64> {
    let t = s.trim().to_ascii_uppercase();
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid size: {s}"))?;
    let mult: f64 = match unit.trim() {
        "" | "B" => 1.0,
        "K" | "KB" | "KIB" => 1024.0,
        "M" | "MB" | "MIB" => 1024f64.powi(2),
        "G" | "GB" | "GIB" => 1024f64.powi(3),
        "T" | "TB" | "TIB" => 1024f64.powi(4),
        other => anyhow::bail!("unknown size unit: {other}"),
    };
    let bytes = (n * mult).round();
    anyhow::ensure!(
        bytes >= 0.0 && bytes < i64::MAX as f64,
        "size out of range: {s}"
    );
    Ok(bytes as i64)
}

async fn resolve_user(state: &AppState, user: &str) -> anyhow::Result<i64> {
    if let Ok(id) = user.parse::<i64>() {
        let row: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM users WHERE user_id = $1")
            .bind(id)
            .fetch_optional(&state.db.pool)
            .await?;
        return row
            .map(|r| r.0)
            .ok_or_else(|| anyhow::anyhow!("no user with id {id}"));
    }
    users::user_id_for_email(state, user)
        .await
        .map_err(|e| anyhow::anyhow!("lookup failed: {}", e.message))?
        .ok_or_else(|| anyhow::anyhow!("no user with email {user}"))
}

fn api(e: crate::error::ApiError) -> anyhow::Error {
    anyhow::anyhow!("{} {}", e.status, e.code.unwrap_or("")).context(e.message)
}

/// Run an admin command.
pub async fn run(state: &AppState, cmd: AdminCmd) -> anyhow::Result<()> {
    match cmd {
        AdminCmd::ListUsers => {
            type Row = (i64, String, String, i64, i64, i64);
            let rows: Vec<Row> = sqlx::query_as(
                "SELECT user_id, encrypted_email, email_decryption_nonce, storage_quota, \
                 CASE WHEN is_deleted THEN 1 ELSE 0 END, CASE WHEN is_admin THEN 1 ELSE 0 END \
                 FROM users ORDER BY user_id",
            )
            .fetch_all(&state.db.pool)
            .await?;
            println!("id\temail\tquota\tusage\tadmin");
            for (id, ct, nonce, quota, deleted, admin) in rows {
                let email = if deleted != 0 || ct.is_empty() {
                    "(deleted)".to_string()
                } else {
                    users::decrypt_email(state, &ct, &nonce).unwrap_or_else(|_| "?".into())
                };
                let usage = users::usage(&state.db, id).await.unwrap_or(0);
                let admin = admin != 0 || users::extract::is_admin(state, id).await;
                println!("{id}\t{email}\t{quota}\t{usage}\t{admin}");
            }
        }
        AdminCmd::SetStorage { user, size } => {
            let id = resolve_user(state, &user).await?;
            let bytes = parse_size(&size)?;
            sqlx::query("UPDATE users SET storage_quota = $1 WHERE user_id = $2")
                .bind(bytes)
                .bind(id)
                .execute(&state.db.pool)
                .await?;
            println!("user {id}: storage quota set to {bytes} bytes");
        }
        AdminCmd::PendingOtt { email } => {
            let otts = users::login::pending_otts(state, &email)
                .await
                .map_err(api)?;
            if otts.is_empty() {
                println!("no pending codes for {email}");
            }
            let now = now_micros();
            for (ott, app, exp) in otts {
                println!(
                    "{ott}\t{app}\texpires in {}s",
                    (exp - now).max(0) / 1_000_000
                );
            }
        }
        AdminCmd::AddOtt {
            email,
            code,
            app,
            minutes,
        } => {
            anyhow::ensure!(
                matches!(app.as_str(), "photos" | "auth" | "locker"),
                "invalid app: {app}"
            );
            anyhow::ensure!(code.len() >= 6, "code must be at least 6 characters");
            anyhow::ensure!(minutes > 0, "minutes must be positive");
            let expiry = now_micros() + minutes * 60 * 1_000_000;
            users::login::add_ott(state, &email, App::parse(&app), &code, expiry)
                .await
                .map_err(api)?;
            println!("code added for {email} ({app}), valid for {minutes} minutes");
        }
        AdminCmd::Disable2fa { user, passkeys } => {
            let id = resolve_user(state, &user).await?;
            crate::routes::admin::disable_totp(state, id)
                .await
                .map_err(api)?;
            if passkeys {
                users::passkeys::delete_all(state, id).await.map_err(api)?;
            }
            println!(
                "user {id}: two-factor authentication disabled{}",
                if passkeys {
                    " and passkeys removed"
                } else {
                    ""
                }
            );
        }
        AdminCmd::DeleteUser { user, yes } => {
            anyhow::ensure!(yes, "refusing to delete without --yes");
            let id = resolve_user(state, &user).await?;
            users::lifecycle::delete_account(state, id, true)
                .await
                .map_err(api)?;
            println!("user {id}: account deleted");
        }
        AdminCmd::MarkAdmin { user, revoke } => {
            let id = resolve_user(state, &user).await?;
            sqlx::query("UPDATE users SET is_admin = $1 WHERE user_id = $2")
                .bind(!revoke)
                .bind(id)
                .execute(&state.db.pool)
                .await?;
            println!(
                "user {id}: admin {}",
                if revoke { "revoked" } else { "granted" }
            );
        }
    }
    Ok(())
}

/// Minimal HTTP/1.1 GET against `url` over plain TCP; fails unless the status is 2xx.
pub async fn healthcheck(url: &str) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let u = url::Url::parse(url)?;
    let host = u.host_str().ok_or_else(|| anyhow::anyhow!("no host"))?;
    let port = u.port_or_known_default().unwrap_or(80);
    let mut s = tokio::net::TcpStream::connect((host, port)).await?;
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n",
        u.path()
    );
    s.write_all(req.as_bytes()).await?;
    let mut buf = vec![0u8; 64];
    let n = s.read(&mut buf).await?;
    let line = String::from_utf8_lossy(&buf[..n]);
    let ok = line
        .split_whitespace()
        .nth(1)
        .is_some_and(|c| c.starts_with('2'));
    anyhow::ensure!(ok, "unhealthy: {}", line.lines().next().unwrap_or(""));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("123").ok(), Some(123));
        assert_eq!(parse_size("1.5GB").ok(), Some(1_610_612_736));
        assert_eq!(parse_size("500 mb").ok(), Some(500 * 1024 * 1024));
        assert_eq!(parse_size("2TB").ok(), Some(2 * 1024_i64.pow(4)));
        assert!(parse_size("12XB").is_err());
        assert!(parse_size("-1").is_err());
    }
}
