//! The on-device plugin store.
//!
//! ```text
//! <root>/pins.json                     plugin id → pinned publisher key
//! <root>/plugins/<id>/plugin.pvplugin  the signed package as installed
//! <root>/plugins/<id>/state.json       grants, enabled, settings, export root
//! <root>/plugins/<id>/kv.json          the plugin's key/value storage
//! ```
//!
//! Every file is written atomically (temporary file, then rename). Packages
//! are verified again when the registry is opened, so a package changed on
//! disk is not loaded.
//!
//! Publisher keys are trusted on first use: the first install of a plugin id
//! pins its key, and every later install or update of that id must be signed
//! by the same key. The pin survives [`Registry::remove`]; only
//! [`Registry::forget_publisher`] clears it.

use crate::fsutil::write_atomic;
use crate::library::Library;
use crate::package::{Package, PackageError, PublicKey};
use crate::runtime::{
    HostLimits, InstanceEnv, PluginInstance, Policy, PolicySource, ProgressFn, RuntimeError,
};
use photovault_plugin_manifest::abi::Runner;
use photovault_plugin_manifest::settings::{
    SettingsError, Values, effective_values, validate_values,
};
use photovault_plugin_manifest::{Manifest, Permission, PermissionSet};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

const PACKAGE_FILE: &str = "plugin.pvplugin";
const STATE_FILE: &str = "state.json";
const KV_FILE: &str = "kv.json";
const PINS_FILE: &str = "pins.json";

/// Why a registry operation failed.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The package is malformed or its signature does not verify.
    #[error(transparent)]
    Package(#[from] PackageError),
    /// A plugin with this id is already installed (use update).
    #[error("plugin `{0}` is already installed")]
    AlreadyInstalled(String),
    /// No plugin with this id is installed.
    #[error("plugin `{0}` is not installed")]
    NotInstalled(String),
    /// The package is signed by a different key than the pinned one.
    #[error(
        "plugin `{id}` is signed by a different publisher key ({offered}) than the one trusted before ({pinned})"
    )]
    KeyMismatch {
        /// Plugin id.
        id: String,
        /// Fingerprint of the pinned key.
        pinned: String,
        /// Fingerprint of the key in the package.
        offered: String,
    },
    /// An update's version is not strictly higher.
    #[error("version {offered} is not newer than the installed {installed}")]
    NotNewer {
        /// Installed version.
        installed: String,
        /// Offered version.
        offered: String,
    },
    /// Permissions were granted that the plugin does not request.
    #[error("permissions not requested by the plugin: {0:?}")]
    NotRequested(Vec<String>),
    /// A settings value is invalid.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// The export folder is not an absolute path to an existing folder.
    #[error("export folder must be an absolute path to an existing folder")]
    BadExportRoot,
    /// Reading or writing the registry directory failed.
    #[error("plugin storage: {0}")]
    Io(#[from] std::io::Error),
    /// Stored state is corrupt.
    #[error("corrupt plugin state: {0}")]
    Corrupt(String),
    /// Loading the plugin into the runtime failed.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}

/// Whether the publisher key of a package is already trusted for its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    /// No key is pinned for this id yet; installing pins this one.
    New,
    /// The key matches the pinned key.
    Pinned,
    /// A different key is pinned: install and update will be refused.
    Mismatch,
}

/// One requested permission, ready to show on an install screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    /// The permission.
    pub permission: Permission,
    /// A short sentence describing it ([`Permission::describe`]).
    pub description: String,
    /// Whether to highlight it ([`Permission::is_sensitive`]).
    pub sensitive: bool,
}

impl PermissionRequest {
    fn list(set: &PermissionSet) -> Vec<Self> {
        set.iter()
            .map(|p| Self {
                description: p.describe(),
                sensitive: p.is_sensitive(),
                permission: p,
            })
            .collect()
    }
}

/// What the app shows before the user confirms an install or update.
#[derive(Debug, Clone)]
pub struct InstallRequest {
    /// The package's manifest.
    pub manifest: Manifest,
    /// The publisher key.
    pub publisher: PublicKey,
    /// Its fingerprint.
    pub fingerprint: String,
    /// Requested permissions with descriptions.
    pub permissions: Vec<PermissionRequest>,
    /// Whether the key is new, pinned or a mismatch.
    pub key_status: KeyStatus,
    /// The installed version of the same id, if any.
    pub installed_version: Option<String>,
}

