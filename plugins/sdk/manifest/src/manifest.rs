//! `plugin.toml`, the plugin manifest.
//!
//! ```toml
//! [plugin]
//! id = "dev.photovault.auto-albums"
//! name = "Auto albums"
//! version = "0.1.0"
//! description = "Sort photos into albums by rules."
//! api_version = 1
//!
//! [permissions]
//! read = ["metadata"]
//! write = ["albums"]
//!
//! [hooks]
//! on_upload = true
//! schedule = { every_minutes = 1440 }
//! on_demand = [{ id = "run", label = "Apply rules now" }]
//!
//! [limits]
//! memory_mb = 32
//! timeout_ms = 20000
//! ```

use crate::permission::{PermissionSet, valid_host_pattern};
use crate::settings::Field;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The plugin ABI version this crate describes. The host refuses manifests
/// with a different `api_version`.
pub const API_VERSION: u32 = 1;

/// Shortest allowed schedule interval, so a plugin cannot keep a phone awake.
pub const MIN_SCHEDULE_MINUTES: u32 = 15;

/// A parsed, validated manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Identity and description.
    pub plugin: PluginInfo,
    /// Permissions the plugin asks for.
    #[serde(default)]
    pub permissions: PermissionSet,
    /// Which hooks the plugin implements.
    #[serde(default)]
    pub hooks: Hooks,
    /// Resource limits the plugin asks for. The host clamps them to its own
    /// maxima.
    #[serde(default)]
    pub limits: Limits,
    /// Settings page.
    #[serde(default)]
    pub settings: Vec<Field>,
}

/// The `[plugin]` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginInfo {
    /// Reverse-DNS id, e.g. `dev.photovault.auto-albums`. Stable across
    /// versions; updates must keep it.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Semantic version `MAJOR.MINOR.PATCH`.
    pub version: String,
    /// One or two sentences for the plugin list.
    #[serde(default)]
    pub description: String,
    /// Authors, free text.
    #[serde(default)]
    pub authors: Vec<String>,
    /// SPDX licence id.
    #[serde(default)]
    pub license: Option<String>,
    /// Project page.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Must equal [`API_VERSION`].
    pub api_version: u32,
}

/// The `[hooks]` table.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    /// Export `on_upload`: called once for each newly uploaded file.
    #[serde(default)]
    pub on_upload: bool,
    /// Export `on_schedule`: called periodically.
    #[serde(default)]
    pub schedule: Option<Schedule>,
    /// Export `on_demand`: one button per action on the plugin's page.
    #[serde(default)]
    pub on_demand: Vec<OnDemandAction>,
}

/// When a scheduled hook runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    /// Interval between runs, at least [`MIN_SCHEDULE_MINUTES`].
    pub every_minutes: u32,
}

/// A button that runs the `on_demand` hook with this action id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnDemandAction {
    /// Passed to the hook as [`crate::abi::HookEvent::OnDemand::action`].
    pub id: String,
    /// Button label.
    pub label: String,
    /// Ask "Are you sure?" before running.
    #[serde(default)]
    pub confirm: bool,
}

/// Requested resource limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Maximum linear memory, in MiB.
    #[serde(default = "Limits::default_memory_mb")]
    pub memory_mb: u32,
    /// Maximum wall-clock time of one hook call, in milliseconds.
    #[serde(default = "Limits::default_timeout_ms")]
    pub timeout_ms: u64,
}

impl Limits {
    fn default_memory_mb() -> u32 {
        32
    }
    fn default_timeout_ms() -> u64 {
        10_000
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_mb: Self::default_memory_mb(),
            timeout_ms: Self::default_timeout_ms(),
        }
    }
}

/// Why a manifest was rejected.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// Not valid TOML, or does not match the schema.
    #[error("invalid plugin.toml: {0}")]
    Parse(#[from] toml::de::Error),
    /// Parsed, but a value is not allowed.
    #[error("invalid plugin.toml: {0}")]
    Invalid(String),
}

impl Manifest {
    /// Parse and validate `plugin.toml` text.
    pub fn from_toml(text: &str) -> Result<Self, ManifestError> {
        let m: Manifest = toml::from_str(text)?;
        m.validate()?;
        Ok(m)
    }

