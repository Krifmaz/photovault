//! Registry lifecycle: install, TOFU key pinning, updates, grants, settings.

mod common;

use common::{EMPTY_WASM, manifest, pack};
use photovault_plugin_host::manifest::{Permission, PermissionSet, ReadScope, WriteScope};
use photovault_plugin_host::runtime::PolicySource;
use photovault_plugin_host::{KeyStatus, Registry, RegistryError, SigningKey};
use serde_json::json;

const ID: &str = "dev.photovault.lifecycle";

fn v1() -> String {
    manifest(
        ID,
        "1.0.0",
        r#"
[permissions]
read = ["metadata", "thumbnails"]
write = ["albums"]

[[settings]]
key = "days"
label = "Days"
type = "number"
min = 0
max = 30
default = 7
"#,
    )
}

fn v2() -> String {
    // Drops read:thumbnails, adds write:trash and a network host.
    manifest(
        ID,
        "1.1.0",
        r#"
[permissions]
read = ["metadata"]
write = ["albums", "trash"]
network = ["api.example.com"]

[[settings]]
key = "days"
label = "Days"
type = "number"
min = 0
max = 10
default = 7
"#,
    )
}

fn set(perms: &[Permission]) -> PermissionSet {
    PermissionSet::from_permissions(perms.iter().cloned())
}

fn meta() -> Permission {
    Permission::Read(ReadScope::Metadata)
}
fn thumbs() -> Permission {
    Permission::Read(ReadScope::Thumbnails)
}
fn albums() -> Permission {
    Permission::Write(WriteScope::Albums)
}

#[test]
fn full_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(dir.path()).unwrap();
    let key = SigningKey::generate();
    let pkg = pack(&v1(), EMPTY_WASM, &key);

    // Inspect before install.
    let req = reg.inspect(&pkg).unwrap();
    assert_eq!(req.key_status, KeyStatus::New);
    assert_eq!(req.fingerprint, key.public_key().fingerprint());
    assert_eq!(req.installed_version, None);
    assert_eq!(req.permissions.len(), 3);
    assert!(req.permissions.iter().all(|p| !p.description.is_empty()));

    // Granting something not requested is refused.
    let err = reg
        .install(&pkg, set(&[Permission::Write(WriteScope::Trash)]))
        .unwrap_err();
    assert!(matches!(err, RegistryError::NotRequested(_)), "{err}");

    // Install with a subset.
    let info = reg.install(&pkg, set(&[meta(), albums()])).unwrap();
    assert!(info.enabled);
    assert_eq!(info.granted, set(&[meta(), albums()]));
    assert_eq!(reg.pinned_key(ID), Some(key.public_key()));
    assert!(matches!(
        reg.install(&pkg, PermissionSet::none()),
        Err(RegistryError::AlreadyInstalled(_))
    ));
    assert_eq!(reg.inspect(&pkg).unwrap().key_status, KeyStatus::Pinned);

    // Grant and revoke.
    reg.grant(ID, &set(&[thumbs()])).unwrap();
    assert!(reg.get(ID).unwrap().granted.contains(&thumbs()));
    assert!(
        reg.grant(ID, &set(&[Permission::Network("x.example.com".into())]))
            .is_err()
    );
    reg.revoke(ID, &set(&[albums()])).unwrap();
    assert_eq!(reg.get(ID).unwrap().granted, set(&[meta(), thumbs()]));
    assert_eq!(reg.policy(ID).unwrap().granted, set(&[meta(), thumbs()]));

    // Enable / disable.
    reg.disable(ID).unwrap();
    assert!(!reg.policy(ID).unwrap().enabled);
    reg.enable(ID).unwrap();
    assert!(reg.policy(ID).unwrap().enabled);

    // Settings are validated.
    let mut values = serde_json::Map::new();
    values.insert("days".into(), json!(40));
    assert!(matches!(
        reg.set_settings(ID, values.clone()),
        Err(RegistryError::Settings(_))
    ));
    values.insert("days".into(), json!(20));
    reg.set_settings(ID, values.clone()).unwrap();
    assert_eq!(reg.get(ID).unwrap().effective_settings["days"], json!(20));
    let mut unknown = serde_json::Map::new();
    unknown.insert("nope".into(), json!(1));
    assert!(reg.set_settings(ID, unknown).is_err());

    // Export root must be an absolute existing folder.
    assert!(matches!(
        reg.set_export_root(ID, Some("relative/path".into())),
        Err(RegistryError::BadExportRoot)
    ));
    assert!(
        reg.set_export_root(ID, Some(dir.path().join("missing")))
            .is_err()
    );
    reg.set_export_root(ID, Some(dir.path().to_path_buf()))
        .unwrap();

    // Everything survives a reopen.
    drop(reg);
    let reg = Registry::open(dir.path()).unwrap();
    assert!(reg.load_errors().is_empty());
    let info = reg.get(ID).unwrap();
    assert_eq!(info.granted, set(&[meta(), thumbs()]));
    assert_eq!(info.settings["days"], json!(20));
    assert_eq!(info.export_root.as_deref(), Some(dir.path()));
    assert_eq!(reg.list().len(), 1);

    // Update: higher version, same key.
    let pkg2 = pack(&v2(), EMPTY_WASM, &key);
    let req = reg.inspect(&pkg2).unwrap();
    assert_eq!(req.installed_version.as_deref(), Some("1.0.0"));
    let report = reg.update(&pkg2).unwrap();
    assert_eq!(report.from_version, "1.0.0");
    assert_eq!(report.to_version, "1.1.0");
    let new: Vec<String> = report
        .new_permissions
        .iter()
        .map(|p| p.permission.to_string())
        .collect();
    assert_eq!(new, ["write:trash", "network:api.example.com"]);
    assert_eq!(report.dropped_permissions, set(&[thumbs()]));
    let info = reg.get(ID).unwrap();
    // New permissions are not granted automatically; dropped ones are gone.
    assert_eq!(info.granted, set(&[meta()]));
    assert_eq!(info.manifest.plugin.version, "1.1.0");
    // days = 20 is invalid under v2 (max 10) and was dropped.
    assert!(info.settings.get("days").is_none());
    assert_eq!(info.effective_settings["days"], json!(7.0));

    // Remove keeps the pin.
    reg.remove(ID).unwrap();
    assert!(reg.get(ID).is_none());
    assert!(reg.policy(ID).is_none());
    assert!(!dir.path().join("plugins").join(ID).exists());
    assert_eq!(reg.pinned_key(ID), Some(key.public_key()));
    assert!(matches!(
        reg.remove(ID),
        Err(RegistryError::NotInstalled(_))
    ));
}