/// The result of [`Registry::update`].
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateReport {
    /// Version before.
    pub from_version: String,
    /// Version after.
    pub to_version: String,
    /// Permissions the new version requests that were not requested before.
    /// They are **not** granted; ask the user and call [`Registry::grant`].
    pub new_permissions: Vec<PermissionRequest>,
    /// Grants dropped because the new version no longer requests them.
    pub dropped_permissions: PermissionSet,
}

/// The persisted per-plugin state (`state.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredState {
    publisher: PublicKey,
    granted: PermissionSet,
    enabled: bool,
    #[serde(default)]
    settings: Values,
    #[serde(default)]
    export_root: Option<PathBuf>,
    #[serde(default)]
    last_run: Option<i64>,
}

/// An installed plugin, as the app's plugin list shows it.
#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    /// The manifest.
    pub manifest: Manifest,
    /// The publisher key.
    pub publisher: PublicKey,
    /// Its fingerprint.
    pub fingerprint: String,
    /// What the plugin requests.
    pub requested: Vec<PermissionRequest>,
    /// What the user granted.
    pub granted: PermissionSet,
    /// Enabled.
    pub enabled: bool,
    /// Stored settings (only the values the user changed).
    pub settings: Values,
    /// Settings with defaults filled in, as hooks see them.
    pub effective_settings: Values,
    /// The export folder, if chosen.
    pub export_root: Option<PathBuf>,
    /// Last successful scheduled run (µs since the epoch).
    pub last_run: Option<i64>,
}

struct Entry {
    package: Package,
    state: StoredState,
}

#[derive(Default)]
struct Inner {
    pins: BTreeMap<String, PublicKey>,
    entries: BTreeMap<String, Entry>,
}

/// A directory-backed plugin store. Share it as `Arc<Registry>`: it is also
/// the [`PolicySource`] running instances consult on every host call.
pub struct Registry {
    root: PathBuf,
    inner: Mutex<Inner>,
    load_errors: Vec<(String, String)>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), RegistryError> {
    let bytes = serde_json::to_vec_pretty(v).map_err(|e| RegistryError::Corrupt(e.to_string()))?;
    write_atomic(path, &bytes)?;
    Ok(())
}

