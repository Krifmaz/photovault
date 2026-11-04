//! Startup checks, run before the server binds its port.
//!
//! Each failure is one actionable line, e.g.
//! `cannot write /data: owned by uid 0, running as uid 65532; fix: chown -R 65532:65532 <host folder mounted at /data>`.
//!
//! A failed check prints that line once, waits 30 s and exits with status 78
//! (`EX_CONFIG`), so `restart: unless-stopped` retries slowly instead of spinning.

use crate::config::{Config, StorageConfig};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Exit status for configuration errors (sysexits `EX_CONFIG`).
pub const EX_CONFIG: i32 = 78;

/// A failed check: the one line to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure(pub String);

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

fn gid() -> u32 {
    rustix::process::getegid().as_raw()
}

/// The three secrets: set, base64, and the right length.
pub fn check_keys(config: &Config) -> Result<(), Failure> {
    let gen_hint = "fix: run `photovault gen-keys` (or `head -c 32 /dev/urandom | base64`) and set it; keep it forever, changing it loses access to existing accounts";
    let checks: [(&str, &str, std::ops::RangeInclusive<usize>); 3] = [
        ("PHOTOVAULT_JWT_SECRET", &config.secrets.jwt, 32..=64),
        (
            "PHOTOVAULT_ENCRYPTION_KEY",
            &config.secrets.encryption,
            32..=32,
        ),
        ("PHOTOVAULT_HASHING_KEY", &config.secrets.hashing, 16..=64),
    ];
    for (name, value, len) in checks {
        if value.trim().is_empty() {
            return Err(Failure(format!("{name} is not set; {gen_hint}")));
        }
        let bytes = crate::crypto::unb64(value.trim())
            .map_err(|_| Failure(format!("{name} is not valid base64; {gen_hint}")))?;
        if !len.contains(&bytes.len()) {
            let want = if len.start() == len.end() {
                format!("{}", len.start())
            } else {
                format!("{}-{}", len.start(), len.end())
            };
            return Err(Failure(format!(
                "{name} decodes to {} bytes, needs {want}; {gen_hint}",
                bytes.len()
            )));
        }
    }
    Ok(())
}

/// `dir` exists, is a directory and the running user can create and delete a file in it.
/// `what` names it in the message (`storage path`, `database folder`).
pub fn check_writable_dir(dir: &Path, what: &str) -> Result<(), Failure> {
    use std::os::unix::fs::MetadataExt;
    let d = dir.display();
    let meta = match std::fs::metadata(dir) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Failure(format!(
                "{what} {d} does not exist; fix: mount a host folder there (volumes: - <host folder>:{d}) or create it"
            )));
        }
        Err(e) => {
            return Err(Failure(format!(
                "cannot read {what} {d}: {e}; fix: check the volume mount and its permissions"
            )));
        }
    };
    if !meta.is_dir() {
        return Err(Failure(format!(
            "{what} {d} is not a directory; fix: point it at a folder"
        )));
    }
    let probe = dir.join(format!(
        ".photovault-write-test-{}",
        hex::encode(crate::crypto::random_bytes(4))
    ));
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => {
            let (u, g) = (uid(), gid());
            let hint = format!("fix: chown -R {u}:{g} <host folder mounted at {d}>");
            if e.kind() == std::io::ErrorKind::PermissionDenied && meta.uid() != u {
                Err(Failure(format!(
                    "cannot write {d}: owned by uid {}, running as uid {u}; {hint}",
                    meta.uid()
                )))
            } else if e.raw_os_error() == Some(30) {
                Err(Failure(format!(
                    "cannot write {d}: read-only file system; fix: mount the volume read-write"
                )))
            } else if e.raw_os_error() == Some(28) {
                Err(Failure(format!(
                    "cannot write {d}: no space left on device; fix: free space on the disk behind it"
                )))
            } else {
                Err(Failure(format!(
                    "cannot write {d}: {e} (owned by uid {}, mode {:o}, running as uid {u}); {hint}",
                    meta.uid(),
                    meta.mode() & 0o7777
                )))
            }
        }
    }
}

/// The folder an SQLite URL keeps its database in, if any.
pub fn sqlite_dir(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("sqlite:")?;
    let rest = rest.trim_start_matches("//");
    let path = rest.split('?').next().unwrap_or("");
    if path.is_empty() || path == ":memory:" {
        return None;
    }
    let p = Path::new(path);
    Some(match p.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    })
}

