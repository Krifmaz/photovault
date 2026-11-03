//! The three example plugins from `plugins/sdk/guest/examples`, compiled to
//! Wasm, signed, installed and run by the real host runtime.

use photovault_plugin_host::manifest::abi::{AlbumInfo, AlbumKind, FileType, Runner};
use photovault_plugin_host::{HostLimits, MemoryLibrary, Package, Registry, SigningKey};
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};

const DAY_US: i64 = 86_400_000_000;

fn guest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../guest")
}

/// Build the guest workspace once per test binary (fixed arguments, no shell)
/// and return the directory holding the `.wasm` files.
fn wasm_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let target_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guest");
        let status = Command::new(env!("CARGO"))
            .arg("build")
            .arg("--release")
            .arg("--locked")
            .arg("--target")
            .arg("wasm32-unknown-unknown")
            .arg("--manifest-path")
            .arg(guest_dir().join("Cargo.toml"))
            .arg("--target-dir")
            .arg(&target_dir)
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .status()
            .expect("run cargo");
        assert!(status.success(), "building the example plugins failed");
        target_dir.join("wasm32-unknown-unknown/release")
    })
}

struct Installed {
    _dir: tempfile::TempDir,
    reg: Arc<Registry>,
    id: String,
}

/// Sign and install an example with every permission it asks for.
fn install(example: &str, wasm_name: &str) -> Installed {
    let manifest = std::fs::read(
        guest_dir()
            .join("examples")
            .join(example)
            .join("plugin.toml"),
    )
    .expect("read plugin.toml");
    let wasm = std::fs::read(wasm_dir().join(wasm_name)).expect("read wasm");
    let pkg = Package::build(&manifest, &wasm, &SigningKey::generate()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let reg = Arc::new(Registry::open(dir.path().join("registry")).unwrap());
    let request = reg.inspect(&pkg).unwrap();
    let granted = request.manifest.permissions.clone();
    let id = request.manifest.plugin.id.clone();
    reg.install(&pkg, granted).unwrap();
    Installed { _dir: dir, reg, id }
}

fn limits() -> HostLimits {
    HostLimits {
        http_use_env_proxy: false,
        ..HostLimits::default()
    }
}

fn file(id: i64, title: &str) -> photovault_plugin_host::manifest::abi::FileInfo {
    MemoryLibrary::sample_file(id, title)
}

#[test]
fn auto_albums_sorts_by_rules() {
    let p = install("auto-albums", "auto_albums.wasm");
    let lib = Arc::new(MemoryLibrary::new());
    let mut video = file(1, "clip.mp4");
    video.file_type = FileType::Video;
    lib.add_file(video, None, None);
    lib.add_file(file(2, "IMG_0002.JPG"), None, None);
    let mut values = serde_json::Map::new();
    values.insert("rules".into(), json!("# test\nVideos: type=video"));
    p.reg.set_settings(&p.id, values).unwrap();

    let mut inst = p
        .reg
        .instantiate(&p.id, lib.clone(), limits(), Runner::Test, None)
        .unwrap();
    let out = inst.run_on_demand("apply_all").unwrap();
    assert_eq!(out.changed, 1, "{out:?}");
    assert_eq!(lib.file(1).unwrap().album_ids.len(), 1);
    assert!(lib.file(2).unwrap().album_ids.is_empty());

    // Idempotent: a second run changes nothing.
    let out = inst.run_on_demand("apply_all").unwrap();
    assert_eq!(out.changed, 0, "{out:?}");

    // The upload hook applies the rules to a single new file.
    let mut new_video = file(3, "new.mov");
    new_video.file_type = FileType::Video;
    lib.add_file(new_video, None, None);
    inst.run_upload(3).unwrap();
    assert_eq!(
        lib.file(3).unwrap().album_ids,
        lib.file(1).unwrap().album_ids
    );
}

#[test]
fn screenshot_cleanup_trashes_old_screenshots_but_not_favourites() {
    let p = install("screenshot-cleanup", "screenshot_cleanup.wasm");
    let lib = Arc::new(MemoryLibrary::new());
    let old = photovault_plugin_host::runtime::now_micros() - 90 * DAY_US;
    let mut shot = file(1, "Screenshot_20240101-120000.png");
    shot.device_folder = Some("Screenshots".into());
    shot.creation_time = old;
    let mut fav = shot.clone();
    fav.id = 2;
    fav.is_favorite = true;
    let mut recent = shot.clone();
    recent.id = 3;
    recent.creation_time = photovault_plugin_host::runtime::now_micros();
    let mut photo = file(4, "IMG_0004.JPG");
    photo.creation_time = old;
    for f in [shot, fav, recent, photo] {
        lib.add_file(f, None, None);
    }
    let mut values = serde_json::Map::new();
    values.insert("action".into(), json!("trash"));
    p.reg.set_settings(&p.id, values).unwrap();

    let mut inst = p
        .reg
        .instantiate(&p.id, lib.clone(), limits(), Runner::Test, None)
        .unwrap();
    inst.run_on_demand("scan").unwrap();
    assert_eq!(lib.trashed(), vec![1]);
}

#[test]
fn export_albums_copies_originals_into_the_export_folder() {
    let p = install("export-albums", "export_albums.wasm");
    let lib = Arc::new(MemoryLibrary::new());
    lib.add_album(AlbumInfo {
        id: 10,
        name: "Holiday: Rome".into(),
        kind: AlbumKind::Album,
        is_owned: true,
        file_count: None,
    });
    let mut f = file(1, "IMG_0001.JPG");
    f.album_ids = vec![10];
    lib.add_file(f, None, Some(b"jpeg bytes".to_vec()));

    let out_dir = tempfile::tempdir().unwrap();
    let root = out_dir.path().canonicalize().unwrap();
    p.reg.set_export_root(&p.id, Some(root.clone())).unwrap();
    let mut values = serde_json::Map::new();
    values.insert("albums".into(), json!([10]));
    p.reg.set_settings(&p.id, values).unwrap();

    let mut inst = p
        .reg
        .instantiate(&p.id, lib.clone(), limits(), Runner::Test, None)
        .unwrap();
    let out = inst.run_on_demand("export_now").unwrap();
    assert_eq!(out.changed, 1, "{out:?}");
    let written: Vec<_> = walk(&root);
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(std::fs::read(&written[0]).unwrap(), b"jpeg bytes");
    assert!(written[0].ends_with("IMG_0001.JPG"));

    // Incremental: nothing to do on the second run.
    let out = inst.run_on_demand("export_now").unwrap();
    assert_eq!(out.changed, 0, "{out:?}");
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
