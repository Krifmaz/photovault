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
    /// List users: id, email, storage quota, usage (bytes), admin flag.
    ListUsers,
    /// Create an account. The person then signs in with this email in the app and
    /// chooses their own password there (the server never sees it, so the end-to-end
    /// encryption keys stay theirs). Works with signups closed.
    CreateUser {
        /// Email address.
        email: String,
        /// Storage quota, e.g. `500GB`, or `none` (default: PHOTOVAULT_DEFAULT_QUOTA).
        #[arg(long)]
        quota: Option<String>,
        /// Also grant access to the `/admin` API.
        #[arg(long)]
        admin: bool,
    },
    /// Help someone who forgot their password. Photos are encrypted with a key that only
    /// the password or the recovery key opens, so the server cannot set a new password:
    /// this prints a sign-in code and the steps for "Forgot password" + recovery key.
    /// `--erase --yes` instead deletes the account and creates an empty one with the same
    /// email (all of its photos are lost).
    ResetPassword {
        /// Email or user id.
        user: String,
        /// Delete everything and start the account over.
        #[arg(long)]
        erase: bool,
        /// Confirm `--erase`.
        #[arg(long)]
        yes: bool,
    },
    /// Set a user's storage quota, e.g. `set-quota alice@example.org 50GB`; `none` removes it
    /// (the free space on the storage backend is then the limit).
    #[command(name = "quota", aliases = ["set-quota", "set-storage"])]
    SetQuota {
        /// Email or user id.
        user: String,
        /// Bytes, a number with a KB/MB/GB/TB suffix (binary units), or `none`.
        size: String,
    },
    /// Open or close sign-up for new accounts (`on`/`off`), or show the state with no value.
    /// Stored in the database; overrides PHOTOVAULT_ALLOW_SIGNUPS; takes effect at once.
    Signups {
        /// `on` or `off`.
        state: Option<String>,
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
            let show = |q: i64| {
                if q > 0 {
                    q.to_string()
                } else {
                    "none".to_string()
                }
            };
            for (id, ct, nonce, quota, deleted, admin) in rows {
                let email = if deleted != 0 || ct.is_empty() {
                    "(deleted)".to_string()
                } else {
                    users::decrypt_email(state, &ct, &nonce).unwrap_or_else(|_| "?".into())
                };
                let usage = users::usage(&state.db, id).await.unwrap_or(0);
                let admin = admin != 0 || users::extract::is_admin(state, id).await;
                println!("{id}\t{email}\t{}\t{usage}\t{admin}", show(quota));
            }
        }
        AdminCmd::CreateUser {
            email,
            quota,
            admin,
        } => {
            let email = users::normalize_email(&email);
            anyhow::ensure!(email.contains('@'), "not an email address: {email}");
            if users::user_id_for_email(state, &email)
                .await
                .map_err(api)?
                .is_some()
            {
                anyhow::bail!("{email} already has an account");
            }
            let id = users::store::create_user(state, &email, Some("admin-cli"))
                .await
                .map_err(api)?;
            if let Some(q) = quota {
                let bytes = crate::config::parse_quota(&q)?;
                sqlx::query("UPDATE users SET storage_quota = $1 WHERE user_id = $2")
                    .bind(bytes)
                    .bind(id)
                    .execute(&state.db.pool)
                    .await?;
            }
            if admin {
                sqlx::query("UPDATE users SET is_admin = $1 WHERE user_id = $2")
                    .bind(true)
                    .bind(id)
                    .execute(&state.db.pool)
                    .await?;
            }
            let code = issue_code(state, &email).await?;
            println!(
                "user {id}: created {email}{}",
                if admin { " (admin)" } else { "" }
            );
            println!(
                "next: in the app choose \"Create account\" with {email}, enter code {code} (valid 24 hours), then choose a password"
            );
        }
        AdminCmd::ResetPassword { user, erase, yes } => {
            let id = resolve_user(state, &user).await?;
            let email = users::email_for_user(state, id)
                .await
                .map_err(api)?
                .ok_or_else(|| anyhow::anyhow!("user {id} has no email"))?;
            if erase {
                anyhow::ensure!(
                    yes,
                    "refusing to erase {email} without --yes (all of its photos are deleted)"
                );
                users::lifecycle::delete_account(state, id, true)
                    .await
                    .map_err(api)?;
                let new_id = users::store::create_user(state, &email, Some("admin-cli"))
                    .await
                    .map_err(api)?;
                let code = issue_code(state, &email).await?;
                println!("user {id}: erased; new empty account {new_id} for {email}");
                println!(
                    "next: in the app sign in with {email}, enter code {code} (valid 24 hours), then choose a new password"
                );
            } else {
                let code = issue_code(state, &email).await?;
                println!(
                    "The server cannot reset {email}'s password: photos are end-to-end encrypted with a key only the password or the recovery key opens."
                );
                println!(
                    "next: in the app enter {email}, tap \"Forgot password\", enter code {code} if asked (valid 24 hours), then the 24-word recovery key, and choose a new password"
                );
                println!(
                    "no recovery key: `photovault admin reset-password {email} --erase --yes` starts the account over (its photos are lost)"
                );
            }
        }
        AdminCmd::Signups { state: value } => {
            match value.as_deref().map(str::to_ascii_lowercase).as_deref() {
                None => {}
                Some("on" | "true" | "open") => {
                    crate::settings::set(&state.db, crate::settings::SIGNUPS, "true").await?
                }
                Some("off" | "false" | "closed") => {
                    crate::settings::set(&state.db, crate::settings::SIGNUPS, "false").await?
                }
                Some(other) => anyhow::bail!("use `signups on` or `signups off` (got {other})"),
            }
            let open = crate::settings::signups_allowed(state).await;
            println!("signups are {}", if open { "on" } else { "off" });
        }
        AdminCmd::SetQuota { user, size } => {
            let id = resolve_user(state, &user).await?;
            let bytes = crate::config::parse_quota(&size)?;
            sqlx::query("UPDATE users SET storage_quota = $1 WHERE user_id = $2")
                .bind(bytes)
                .bind(id)
                .execute(&state.db.pool)
                .await?;
            if bytes > 0 {
                println!("user {id}: storage quota set to {bytes} bytes");
            } else {
                println!("user {id}: no quota (limited by free space on the storage backend)");
            }
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

/// A 6-digit sign-in code for `email`, valid for 24 hours (Photos app).
async fn issue_code(state: &AppState, email: &str) -> anyhow::Result<String> {
    let n: u32 = rand::random_range(0..1_000_000);
    let code = format!("{n:06}");
    let expiry = now_micros() + 24 * 3600 * 1_000_000;
    users::login::add_ott(state, email, App::Photos, &code, expiry)
        .await
        .map_err(api)?;
    Ok(code)
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