/// Hide the password in a database URL for messages.
pub fn redact_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) if u.password().is_some() => {
            let _ = u.set_password(Some("***"));
            u.to_string()
        }
        _ => url.to_string(),
    }
}

/// Connect to the database once (with a timeout).
pub async fn check_database(config: &Config) -> Result<(), Failure> {
    let url = &config.database_url;
    if let Some(dir) = sqlite_dir(url) {
        check_writable_dir(&dir, "database folder")?;
    }
    let shown = redact_url(url);
    let attempt = tokio::time::timeout(Duration::from_secs(10), async {
        let db = crate::db::Db::connect(url).await?;
        sqlx::query("SELECT 1").execute(&db.pool).await?;
        db.pool.close().await;
        anyhow::Ok(())
    })
    .await;
    match attempt {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            // The root cause only: sqlx repeats it in the context chain.
            let msg = e.root_cause().to_string().replace('\n', " ");
            let hint = if msg.contains("password authentication failed") {
                "fix: the password in PHOTOVAULT_DATABASE_URL must match POSTGRES_PASSWORD (Postgres only reads POSTGRES_PASSWORD when its data folder is empty)"
            } else if msg.contains("role") && msg.contains("does not exist") {
                "fix: the user name in PHOTOVAULT_DATABASE_URL must match POSTGRES_USER"
            } else if msg.contains("does not exist") {
                "fix: create the database or correct its name in PHOTOVAULT_DATABASE_URL"
            } else if msg.contains("unable to open database file") {
                "fix: check the SQLite folder is mounted and writable"
            } else {
                "fix: check PHOTOVAULT_DATABASE_URL and that the database container is running on the same network"
            };
            Err(Failure(format!(
                "cannot reach database {shown}: {msg}; {hint}"
            )))
        }
        Err(_) => Err(Failure(format!(
            "cannot reach database {shown}: no answer within 10 s; fix: check the host and port in PHOTOVAULT_DATABASE_URL and that the database container is running"
        ))),
    }
}

/// Older images kept SQLite at `/db/photovault.sqlite`. Refuse to start an empty
/// database in `/config` while that one is still mounted.
pub fn check_old_layout(config: &Config) -> Result<(), Failure> {
    let old = Path::new("/db/photovault.sqlite");
    if config.database_url == config.default_database_url()
        && old.is_file()
        && !config.config_dir.join("photovault.db").exists()
    {
        return Err(Failure(format!(
            "found the database of an older install at {}; fix: mount the config folder at /config and move that file to /config/photovault.db, or set PHOTOVAULT_DATABASE_URL=sqlite://{}?mode=rwc",
            old.display(),
            old.display()
        )));
    }
    Ok(())
}

/// Every check that does not need the running server, in order. Fills missing
/// secrets from (or into) `<config dir>/keys.toml` first.
pub async fn run(config: &mut Config) -> Result<(), Failure> {
    let s = &config.secrets;
    let needs_keys = s.jwt.is_empty() || s.encryption.is_empty() || s.hashing.is_empty();
    if needs_keys
        || sqlite_dir(&config.database_url).as_deref() == Some(config.config_dir.as_path())
    {
        // Outside a container the default `./config` may not exist yet.
        if !config.config_dir.exists() {
            let _ = std::fs::create_dir_all(&config.config_dir);
        }
        check_writable_dir(&config.config_dir, "config folder")?;
    }
    match crate::keyfile::ensure(config)? {
        crate::keyfile::Outcome::Generated(p) => tracing::warn!(
            "Generated new keys in {}. Back up this file; it cannot be recreated.",
            p.display()
        ),
        crate::keyfile::Outcome::Loaded(_) | crate::keyfile::Outcome::FromConfig => {}
    }
    check_keys(config)?;
    check_old_layout(config)?;
    if let StorageConfig::Local { path, .. } = &config.storage {
        // Outside Docker a missing folder is created (as earlier versions did).
        if !path.exists() {
            let _ = std::fs::create_dir_all(path);
        }
        check_writable_dir(path, "storage path")?;
    }
    if let StorageConfig::S3(c) = &config.storage {
        if c.endpoint.is_empty() || c.bucket.is_empty() {
            return Err(Failure(
                "S3 storage needs PHOTOVAULT_S3_ENDPOINT and PHOTOVAULT_S3_BUCKET; fix: set both"
                    .into(),
            ));
        }
        let key = crate::crypto::unb64(&config.secrets.jwt).unwrap_or_default();
        let store = crate::storage::open(config, &key).map_err(|e| {
            Failure(format!(
                "cannot open S3 bucket {}: {e:#}; fix: check the PHOTOVAULT_S3_* settings",
                c.bucket
            ))
        })?;
        match tokio::time::timeout(
            Duration::from_secs(15),
            store.head("photovault-startup-check"),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                return Err(Failure(format!(
                    "cannot reach S3 bucket {} at {}: {e}; fix: check the endpoint, bucket name and access keys",
                    c.bucket, c.endpoint
                )));
            }
            Err(_) => {
                return Err(Failure(format!(
                    "cannot reach S3 endpoint {}: no answer within 15 s; fix: check the address and network",
                    c.endpoint
                )));
            }
        }
    }
    check_database(config).await
}

