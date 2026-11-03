//! The sandbox, end to end, with the real probe plugin compiled to Wasm.

mod common;

use common::{pack, probe_manifest, probe_wasm};
use photovault_plugin_host::manifest::abi::{AlbumInfo, AlbumKind, HookOutput, Runner};
use photovault_plugin_host::manifest::{Permission, PermissionSet, ReadScope, WriteScope};
use photovault_plugin_host::{
    Dispatcher, HostLimits, LibraryWrite, MemoryLibrary, PluginInstance, ProgressFn,
    ProgressUpdate, Registry, RuntimeError, SigningKey,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ID: &str = "dev.photovault.probe";

struct Fixture {
    dir: tempfile::TempDir,
    reg: Arc<Registry>,
    lib: Arc<MemoryLibrary>,
    key: SigningKey,
}

fn limits() -> HostLimits {
    HostLimits {
        http_use_env_proxy: false,
        ..HostLimits::default()
    }
}

fn library() -> Arc<MemoryLibrary> {
    let lib = MemoryLibrary::new();
    lib.add_album(AlbumInfo {
        id: 10,
        name: "Mine".into(),
        kind: AlbumKind::Album,
        is_owned: true,
        file_count: None,
    });
    lib.add_album(AlbumInfo {
        id: 11,
        name: "Shared with me".into(),
        kind: AlbumKind::Album,
        is_owned: false,
        file_count: None,
    });
    let mut f = MemoryLibrary::sample_file(1, "IMG_0001.HEIC");
    f.album_ids = vec![10];
    f.size = Some(8);
    lib.add_file(f, Some(b"thumb-1".to_vec()), Some(b"original".to_vec()));
    lib.add_file(MemoryLibrary::sample_file(2, "IMG_0002.JPG"), None, None);
    Arc::new(lib)
}

fn fixture_with(limits_toml: &str, grants: PermissionSet) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let reg = Arc::new(Registry::open(dir.path().join("registry")).unwrap());
    let key = SigningKey::generate();
    let pkg = pack(
        &probe_manifest(ID, "1.0.0", limits_toml),
        probe_wasm(),
        &key,
    );
    reg.install(&pkg, grants).unwrap();
    Fixture {
        dir,
        reg,
        lib: library(),
        key,
    }
}

fn fixture(grants: PermissionSet) -> Fixture {
    fixture_with("memory_mb = 32\ntimeout_ms = 2000", grants)
}

fn all() -> PermissionSet {
    PermissionSet::from_permissions([
        Permission::Read(ReadScope::Metadata),
        Permission::Read(ReadScope::Thumbnails),
        Permission::Read(ReadScope::Originals),
        Permission::Write(WriteScope::Tags),
        Permission::Write(WriteScope::Albums),
        Permission::Write(WriteScope::Trash),
        Permission::Write(WriteScope::Export),
        Permission::Network("127.0.0.1".into()),
        Permission::Network("allowed.example.com".into()),
    ])
}

fn one(p: Permission) -> PermissionSet {
    PermissionSet::from_permissions([p])
}

impl Fixture {
    fn instance(&self) -> PluginInstance {
        self.instance_with(limits(), None)
    }

    fn instance_with(&self, limits: HostLimits, progress: Option<ProgressFn>) -> PluginInstance {
        self.reg
            .instantiate(ID, self.lib.clone(), limits, Runner::Test, progress)
            .unwrap()
    }

    fn set_arg(&self, arg: &str) {
        let mut v = serde_json::Map::new();
        v.insert("arg".into(), json!(arg));
        self.reg.set_settings(ID, v).unwrap();
    }

    fn run(
        &self,
        inst: &mut PluginInstance,
        action: &str,
        arg: &str,
    ) -> Result<HookOutput, RuntimeError> {
        self.set_arg(arg);
        inst.run_on_demand(action)
    }

    /// Send one host call and return the decoded `Response`.
    fn call(&self, inst: &mut PluginInstance, request: Value) -> Value {
        let out = self.run(inst, "call", &request.to_string()).unwrap();
        serde_json::from_str(&out.summary.unwrap()).unwrap()
    }
}

fn denied(resp: &Value) -> bool {
    resp["err"]["code"] == "permission_denied"
}

