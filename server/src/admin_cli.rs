//! Admin CLI (`photovault admin …`): users, storage quotas, and OTP codes.
//! Fleshed out by the users workstream.

use crate::state::AppState;
use clap::Subcommand;

/// Admin subcommands.
#[derive(Subcommand, Debug)]
pub enum AdminCmd {
    /// List users.
    ListUsers,
}

/// Run an admin command.
pub async fn run(state: &AppState, cmd: AdminCmd) -> anyhow::Result<()> {
    match cmd {
        AdminCmd::ListUsers => {
            let rows: Vec<(i64, i64)> =
                sqlx::query_as("SELECT user_id, storage_quota FROM users ORDER BY user_id")
                    .fetch_all(&state.db.pool)
                    .await?;
            for (id, quota) in rows {
                println!("{id}\t{quota}");
            }
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