#[test]
fn tofu_rejects_other_keys() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(dir.path()).unwrap();
    let key = SigningKey::generate();
    let evil = SigningKey::generate();
    reg.install(&pack(&v1(), EMPTY_WASM, &key), PermissionSet::none())
        .unwrap();

    // An update signed by another key is refused.
    let bad = pack(&v2(), EMPTY_WASM, &evil);
    assert_eq!(reg.inspect(&bad).unwrap().key_status, KeyStatus::Mismatch);
    let err = reg.update(&bad).unwrap_err();
    assert!(matches!(err, RegistryError::KeyMismatch { .. }), "{err}");
    assert_eq!(reg.get(ID).unwrap().manifest.plugin.version, "1.0.0");

    // Even after removal, the pin blocks a reinstall from another key ...
    reg.remove(ID).unwrap();
    assert!(matches!(
        reg.install(&bad, PermissionSet::none()),
        Err(RegistryError::KeyMismatch { .. })
    ));
    // ... until the user explicitly forgets the publisher.
    reg.forget_publisher(ID).unwrap();
    reg.install(&bad, PermissionSet::none()).unwrap();
    assert_eq!(reg.pinned_key(ID), Some(evil.public_key()));
    // forget_publisher refuses while installed.
    assert!(reg.forget_publisher(ID).is_err());

    // Pins persist across reopen.
    drop(reg);
    let reg = Registry::open(dir.path()).unwrap();
    assert_eq!(reg.pinned_key(ID), Some(evil.public_key()));
}

#[test]
fn downgrade_and_same_version_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(dir.path()).unwrap();
    let key = SigningKey::generate();
    reg.install(&pack(&v2(), EMPTY_WASM, &key), PermissionSet::none())
        .unwrap();
    for version in ["1.1.0", "1.0.9", "0.9.0"] {
        let m = v2().replace("version = \"1.1.0\"", &format!("version = \"{version}\""));
        let err = reg.update(&pack(&m, EMPTY_WASM, &key)).unwrap_err();
        assert!(
            matches!(err, RegistryError::NotNewer { .. }),
            "{version}: {err}"
        );
    }
    // Updating a plugin that is not installed fails.
    let other = manifest("dev.photovault.other", "1.0.0", "");
    assert!(matches!(
        reg.update(&pack(&other, EMPTY_WASM, &key)),
        Err(RegistryError::NotInstalled(_))
    ));
    // Version comparison is numeric, not textual.
    let m = v2().replace("version = \"1.1.0\"", "version = \"1.10.0\"");
    reg.update(&pack(&m, EMPTY_WASM, &key)).unwrap();
}

#[test]
fn tampered_package_on_disk_is_not_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(dir.path()).unwrap();
    let key = SigningKey::generate();
    reg.install(&pack(&v1(), EMPTY_WASM, &key), PermissionSet::none())
        .unwrap();
    drop(reg);
    let path = dir.path().join("plugins").join(ID).join("plugin.pvplugin");
    let mut bytes = std::fs::read(&path).unwrap();
    let n = bytes.len();
    bytes[n - 1] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    let reg = Registry::open(dir.path()).unwrap();
    assert!(reg.get(ID).is_none());
    assert_eq!(reg.load_errors().len(), 1);
    assert_eq!(reg.load_errors()[0].0, ID);
}

#[test]
fn rejects_bad_packages() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(dir.path()).unwrap();
    assert!(reg.inspect(b"garbage").is_err());
    assert!(reg.install(b"PVPLUG01", PermissionSet::none()).is_err());
}