fn every_request() -> Vec<(Value, Option<Permission>)> {
    vec![
        (
            json!({"method": "list_files", "params": {}}),
            Some(Permission::Read(ReadScope::Metadata)),
        ),
        (
            json!({"method": "get_file", "params": {"file_id": 1}}),
            Some(Permission::Read(ReadScope::Metadata)),
        ),
        (
            json!({"method": "list_albums"}),
            Some(Permission::Read(ReadScope::Metadata)),
        ),
        (
            json!({"method": "add_tags", "params": {"file_id": 1, "tags": ["sunset"]}}),
            Some(Permission::Write(WriteScope::Tags)),
        ),
        (
            json!({"method": "ensure_album", "params": {"name": "Plugin album"}}),
            Some(Permission::Write(WriteScope::Albums)),
        ),
        (
            json!({"method": "add_to_album", "params": {"album_id": 10, "file_ids": [2]}}),
            Some(Permission::Write(WriteScope::Albums)),
        ),
        (
            json!({"method": "remove_from_album", "params": {"album_id": 10, "file_ids": [1]}}),
            Some(Permission::Write(WriteScope::Albums)),
        ),
        (
            json!({"method": "trash_files", "params": {"file_ids": [2]}}),
            Some(Permission::Write(WriteScope::Trash)),
        ),
        (
            json!({"method": "export_file", "params": {"file_id": 1, "path": "x/y.heic"}}),
            Some(Permission::Write(WriteScope::Export)),
        ),
        (json!({"method": "kv_get", "params": {"key": "k"}}), None),
        (
            json!({"method": "kv_set", "params": {"key": "k", "value": 1}}),
            None,
        ),
        (json!({"method": "progress", "params": {"done": 1}}), None),
    ]
}

#[test]
fn every_method_denied_without_permission() {
    let fx = fixture(PermissionSet::none());
    let mut inst = fx.instance();
    for (req, perm) in every_request() {
        let resp = fx.call(&mut inst, req.clone());
        assert_eq!(denied(&resp), perm.is_some(), "{req}: {resp}");
    }
    assert!(fx.lib.writes().is_empty());
}

#[test]
fn every_method_allowed_with_its_permission() {
    let fx = fixture(PermissionSet::none());
    fx.reg
        .set_export_root(ID, Some(fx.dir.path().to_path_buf()))
        .unwrap();
    let mut inst = fx.instance();
    for (req, perm) in every_request() {
        if let Some(p) = &perm {
            // Granting a *different* permission does not help.
            let other = if *p == Permission::Write(WriteScope::Tags) {
                Permission::Write(WriteScope::Albums)
            } else {
                Permission::Write(WriteScope::Tags)
            };
            fx.reg.grant(ID, &one(other.clone())).unwrap();
            assert!(denied(&fx.call(&mut inst, req.clone())), "{req}");
            fx.reg.revoke(ID, &one(other)).unwrap();
            fx.reg.grant(ID, &one(p.clone())).unwrap();
        }
        let resp = fx.call(&mut inst, req.clone());
        assert!(resp.get("ok").is_some(), "{req}: {resp}");
        if let Some(p) = &perm {
            fx.reg.revoke(ID, &one(p.clone())).unwrap();
        }
    }
    assert_eq!(
        fx.lib.writes(),
        [
            LibraryWrite::AddTags {
                file_id: 1,
                tags: vec!["sunset".into()]
            },
            LibraryWrite::CreateAlbum {
                album_id: 12,
                name: "Plugin album".into()
            },
            LibraryWrite::AddToAlbum {
                album_id: 10,
                file_ids: vec![2]
            },
            LibraryWrite::RemoveFromAlbum {
                album_id: 10,
                file_ids: vec![1]
            },
            LibraryWrite::Trash { file_ids: vec![2] },
        ]
    );
    assert_eq!(
        std::fs::read(fx.dir.path().join("x/y.heic")).unwrap(),
        b"original"
    );
}