    /// Serialise back to TOML.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("manifest always serialises")
    }

    /// Check every rule that serde cannot express.
    pub fn validate(&self) -> Result<(), ManifestError> {
        let bad = |m: String| Err(ManifestError::Invalid(m));
        let p = &self.plugin;
        if p.api_version != API_VERSION {
            return bad(format!(
                "api_version {} is not supported (this host speaks {API_VERSION})",
                p.api_version
            ));
        }
        if !valid_plugin_id(&p.id) {
            return bad(format!(
                "id `{}` must be reverse-DNS like `com.example.my-plugin`",
                p.id
            ));
        }
        if p.name.trim().is_empty() || p.name.chars().count() > 64 {
            return bad("name must be 1-64 characters".into());
        }
        if parse_version(&p.version).is_none() {
            return bad(format!("version `{}` must be MAJOR.MINOR.PATCH", p.version));
        }
        for host in &self.permissions.network {
            if !valid_host_pattern(host) {
                return bad(format!("network host `{host}` is not a valid host pattern"));
            }
        }
        if let Some(s) = self.hooks.schedule
            && s.every_minutes < MIN_SCHEDULE_MINUTES
        {
            return bad(format!(
                "schedule.every_minutes must be at least {MIN_SCHEDULE_MINUTES}"
            ));
        }
        let mut ids = BTreeSet::new();
        for a in &self.hooks.on_demand {
            if a.id.is_empty() || a.label.trim().is_empty() || !ids.insert(a.id.as_str()) {
                return bad(format!("on_demand action `{}` is empty or repeated", a.id));
            }
        }
        if self.limits.memory_mb == 0 || self.limits.timeout_ms == 0 {
            return bad("limits must be greater than zero".into());
        }
        let mut keys = BTreeSet::new();
        for f in &self.settings {
            f.check_definition()
                .map_err(|e| ManifestError::Invalid(e.to_string()))?;
            if !keys.insert(f.key.as_str()) {
                return bad(format!("setting `{}` is repeated", f.key));
            }
        }
        Ok(())
    }

    /// The parsed `(major, minor, patch)` version.
    pub fn version(&self) -> (u64, u64, u64) {
        parse_version(&self.plugin.version).expect("validated")
    }
}

/// `true` for ids like `com.example.my-plugin`: two or more dot-separated
/// labels of `a-z`, `0-9` and `-`, at most 128 characters.
pub fn valid_plugin_id(id: &str) -> bool {
    id.len() <= 128
        && id.split('.').count() >= 2
        && id.split('.').all(|l| {
            !l.is_empty()
                && !l.starts_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// Parse `MAJOR.MINOR.PATCH` (pre-release and build suffixes are not allowed).
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.split('.');
    let mut n = || -> Option<u64> {
        let s = it.next()?;
        if s.is_empty() || (s.len() > 1 && s.starts_with('0')) {
            return None;
        }
        s.parse().ok()
    };
    let r = (n()?, n()?, n()?);
    it.next().is_none().then_some(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Permission, ReadScope, WriteScope};

    const SAMPLE: &str = r#"
        [plugin]
        id = "dev.photovault.auto-albums"
        name = "Auto albums"
        version = "0.1.0"
        api_version = 1

        [permissions]
        read = ["metadata"]
        write = ["albums"]
        network = ["api.example.com"]

        [hooks]
        on_upload = true
        schedule = { every_minutes = 60 }
        on_demand = [{ id = "run", label = "Run now" }]

        [[settings]]
        key = "rules"
        label = "Rules"
        type = "text"
        multiline = true
    "#;

    #[test]
    fn parses_sample() {
        let m = Manifest::from_toml(SAMPLE).unwrap();
        assert_eq!(m.version(), (0, 1, 0));
        assert!(
            m.permissions
                .contains(&Permission::Read(ReadScope::Metadata))
        );
        assert!(
            m.permissions
                .contains(&Permission::Write(WriteScope::Albums))
        );
        assert_eq!(m.limits, Limits::default());
        // Round trip.
        assert_eq!(Manifest::from_toml(&m.to_toml()).unwrap(), m);
    }

    #[test]
    fn rejects_bad_values() {
        for (from, to) in [
            ("api_version = 1", "api_version = 2"),
            ("dev.photovault.auto-albums", "Auto Albums"),
            ("0.1.0", "0.1"),
            ("every_minutes = 60", "every_minutes = 1"),
            ("api.example.com", "*"),
            ("read = [\"metadata\"]", "read = [\"everything\"]"),
        ] {
            let text = SAMPLE.replace(from, to);
            assert!(Manifest::from_toml(&text).is_err(), "accepted {to}");
        }
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("01.2.3"), None);
        assert_eq!(parse_version("1.2.3-rc1"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }
}
