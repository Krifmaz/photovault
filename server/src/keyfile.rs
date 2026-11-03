//! Server secrets kept in `<config dir>/keys.toml` (mode 0600).
//!
//! On first start, any of the three secrets not given through the environment
//! (or the config file) is generated and written there, and a one-time reminder
//! to back the file up is logged. Values from the environment always win, so an
//! install that already sets `PHOTOVAULT_JWT_SECRET` etc. keeps using them.

use crate::config::{Config, Secrets};
use crate::preflight::Failure;
use std::path::{Path, PathBuf};

/// File name inside the config directory.
pub const FILE_NAME: &str = "keys.toml";

/// Write `secrets` to `<config dir>/keys.toml` unless it exists (used when moving an
/// install that kept its keys in environment variables). Returns whether it wrote.
pub fn save_if_missing(config: &Config) -> anyhow::Result<bool> {
    let p = path(config);
    if p.exists() {
        return Ok(false);
    }
    let s = &config.secrets;
    anyhow::ensure!(
        !s.jwt.is_empty() && !s.encryption.is_empty() && !s.hashing.is_empty(),
        "the server keys are not set, so there is nothing to save"
    );
    std::fs::create_dir_all(&config.config_dir)?;
    write_private(
        &p,
        &render(&KeyFile {
            jwt_secret: s.jwt.clone(),
            encryption_key: s.encryption.clone(),
            hashing_key: s.hashing.clone(),
        }),
    )?;
    Ok(true)
}

/// What [`ensure`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Every secret was already set (environment or config file).
    FromConfig,
    /// Secrets were read from the keys file.
    Loaded(PathBuf),
    /// Missing secrets were generated and written to the keys file.
    Generated(PathBuf),
}

#[derive(serde::Deserialize, serde::Serialize, Default)]
#[serde(default)]
struct KeyFile {
    jwt_secret: String,
    encryption_key: String,
    hashing_key: String,
}

/// Path of the keys file.
pub fn path(config: &Config) -> PathBuf {
    config.config_dir.join(FILE_NAME)
}

fn render(k: &KeyFile) -> String {
    format!(
        "# Photovault server keys. Back this file up: without it no one can sign in and encrypted\n\
         # server-side fields cannot be read. Environment variables PHOTOVAULT_JWT_SECRET,\n\
         # PHOTOVAULT_ENCRYPTION_KEY and PHOTOVAULT_HASHING_KEY override these values.\n\
         jwt_secret = \"{}\"\nencryption_key = \"{}\"   # libsodium secretbox key\nhashing_key = \"{}\"      # generichash key\n",
        k.jwt_secret, k.encryption_key, k.hashing_key
    )
}

fn write_private(p: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = p.with_extension("toml.tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(contents.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, p)
}

fn fill(target: &mut String, from: &str) {
    if target.trim().is_empty() {
        *target = from.to_string();
    }
}

/// Fill empty secrets in `config` from the keys file, generating (and saving) any
/// still missing.
pub fn ensure(config: &mut Config) -> Result<Outcome, Failure> {
    let s = &config.secrets;
    if !s.jwt.is_empty() && !s.encryption.is_empty() && !s.hashing.is_empty() {
        return Ok(Outcome::FromConfig);
    }
    let p = path(config);
    let shown = p.display().to_string();
    let mut file = match std::fs::read_to_string(&p) {
        Ok(text) => toml::from_str::<KeyFile>(&text).map_err(|e| {
            Failure(format!(
                "cannot parse {shown}: {}; fix: restore it from your backup",
                e.to_string().lines().next().unwrap_or("")
            ))
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => KeyFile::default(),
        Err(e) => {
            return Err(Failure(format!(
                "cannot read {shown}: {e}; fix: chown {}:{} {shown} && chmod 600 {shown}",
                rustix::process::geteuid().as_raw(),
                rustix::process::getegid().as_raw()
            )));
        }
    };
    let mut generated = false;
    for (v, n) in [
        (&mut file.jwt_secret, 32),
        (&mut file.encryption_key, 32),
        (&mut file.hashing_key, 64),
    ] {
        if v.trim().is_empty() {
            crate::crypto::init();
            *v = crate::crypto::b64(&crate::crypto::random_bytes(n));
            generated = true;
        }
    }
    if generated {
        crate::preflight::check_writable_dir(&config.config_dir, "config folder")?;
        write_private(&p, &render(&file)).map_err(|e| {
            Failure(format!(
                "cannot write {shown}: {e}; fix: make the config folder writable"
            ))
        })?;
    }
    let Secrets {
        jwt,
        encryption,
        hashing,
    } = &mut config.secrets;
    fill(jwt, &file.jwt_secret);
    fill(encryption, &file.encryption_key);
    fill(hashing, &file.hashing_key);
    Ok(if generated {
        Outcome::Generated(p)
    } else {
        Outcome::Loaded(p)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_then_reload_and_env_wins() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let mut c = Config {
            config_dir: t.path().to_path_buf(),
            ..Config::default()
        };
        let out = ensure(&mut c).unwrap();
        assert_eq!(out, Outcome::Generated(t.path().join(FILE_NAME)));
        let mode = std::fs::metadata(t.path().join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        crate::preflight::check_keys(&c).unwrap();
        let first = c.secrets.clone();

        let mut again = Config {
            config_dir: t.path().to_path_buf(),
            ..Config::default()
        };
        assert!(matches!(ensure(&mut again).unwrap(), Outcome::Loaded(_)));
        assert_eq!(again.secrets.jwt, first.jwt);
        assert_eq!(again.secrets.hashing, first.hashing);

        let mut env = Config {
            config_dir: t.path().to_path_buf(),
            ..Config::default()
        };
        env.secrets.jwt = "from-env".into();
        ensure(&mut env).unwrap();
        assert_eq!(env.secrets.jwt, "from-env");
        assert_eq!(env.secrets.encryption, first.encryption);
    }
}