#[test]
fn results_and_host_rules() {
    let fx = fixture(all());
    let mut inst = fx.instance();
    let page = fx.call(&mut inst, json!({"method": "list_files", "params": {}}));
    assert_eq!(page["ok"]["files"].as_array().unwrap().len(), 2);
    let f = fx.call(
        &mut inst,
        json!({"method": "get_file", "params": {"file_id": 1}}),
    );
    assert_eq!(f["ok"]["title"], "IMG_0001.HEIC");
    let missing = fx.call(
        &mut inst,
        json!({"method": "get_file", "params": {"file_id": 99}}),
    );
    assert_eq!(missing["ok"], Value::Null);
    // Albums the user does not own cannot be changed.
    let r = fx.call(
        &mut inst,
        json!({"method": "add_to_album", "params": {"album_id": 11, "file_ids": [1]}}),
    );
    assert!(denied(&r), "{r}");
    let r = fx.call(
        &mut inst,
        json!({"method": "add_to_album", "params": {"album_id": 99, "file_ids": [1]}}),
    );
    assert_eq!(r["err"]["code"], "not_found");
    // Malformed requests and bad values.
    let r = fx.call(&mut inst, json!({"method": "format_disk"}));
    assert_eq!(r["err"]["code"], "invalid_request");
    let r = fx.call(
        &mut inst,
        json!({"method": "add_tags", "params": {"file_id": 1, "tags": [""]}}),
    );
    assert_eq!(r["err"]["code"], "invalid_request");
    let r = fx.call(
        &mut inst,
        json!({"method": "ensure_album", "params": {"name": "a\nb"}}),
    );
    assert_eq!(r["err"]["code"], "invalid_request");
    // Export without a chosen folder.
    let r = fx.call(
        &mut inst,
        json!({"method": "export_file", "params": {"file_id": 1, "path": "a.jpg"}}),
    );
    assert_eq!(r["err"]["code"], "invalid_request");
}

#[test]
fn page_size_is_capped() {
    let fx = fixture(one(Permission::Read(ReadScope::Metadata)));
    for id in 100..700 {
        fx.lib
            .add_file(MemoryLibrary::sample_file(id, "x.jpg"), None, None);
    }
    let mut inst = fx.instance_with(
        HostLimits {
            max_output_bytes: 4 * 1024 * 1024,
            ..limits()
        },
        None,
    );
    let page = fx.call(
        &mut inst,
        json!({"method": "list_files", "params": {"limit": 100000}}),
    );
    assert_eq!(page["ok"]["files"].as_array().unwrap().len(), 500);
    let cursor = page["ok"]["next_cursor"].as_str().unwrap().to_owned();
    let page2 = fx.call(
        &mut inst,
        json!({"method": "list_files", "params": {"limit": 1000, "cursor": cursor}}),
    );
    assert_eq!(page2["ok"]["files"].as_array().unwrap().len(), 102);
    assert_eq!(page2["ok"]["next_cursor"], Value::Null);
}

#[test]
fn reading_thumbnails_and_originals() {
    let fx = fixture(PermissionSet::none());
    let mut inst = fx.instance();
    let thumb = json!({"file_id": 1, "kind": "thumbnail"}).to_string();
    let orig = json!({"file_id": 1, "kind": "original"}).to_string();

    // Denied: the read traps with a JSON HostError.
    let err = fx.run(&mut inst, "read", &thumb).unwrap_err();
    assert!(
        matches!(&err, RuntimeError::Failed(m) if m.contains("permission_denied")),
        "{err}"
    );

    fx.reg
        .grant(ID, &one(Permission::Read(ReadScope::Thumbnails)))
        .unwrap();
    let out = fx.run(&mut inst, "read", &thumb).unwrap();
    assert_eq!(out.summary.as_deref(), Some("7:thumb-1"));
    // Thumbnails do not imply originals.
    assert!(fx.run(&mut inst, "read", &orig).is_err());
    fx.reg
        .grant(ID, &one(Permission::Read(ReadScope::Originals)))
        .unwrap();
    assert_eq!(
        fx.run(&mut inst, "read", &orig).unwrap().summary.as_deref(),
        Some("8:original")
    );

    // Missing file and the read size cap.
    let err = fx
        .run(
            &mut inst,
            "read",
            &json!({"file_id": 2, "kind": "thumbnail"}).to_string(),
        )
        .unwrap_err();
    assert!(
        matches!(&err, RuntimeError::Failed(m) if m.contains("not_found")),
        "{err}"
    );
    let mut inst = fx.instance_with(
        HostLimits {
            max_read_bytes: 4,
            ..limits()
        },
        None,
    );
    let err = fx.run(&mut inst, "read", &orig).unwrap_err();
    assert!(
        matches!(&err, RuntimeError::Failed(m) if m.contains("limit_exceeded")),
        "{err}"
    );
}