impl Registry {
    /// Open (creating if needed) a registry at `root`. Plugins whose package
    /// no longer verifies, or whose state is corrupt, are skipped and listed
    /// in [`Registry::load_errors`].
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, RegistryError> {
        let root = root.into();
        std::fs::create_dir_all(root.join("plugins"))?;
        let mut inner = Inner::default();
        match std::fs::read(root.join(PINS_FILE)) {
            Ok(b) => {
                inner.pins = serde_json::from_slice(&b)
                    .map_err(|e| RegistryError::Corrupt(format!("{PINS_FILE}: {e}")))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let mut load_errors = Vec::new();
        for dirent in std::fs::read_dir(root.join("plugins"))? {
            let dirent = dirent?;
            let id = dirent.file_name().to_string_lossy().into_owned();
            if !dirent.file_type()?.is_dir() {
                continue;
            }
            match Self::load_entry(&dirent.path(), &id, &inner.pins) {
                Ok(Some(e)) => {
                    inner.entries.insert(id, e);
                }
                Ok(None) => {}
                Err(e) => load_errors.push((id, e.to_string())),
            }
        }
        Ok(Self {
            root,
            inner: Mutex::new(inner),
            load_errors,
        })
    }

    fn load_entry(
        dir: &Path,
        id: &str,
        pins: &BTreeMap<String, PublicKey>,
    ) -> Result<Option<Entry>, RegistryError> {
        let bytes = match std::fs::read(dir.join(PACKAGE_FILE)) {
            Ok(b) => b,
            // A directory without a package is a leftover; ignore it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let package = Package::parse(&bytes)?;
        if package.manifest().plugin.id != id {
            return Err(RegistryError::Corrupt(
                "package id does not match its folder".into(),
            ));
        }
        let state: StoredState = serde_json::from_slice(&std::fs::read(dir.join(STATE_FILE))?)
            .map_err(|e| RegistryError::Corrupt(e.to_string()))?;
        if state.publisher != package.publisher() || pins.get(id) != Some(&package.publisher()) {
            return Err(RegistryError::Corrupt(
                "publisher key does not match the pin".into(),
            ));
        }
        Ok(Some(Entry { package, state }))
    }

    /// Plugins skipped by [`Registry::open`], with the reason.
    pub fn load_errors(&self) -> &[(String, String)] {
        &self.load_errors
    }

    /// The registry directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn dir(&self, id: &str) -> PathBuf {
        // Ids are validated reverse-DNS names ([a-z0-9-] labels), safe as a
        // single path component.
        self.root.join("plugins").join(id)
    }

    /// The key/value file of plugin `id`.
    pub fn kv_path(&self, id: &str) -> PathBuf {
        self.dir(id).join(KV_FILE)
    }

    fn save_state(&self, id: &str, state: &StoredState) -> Result<(), RegistryError> {
        write_json(&self.dir(id).join(STATE_FILE), state)
    }

    fn save_pins(&self, pins: &BTreeMap<String, PublicKey>) -> Result<(), RegistryError> {
        write_json(&self.root.join(PINS_FILE), pins)
    }

    fn key_status(pins: &BTreeMap<String, PublicKey>, id: &str, key: &PublicKey) -> KeyStatus {
        match pins.get(id) {
            None => KeyStatus::New,
            Some(k) if k == key => KeyStatus::Pinned,
            Some(_) => KeyStatus::Mismatch,
        }
    }

    fn check_key(pins: &BTreeMap<String, PublicKey>, p: &Package) -> Result<(), RegistryError> {
        let id = &p.manifest().plugin.id;
        if Self::key_status(pins, id, &p.publisher()) == KeyStatus::Mismatch {
            return Err(RegistryError::KeyMismatch {
                id: id.clone(),
                pinned: pins[id].fingerprint(),
                offered: p.publisher().fingerprint(),
            });
        }
        Ok(())
    }

    /// Verify a package and describe it for the install (or update) screen.
    pub fn inspect(&self, bytes: &[u8]) -> Result<InstallRequest, RegistryError> {
        let p = Package::parse(bytes)?;
        let inner = self.lock();
        let m = p.manifest();
        Ok(InstallRequest {
            manifest: m.clone(),
            publisher: p.publisher(),
            fingerprint: p.publisher().fingerprint(),
            permissions: PermissionRequest::list(&m.permissions),
            key_status: Self::key_status(&inner.pins, &m.plugin.id, &p.publisher()),
            installed_version: inner
                .entries
                .get(&m.plugin.id)
                .map(|e| e.package.manifest().plugin.version.clone()),
        })
    }

    /// Install a new plugin with the permissions the user approved
    /// (`granted` must be a subset of what the manifest requests). Pins the
    /// publisher key on first install. The plugin starts enabled.
    pub fn install(
        &self,
        bytes: &[u8],
        granted: PermissionSet,
    ) -> Result<InstalledPlugin, RegistryError> {
        let package = Package::parse(bytes)?;
        let id = package.manifest().plugin.id.clone();
        let mut inner = self.lock();
        if inner.entries.contains_key(&id) {
            return Err(RegistryError::AlreadyInstalled(id));
        }
        Self::check_key(&inner.pins, &package)?;
        let extra = granted.difference(&package.manifest().permissions);
        if !extra.is_empty() {
            return Err(RegistryError::NotRequested(
                extra.iter().map(|p| p.to_string()).collect(),
            ));
        }
        let dir = self.dir(&id);
        std::fs::create_dir_all(&dir)?;
        let state = StoredState {
            publisher: package.publisher(),
            granted,
            enabled: true,
            settings: Values::new(),
            export_root: None,
            last_run: None,
        };
        // Pin first, so a crash never leaves an installed but unpinned plugin.
        if !inner.pins.contains_key(&id) {
            let mut pins = inner.pins.clone();
            pins.insert(id.clone(), package.publisher());
            self.save_pins(&pins)?;
            inner.pins = pins;
        }
        self.save_state(&id, &state)?;
        write_atomic(&dir.join(PACKAGE_FILE), &package.to_bytes())?;
        let entry = Entry { package, state };
        let info = Self::info(&entry);
        inner.entries.insert(id, entry);
        Ok(info)
    }

    /// Replace an installed plugin with a newer version of the same id,
    /// signed by the pinned key. Grants the new version no longer requests
    /// are dropped; permissions it newly requests are **not** granted and are
    /// returned so the app can ask the user. Settings that are no longer
    /// valid are dropped.
    pub fn update(&self, bytes: &[u8]) -> Result<UpdateReport, RegistryError> {
        let package = Package::parse(bytes)?;
        let id = package.manifest().plugin.id.clone();
        let mut inner = self.lock();
        Self::check_key(&inner.pins, &package)?;
        let Inner { entries, .. } = &mut *inner;
        let entry = entries
            .get_mut(&id)
            .ok_or_else(|| RegistryError::NotInstalled(id.clone()))?;
        let old = entry.package.manifest();
        let new = package.manifest();
        if new.version() <= old.version() {
            return Err(RegistryError::NotNewer {
                installed: old.plugin.version.clone(),
                offered: new.plugin.version.clone(),
            });
        }
        let new_permissions = new.permissions.difference(&old.permissions);
        let dropped = entry.state.granted.difference(&new.permissions);
        let mut state = entry.state.clone();
        state.granted = state.granted.intersection(&new.permissions);
        state.publisher = package.publisher();
        state.settings.retain(|k, v| {
            new.settings
                .iter()
                .any(|f| &f.key == k && f.validate(v).is_ok())
        });
        let report = UpdateReport {
            from_version: old.plugin.version.clone(),
            to_version: new.plugin.version.clone(),
            new_permissions: PermissionRequest::list(&new_permissions),
            dropped_permissions: dropped,
        };
        // Narrow grants on disk before the new code is in place.
        self.save_state(&id, &state)?;
        write_atomic(&self.dir(&id).join(PACKAGE_FILE), &package.to_bytes())?;
        entry.package = package;
        entry.state = state;
        Ok(report)
    }

    /// Uninstall a plugin and delete its state and storage. The publisher
    /// pin is kept.
    pub fn remove(&self, id: &str) -> Result<(), RegistryError> {
        let mut inner = self.lock();
        if inner.entries.remove(id).is_none() {
            return Err(RegistryError::NotInstalled(id.into()));
        }
        // Remove the package first so a partial delete is not reloaded.
        let dir = self.dir(id);
        std::fs::remove_file(dir.join(PACKAGE_FILE))?;
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// Forget the pinned publisher key of a plugin that is not installed, so
    /// a package from another publisher can be installed under that id.
    pub fn forget_publisher(&self, id: &str) -> Result<(), RegistryError> {
        let mut inner = self.lock();
        if inner.entries.contains_key(id) {
            return Err(RegistryError::AlreadyInstalled(id.into()));
        }
        if inner.pins.contains_key(id) {
            let mut pins = inner.pins.clone();
            pins.remove(id);
            self.save_pins(&pins)?;
            inner.pins = pins;
        }
        Ok(())
    }

    fn modify<T>(
        &self,
        id: &str,
        f: impl FnOnce(&Manifest, &mut StoredState) -> Result<T, RegistryError>,
    ) -> Result<T, RegistryError> {
        let mut inner = self.lock();
        let entry = inner
            .entries
            .get_mut(id)
            .ok_or_else(|| RegistryError::NotInstalled(id.into()))?;
        let mut state = entry.state.clone();
        let out = f(entry.package.manifest(), &mut state)?;
        self.save_state(id, &state)?;
        entry.state = state;
        Ok(out)
    }

    /// Enable or disable a plugin. Takes effect on the next host call of any
    /// running instance.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), RegistryError> {
        self.modify(id, |_, s| {
            s.enabled = enabled;
            Ok(())
        })
    }

    /// Enable a plugin.
    pub fn enable(&self, id: &str) -> Result<(), RegistryError> {
        self.set_enabled(id, true)
    }

    /// Disable a plugin.
    pub fn disable(&self, id: &str) -> Result<(), RegistryError> {
        self.set_enabled(id, false)
    }

    /// Grant more permissions; each must be requested by the manifest.
    pub fn grant(&self, id: &str, perms: &PermissionSet) -> Result<(), RegistryError> {
        self.modify(id, |m, s| {
            let extra = perms.difference(&m.permissions);
            if !extra.is_empty() {
                return Err(RegistryError::NotRequested(
                    extra.iter().map(|p| p.to_string()).collect(),
                ));
            }
            for p in perms.iter() {
                s.granted.insert(p);
            }
            Ok(())
        })
    }

    /// Revoke permissions. Takes effect on the next host call of any running
    /// instance.
    pub fn revoke(&self, id: &str, perms: &PermissionSet) -> Result<(), RegistryError> {
        self.modify(id, |_, s| {
            for p in perms.iter() {
                s.granted.remove(&p);
            }
            Ok(())
        })
    }

    /// Replace the stored settings with `values`, after validating them
    /// against the manifest's settings page.
    pub fn set_settings(&self, id: &str, values: Values) -> Result<(), RegistryError> {
        self.modify(id, |m, s| {
            validate_values(&m.settings, &values)?;
            s.settings = values;
            Ok(())
        })
    }

    /// Choose (or clear) the folder `ExportFile` writes into. It must be an
    /// absolute path to an existing folder; the plugin never learns it.
    pub fn set_export_root(&self, id: &str, root: Option<PathBuf>) -> Result<(), RegistryError> {
        if let Some(r) = &root
            && (!r.is_absolute() || !r.is_dir())
        {
            return Err(RegistryError::BadExportRoot);
        }
        self.modify(id, |_, s| {
            s.export_root = root;
            Ok(())
        })
    }

    /// Record a successful scheduled run at `now` (µs since the epoch).
    pub fn record_run(&self, id: &str, now: i64) -> Result<(), RegistryError> {
        self.modify(id, |_, s| {
            s.last_run = Some(now);
            Ok(())
        })
    }

    fn info(e: &Entry) -> InstalledPlugin {
        let m = e.package.manifest();
        InstalledPlugin {
            manifest: m.clone(),
            publisher: e.package.publisher(),
            fingerprint: e.package.publisher().fingerprint(),
            requested: PermissionRequest::list(&m.permissions),
            granted: e.state.granted.clone(),
            enabled: e.state.enabled,
            settings: e.state.settings.clone(),
            effective_settings: effective_values(&m.settings, &e.state.settings),
            export_root: e.state.export_root.clone(),
            last_run: e.state.last_run,
        }
    }

    /// Every installed plugin, ordered by id.
    pub fn list(&self) -> Vec<InstalledPlugin> {
        self.lock().entries.values().map(Self::info).collect()
    }

    /// One installed plugin.
    pub fn get(&self, id: &str) -> Option<InstalledPlugin> {
        self.lock().entries.get(id).map(Self::info)
    }

    /// The installed (verified) package of a plugin.
    pub fn package(&self, id: &str) -> Option<Package> {
        self.lock().entries.get(id).map(|e| e.package.clone())
    }

    /// The pinned publisher key for an id, installed or not.
    pub fn pinned_key(&self, id: &str) -> Option<PublicKey> {
        self.lock().pins.get(id).copied()
    }

    /// Load an installed plugin into the sandboxed runtime. The instance
    /// consults this registry for its policy on every host call.
    pub fn instantiate(
        self: &Arc<Self>,
        id: &str,
        library: Arc<dyn Library>,
        limits: HostLimits,
        runner: Runner,
        progress: Option<ProgressFn>,
    ) -> Result<PluginInstance, RegistryError> {
        let package = self
            .package(id)
            .ok_or_else(|| RegistryError::NotInstalled(id.into()))?;
        let env = InstanceEnv {
            library,
            policy: self.clone(),
            kv_path: self.kv_path(id),
            limits,
            runner,
            progress,
        };
        Ok(PluginInstance::load(&package, env)?)
    }
}

impl PolicySource for Registry {
    fn policy(&self, plugin_id: &str) -> Option<Policy> {
        self.lock().entries.get(plugin_id).map(|e| Policy {
            enabled: e.state.enabled,
            granted: e.state.granted.clone(),
            settings: e.state.settings.clone(),
            export_root: e.state.export_root.clone(),
            last_run: e.state.last_run,
        })
    }
}
