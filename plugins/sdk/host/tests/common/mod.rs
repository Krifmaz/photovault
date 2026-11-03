//! Shared helpers for the integration tests.

#![allow(dead_code)]

use photovault_plugin_host::{Package, SigningKey};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// A minimal valid Wasm module (header only), for tests that never run it.
pub const EMPTY_WASM: &[u8] = b"\0asm\x01\0\0\0";

/// A manifest for `id` at `version` with the given extra TOML.
pub fn manifest(id: &str, version: &str, extra: &str) -> String {
    format!(
        "[plugin]\nid = \"{id}\"\nname = \"Test plugin\"\nversion = \"{version}\"\napi_version = 1\n\n{extra}\n"
    )
}

/// Build a signed package.
pub fn pack(manifest: &str, wasm: &[u8], key: &SigningKey) -> Vec<u8> {
    Package::build(manifest.as_bytes(), wasm, key).expect("package builds")
}

/// The probe fixture, built once per test binary for wasm32-unknown-unknown
/// with a fixed `cargo build` command line (no shell).
pub fn probe_wasm() -> &'static [u8] {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    WASM.get_or_init(|| {
        let manifest =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/probe/Cargo.toml");
        let target_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("probe");
        let status = Command::new(env!("CARGO"))
            .arg("build")
            .arg("--release")
            .arg("--locked")
            .arg("--target")
            .arg("wasm32-unknown-unknown")
            .arg("--manifest-path")
            .arg(&manifest)
            .arg("--target-dir")
            .arg(&target_dir)
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .status()
            .expect("run cargo");
        assert!(status.success(), "building the probe fixture failed");
        std::fs::read(
            target_dir.join("wasm32-unknown-unknown/release/photovault_probe_plugin.wasm"),
        )
        .expect("read probe wasm")
    })
}

/// The probe's manifest: every permission, every hook, one text setting.
pub fn probe_manifest(id: &str, version: &str, limits: &str) -> String {
    manifest(
        id,
        version,
        &format!(
            r#"
[permissions]
read = ["metadata", "thumbnails", "originals"]
write = ["tags", "albums", "trash", "export"]
network = ["127.0.0.1", "allowed.example.com"]

[hooks]
on_upload = true
schedule = {{ every_minutes = 15 }}
on_demand = [
  {{ id = "call", label = "Call" }},
  {{ id = "progress_then_call", label = "Progress then call" }},
  {{ id = "read", label = "Read" }},
  {{ id = "loop", label = "Loop" }},
  {{ id = "alloc", label = "Alloc" }},
  {{ id = "http", label = "HTTP" }},
  {{ id = "settings", label = "Settings" }},
  {{ id = "bad_output", label = "Bad output" }},
]

[limits]
{limits}

[[settings]]
key = "arg"
label = "Argument"
type = "text"
max_length = 4096

[[settings]]
key = "flag"
label = "Flag"
type = "toggle"
default = true
"#
        ),
    )
}