#[test]
fn revocation_takes_effect_on_the_next_call() {
    let fx = fixture(all());
    let mut inst = fx.instance();
    let list = json!({"method": "list_albums"});
    assert!(fx.call(&mut inst, list.clone()).get("ok").is_some());
    fx.reg
        .revoke(ID, &one(Permission::Read(ReadScope::Metadata)))
        .unwrap();
    assert!(denied(&fx.call(&mut inst, list.clone())));
    fx.reg
        .grant(ID, &one(Permission::Read(ReadScope::Metadata)))
        .unwrap();
    assert!(fx.call(&mut inst, list.clone()).get("ok").is_some());

    // Revoked *during* a running hook: the plugin reports progress, the
    // callback revokes, and the plugin's next call in the same hook fails.
    let reg = fx.reg.clone();
    let seen: Arc<Mutex<Vec<ProgressUpdate>>> = Arc::default();
    let seen2 = seen.clone();
    let progress: ProgressFn = Arc::new(move |id, update| {
        seen2.lock().unwrap().push(update.clone());
        reg.revoke(id, &one(Permission::Read(ReadScope::Metadata)))
            .unwrap();
    });
    let mut inst = fx.instance_with(limits(), Some(progress));
    let out = fx
        .run(&mut inst, "progress_then_call", &list.to_string())
        .unwrap();
    let resp: Value = serde_json::from_str(&out.summary.unwrap()).unwrap();
    assert!(denied(&resp), "{resp}");
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [ProgressUpdate {
            done: 1,
            total: Some(2),
            message: Some("half".into())
        }]
    );

    // Disabling stops hooks and host calls.
    fx.reg.disable(ID).unwrap();
    assert!(matches!(
        inst.run_on_demand("call"),
        Err(RuntimeError::Disabled)
    ));
    fx.reg.enable(ID).unwrap();
    // Removing the plugin stops a loaded instance too.
    fx.reg.remove(ID).unwrap();
    assert!(matches!(
        inst.run_on_demand("call"),
        Err(RuntimeError::NotInstalled)
    ));
}

#[test]
fn infinite_loop_is_stopped_by_the_timeout() {
    let fx = fixture_with("memory_mb = 32\ntimeout_ms = 500", PermissionSet::none());
    let mut inst = fx.instance_with(
        HostLimits {
            fuel_per_call: None,
            ..limits()
        },
        None,
    );
    let start = Instant::now();
    let err = fx.run(&mut inst, "loop", "").unwrap_err();
    assert!(matches!(err, RuntimeError::Timeout), "{err}");
    assert!(start.elapsed() < Duration::from_secs(10));
    // The instance is still usable afterwards.
    assert!(fx.run(&mut inst, "settings", "").is_ok());

    // The host maximum wins over a longer manifest timeout.
    let fx = fixture_with(
        "memory_mb = 32\ntimeout_ms = 3600000",
        PermissionSet::none(),
    );
    let mut inst = fx.instance_with(
        HostLimits {
            fuel_per_call: None,
            max_timeout: Duration::from_millis(300),
            ..limits()
        },
        None,
    );
    assert_eq!(inst.timeout(), Duration::from_millis(300));
    assert!(matches!(
        fx.run(&mut inst, "loop", ""),
        Err(RuntimeError::Timeout)
    ));
}

#[test]
fn infinite_loop_runs_out_of_fuel() {
    let fx = fixture_with("memory_mb = 32\ntimeout_ms = 60000", PermissionSet::none());
    let mut inst = fx.instance_with(
        HostLimits {
            fuel_per_call: Some(50_000_000),
            ..limits()
        },
        None,
    );
    let err = fx.run(&mut inst, "loop", "").unwrap_err();
    assert!(matches!(err, RuntimeError::OutOfFuel), "{err}");
}

#[test]
fn memory_limit_is_enforced() {
    let fx = fixture_with("memory_mb = 32\ntimeout_ms = 5000", PermissionSet::none());
    let mut inst = fx.instance();
    let ok = fx.run(&mut inst, "alloc", "8").unwrap();
    assert!(ok.summary.unwrap().starts_with("allocated 8 MiB"));
    let err = fx.run(&mut inst, "alloc", "64").unwrap_err();
    assert!(matches!(err, RuntimeError::Failed(_)), "{err}");

    // The host maximum clamps a generous manifest.
    let fx = fixture_with("memory_mb = 4096\ntimeout_ms = 5000", PermissionSet::none());
    let mut inst = fx.instance_with(
        HostLimits {
            max_memory_mb: 4,
            ..limits()
        },
        None,
    );
    assert!(fx.run(&mut inst, "alloc", "8").is_err());
}