/// Report a failed check as one line on stderr, wait 30 s (so a restart policy does not
/// spin) and exit with [`EX_CONFIG`]. `SIGTERM` during the wait exits at once.
pub async fn fail(f: &Failure) -> ! {
    eprintln!("photovault: {f}");
    tokio::select! {
        () = tokio::time::sleep(Duration::from_secs(30)) => {}
        () = crate::shutdown_signal() => {}
    }
    std::process::exit(EX_CONFIG)
}

/// Run the checks for `serve`; on failure see [`fail`].
pub async fn run_or_exit(config: &mut Config) {
    if let Err(f) = run(config).await {
        fail(&f).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(n: usize) -> String {
        crate::crypto::b64(&vec![7u8; n])
    }

    #[test]
    fn keys() {
        let mut c = Config::default();
        let e = check_keys(&c).unwrap_err();
        assert!(
            e.0.starts_with("PHOTOVAULT_JWT_SECRET is not set; fix:"),
            "{e}"
        );
        c.secrets.jwt = b64(32);
        c.secrets.encryption = b64(16);
        c.secrets.hashing = b64(32);
        let e = check_keys(&c).unwrap_err();
        assert!(
            e.0.contains("PHOTOVAULT_ENCRYPTION_KEY decodes to 16 bytes, needs 32"),
            "{e}"
        );
        c.secrets.encryption = "not base64!".into();
        assert!(check_keys(&c).unwrap_err().0.contains("not valid base64"));
        c.secrets.encryption = b64(32);
        assert!(check_keys(&c).is_ok());
    }

    #[test]
    fn writable_dir() {
        let t = tempfile::tempdir().unwrap();
        assert!(check_writable_dir(t.path(), "storage path").is_ok());
        let missing = t.path().join("nope");
        let e = check_writable_dir(&missing, "storage path").unwrap_err();
        assert!(e.0.contains("does not exist; fix:"), "{e}");
        let file = t.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        assert!(
            check_writable_dir(&file, "storage path")
                .unwrap_err()
                .0
                .contains("not a directory")
        );
    }

    #[test]
    fn unwritable_dir_names_owner_and_fix() {
        use std::os::unix::fs::PermissionsExt;
        if uid() == 0 {
            return; // root can write anywhere
        }
        let t = tempfile::tempdir().unwrap();
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let e = check_writable_dir(t.path(), "storage path").unwrap_err();
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            e.0.starts_with(&format!("cannot write {}", t.path().display())),
            "{e}"
        );
        assert!(e.0.contains("fix: chown -R"), "{e}");
    }

    #[test]
    fn sqlite_paths() {
        assert_eq!(
            sqlite_dir("sqlite:///db/photovault.sqlite?mode=rwc"),
            Some(PathBuf::from("/db"))
        );
        assert_eq!(
            sqlite_dir("sqlite://photovault.sqlite?mode=rwc"),
            Some(PathBuf::from("."))
        );
        assert_eq!(sqlite_dir("postgres://a:b@db/x"), None);
        assert_eq!(
            redact_url("postgres://photovault:secret@db:5432/photovault"),
            "postgres://photovault:***@db:5432/photovault"
        );
    }

    #[tokio::test]
    async fn unreachable_postgres_is_one_line() {
        let c = Config {
            database_url: "postgres://u:pw@127.0.0.1:1/x".into(),
            ..Config::default()
        };
        let e = check_database(&c).await.unwrap_err();
        assert!(
            e.0.starts_with("cannot reach database postgres://u:***@127.0.0.1:1/x"),
            "{e}"
        );
        assert!(!e.0.contains('\n'));
    }
}