/// A one-shot HTTP server on 127.0.0.1 answering `body`.
fn serve_once(body: &'static str) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (port, h)
}

#[test]
fn http_only_to_granted_hosts() {
    let fx = fixture(all());
    let mut inst = fx.instance();

    // A host the manifest does not list.
    let err = fx
        .run(&mut inst, "http", "http://unlisted.example.net/")
        .unwrap_err();
    assert!(
        matches!(&err, RuntimeError::Failed(m) if m.contains("not allowed")),
        "{err}"
    );
    // Not a URL / not http(s).
    assert!(fx.run(&mut inst, "http", "file:///etc/passwd").is_err());

    // A granted host works (local server, no internet needed).
    let (port, h) = serve_once("hello");
    let out = fx
        .run(&mut inst, "http", &format!("http://127.0.0.1:{port}/x"))
        .unwrap();
    assert_eq!(out.summary.as_deref(), Some("200:hello"));
    h.join().unwrap();

    // Revoked on the running instance: refused without connecting.
    fx.reg
        .revoke(ID, &one(Permission::Network("127.0.0.1".into())))
        .unwrap();
    let (port, _h) = serve_once("hello");
    let err = fx
        .run(&mut inst, "http", &format!("http://127.0.0.1:{port}/x"))
        .unwrap_err();
    assert!(
        matches!(&err, RuntimeError::Failed(m) if m.contains("not allowed")),
        "{err}"
    );
    // Unblock the server thread.
    let _ = std::net::TcpStream::connect(("127.0.0.1", port));

    // Requested but never granted.
    let fx = fixture(PermissionSet::none());
    let mut inst = fx.instance();
    let err = fx
        .run(&mut inst, "http", "http://allowed.example.com/")
        .unwrap_err();
    assert!(
        matches!(&err, RuntimeError::Failed(m) if m.contains("not allowed")),
        "{err}"
    );
}

#[test]
fn export_paths_cannot_escape() {
    let fx = fixture(all());
    let root = fx.dir.path().join("export");
    let outside = fx.dir.path().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    fx.reg.set_export_root(ID, Some(root.clone())).unwrap();
    let mut inst = fx.instance();
    let export = |inst: &mut PluginInstance, path: &str, overwrite: bool| {
        fx.call(
            inst,
            json!({"method": "export_file", "params": {"file_id": 1, "path": path, "overwrite": overwrite}}),
        )
    };

    for bad in [
        "../escape.jpg",
        "a/../../escape.jpg",
        "/tmp/escape.jpg",
        "a\\..\\escape.jpg",
        "a//b.jpg",
        "./a.jpg",
        "nul\0.jpg",
        "",
    ] {
        let r = export(&mut inst, bad, false);
        assert_eq!(r["err"]["code"], "invalid_request", "{bad:?}: {r}");
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        let r = export(&mut inst, "link/escape.jpg", false);
        assert_eq!(r["err"]["code"], "invalid_request", "{r}");
        std::os::unix::fs::symlink(outside.join("target.jpg"), root.join("file-link.jpg")).unwrap();
        let r = export(&mut inst, "file-link.jpg", true);
        assert_eq!(r["err"]["code"], "invalid_request", "{r}");
    }
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);

    // A good path creates parents and writes the original.
    let r = export(&mut inst, "Albums/Mine/IMG_0001.HEIC", false);
    assert_eq!(r["ok"], json!({"written": true, "bytes": 8}), "{r}");
    let target = root.join("Albums/Mine/IMG_0001.HEIC");
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    // Existing file without overwrite is left alone.
    std::fs::write(&target, b"changed").unwrap();
    let r = export(&mut inst, "Albums/Mine/IMG_0001.HEIC", false);
    assert_eq!(r["ok"], json!({"written": false, "bytes": 0}));
    assert_eq!(std::fs::read(&target).unwrap(), b"changed");
    let r = export(&mut inst, "Albums/Mine/IMG_0001.HEIC", true);
    assert_eq!(r["ok"]["written"], true);
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    // A file without original bytes fails cleanly and leaves nothing behind.
    let r = fx.call(
        &mut inst,
        json!({"method": "export_file", "params": {"file_id": 2, "path": "two.jpg"}}),
    );
    assert_eq!(r["err"]["code"], "not_found", "{r}");
    assert!(!root.join("two.jpg").exists());
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        if cfg!(unix) { 3 } else { 1 }
    );
}

#[test]
fn kv_persists_across_reloads() {
    let fx = fixture(PermissionSet::none());
    let mut inst = fx.instance();
    let r = fx.call(
        &mut inst,
        json!({"method": "kv_set", "params": {"key": "cursor", "value": {"n": 42}}}),
    );
    assert!(r.get("ok").is_some(), "{r}");
    drop(inst);

    let mut inst = fx.instance();
    let r = fx.call(
        &mut inst,
        json!({"method": "kv_get", "params": {"key": "cursor"}}),
    );
    assert_eq!(r["ok"], json!({"n": 42}));
    drop(inst);

    // And across reopening the registry.
    let reg = Arc::new(Registry::open(fx.dir.path().join("registry")).unwrap());
    let mut inst = reg
        .instantiate(ID, fx.lib.clone(), limits(), Runner::Test, None)
        .unwrap();
    let mut v = serde_json::Map::new();
    v.insert(
        "arg".into(),
        json!(json!({"method": "kv_get", "params": {"key": "cursor"}}).to_string()),
    );
    reg.set_settings(ID, v).unwrap();
    let out = inst.run_on_demand("call").unwrap();
    let r: Value = serde_json::from_str(&out.summary.unwrap()).unwrap();
    assert_eq!(r["ok"], json!({"n": 42}));

    // The size cap.
    let mut inst = fx.instance_with(
        HostLimits {
            max_kv_bytes: 100,
            ..limits()
        },
        None,
    );
    let r = fx.call(
        &mut inst,
        json!({"method": "kv_set", "params": {"key": "big", "value": "x".repeat(200)}}),
    );
    assert_eq!(r["err"]["code"], "limit_exceeded");
    let r = fx.call(
        &mut inst,
        json!({"method": "kv_get", "params": {"key": "k".repeat(129)}}),
    );
    assert_eq!(r["err"]["code"], "invalid_request");
}

#[test]
fn hook_inputs() {
    let fx = fixture(PermissionSet::none());
    let mut inst = fx.instance();

    // Settings: defaults filled in.
    let out = fx.run(&mut inst, "settings", "hello").unwrap();
    let settings: Value = serde_json::from_str(&out.summary.unwrap()).unwrap();
    assert_eq!(settings, json!({"arg": "hello", "flag": true}));

    // Upload: metadata only with read:metadata.
    let out = inst.run_upload(1).unwrap();
    let input: Value = serde_json::from_str(&out.summary.unwrap()).unwrap();
    assert_eq!(input["event"]["type"], "upload");
    assert_eq!(input["event"]["file_id"], 1);
    assert_eq!(input["event"]["file"], Value::Null);
    assert_eq!(input["runner"], "test");
    assert_eq!(out.changed, 1);
    fx.reg
        .grant(ID, &one(Permission::Read(ReadScope::Metadata)))
        .unwrap();
    let out = inst.run_upload(1).unwrap();
    let input: Value = serde_json::from_str(&out.summary.unwrap()).unwrap();
    assert_eq!(input["event"]["file"]["title"], "IMG_0001.HEIC");

    // Schedule: last_run from the registry.
    fx.reg.record_run(ID, 1234).unwrap();
    let out = inst.run_schedule().unwrap();
    let input: Value = serde_json::from_str(&out.summary.unwrap()).unwrap();
    assert_eq!(
        input["event"],
        json!({"type": "schedule", "last_run": 1234})
    );

    // Unknown actions and invalid output.
    assert!(matches!(
        inst.run_on_demand("nope"),
        Err(RuntimeError::UnknownAction(_))
    ));
    assert!(matches!(
        inst.run_on_demand("bad_output"),
        Err(RuntimeError::BadOutput(_))
    ));
}

#[test]
fn undeclared_hooks_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Arc::new(Registry::open(dir.path()).unwrap());
    let key = SigningKey::generate();
    // Same module, but the manifest declares only one on-demand action.
    let m = common::manifest(
        "dev.photovault.narrow",
        "1.0.0",
        "[hooks]\non_demand = [{ id = \"settings\", label = \"Go\" }]",
    );
    reg.install(&pack(&m, probe_wasm(), &key), PermissionSet::none())
        .unwrap();
    let mut inst = reg
        .instantiate(
            "dev.photovault.narrow",
            Arc::new(MemoryLibrary::new()),
            limits(),
            Runner::Test,
            None,
        )
        .unwrap();
    assert!(matches!(
        inst.run_upload(1),
        Err(RuntimeError::HookNotDeclared("on_upload"))
    ));
    assert!(matches!(
        inst.run_schedule(),
        Err(RuntimeError::HookNotDeclared("on_schedule"))
    ));
    assert!(matches!(
        inst.run_on_demand("loop"),
        Err(RuntimeError::UnknownAction(_))
    ));
    assert!(inst.run_on_demand("settings").is_ok());

    // Declared in the manifest but not exported by the module.
    let m = common::manifest(
        "dev.photovault.liar",
        "1.0.0",
        "[hooks]\non_demand = [{ id = \"x\", label = \"X\" }]",
    );
    let empty = b"\0asm\x01\0\0\0";
    reg.install(&pack(&m, empty, &key), PermissionSet::none())
        .unwrap();
    let mut inst = reg
        .instantiate(
            "dev.photovault.liar",
            Arc::new(MemoryLibrary::new()),
            limits(),
            Runner::Test,
            None,
        )
        .unwrap();
    assert!(matches!(
        inst.run_on_demand("x"),
        Err(RuntimeError::HookNotExported("on_demand"))
    ));
}

#[test]
fn dispatch_isolates_failures_and_records_runs() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Arc::new(Registry::open(dir.path()).unwrap());
    let key = SigningKey::generate();
    let good = "dev.photovault.good";
    let broken = "dev.photovault.broken";
    let disabled = "dev.photovault.disabled";
    reg.install(
        &pack(
            &probe_manifest(good, "1.0.0", "memory_mb = 32"),
            probe_wasm(),
            &key,
        ),
        PermissionSet::none(),
    )
    .unwrap();
    // 1 MiB is less than the module's initial memory: it cannot even load.
    reg.install(
        &pack(
            &probe_manifest(broken, "1.0.0", "memory_mb = 1"),
            probe_wasm(),
            &key,
        ),
        PermissionSet::none(),
    )
    .unwrap();
    reg.install(
        &pack(
            &probe_manifest(disabled, "1.0.0", "memory_mb = 32"),
            probe_wasm(),
            &key,
        ),
        PermissionSet::none(),
    )
    .unwrap();
    reg.disable(disabled).unwrap();

    let d = Dispatcher {
        registry: reg.clone(),
        library: library(),
        limits: limits(),
        runner: Runner::Test,
        progress: None,
    };
    let runs = d.dispatch_upload(1);
    let ids: Vec<&str> = runs.iter().map(|r| r.plugin_id.as_str()).collect();
    assert_eq!(ids, [broken, good]);
    assert!(runs[0].result.is_err());
    assert!(runs[1].result.is_ok());

    // Scheduled: all enabled plugins are due at first; only successes record.
    let now = 10_000_000_000;
    let runs = d.run_due(now);
    assert_eq!(runs.len(), 2);
    assert_eq!(reg.get(good).unwrap().last_run, Some(now));
    assert_eq!(reg.get(broken).unwrap().last_run, None);
    // Ten minutes later only the broken one is due again (every 15 min).
    let runs = d.run_due(now + 10 * 60_000_000);
    assert_eq!(
        runs.iter()
            .map(|r| r.plugin_id.as_str())
            .collect::<Vec<_>>(),
        [broken]
    );
    let runs = d.run_due(now + 15 * 60_000_000);
    assert_eq!(runs.len(), 2);

    let run = d.run_on_demand(good, "settings");
    assert!(run.result.is_ok());
    let _ = &key;
}

#[test]
fn update_keeps_kv_and_unchanged_key_works_at_runtime() {
    let fx = fixture(PermissionSet::none());
    let mut inst = fx.instance();
    fx.call(
        &mut inst,
        json!({"method": "kv_set", "params": {"key": "a", "value": 1}}),
    );
    drop(inst);
    let pkg2 = pack(
        &probe_manifest(ID, "1.0.1", "memory_mb = 32"),
        probe_wasm(),
        &fx.key,
    );
    fx.reg.update(&pkg2).unwrap();
    let mut inst = fx.instance();
    let r = fx.call(
        &mut inst,
        json!({"method": "kv_get", "params": {"key": "a"}}),
    );
    assert_eq!(r["ok"], 1);
}
