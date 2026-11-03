# photovault-plugin-host

The host runtime for Photovault's WebAssembly plugins. The apps (desktop,
mobile through FFI) and the compute node embed it to install, verify and run
plugins **on the user's own devices**; plugins never run on the server, and
they only ever see what the device already holds decrypted.

Photovault is based on Ente (ente.com). AGPL-3.0-only.

## What it does

| Module      | Purpose |
|-------------|---------|
| `package`   | Signed `.pvplugin` container, Ed25519 keys (libsodium), fingerprints |
| `library`   | `Library` trait the embedder implements; `MemoryLibrary` (feature `testing`) |
| `runtime`   | Sandboxed Extism instance, host functions, permission checks, hooks |
| `registry`  | On-device plugin store: install, update, grants, settings, TOFU key pins |
| `scheduler` | Which schedules are due; failure-isolated upload and schedule dispatch |

The manifest, permission model and ABI live in
[`photovault-plugin-manifest`](../manifest) (re-exported as `manifest`).

## Package format

```text
"PVPLUG01"
u32 LE len | plugin.toml bytes   (<= 64 KiB)
u32 LE len | wasm bytes          (<= 64 MiB)
u32 LE 32  | Ed25519 public key
u32 LE 64  | Ed25519 signature
```

Nothing may follow. The signature is `crypto_sign_detached` over
`b"photovault-plugin-v1\0" || BLAKE2b-256(manifest) || BLAKE2b-256(wasm)`.
Parsing rejects bad magic, truncation, oversized or wrongly sized sections,
trailing bytes, invalid manifests, non-Wasm modules and bad signatures.
Fingerprints are the first 16 bytes of BLAKE2b-256(public key) as eight
groups of four hex digits.

## Sandbox

* Memory: manifest `limits.memory_mb`, clamped to `HostLimits::max_memory_mb`
  (default 256 MiB).
* Time: manifest `limits.timeout_ms` per hook call, clamped to
  `HostLimits::max_timeout` (default 60 s); plus an optional fuel budget
  (`fuel_per_call`).
* No file system: WASI is on but without preopened directories, environment or
  arguments, and Extism `allowed_paths` is never set.
* Network: the runtime replaces Extism's `http_request` with its own, which
  checks the **current** `network` grants on every request (exact host or
  `*.suffix`), refuses non-HTTP(S) URLs, follows no redirects and caps the
  response size. Extism's `allowed_hosts` is also set to the grants at load
  time as a second line of defence.
* Caps: page size (500), read size, request size, batch size, per-plugin KV
  size, Extism variable size, hook output size. See `HostLimits`.

## Permission checks

Every `photovault_call` and `photovault_read` asks the `PolicySource` (the
`Registry`) for the plugin's policy at that moment: a revoked permission, a
disabled plugin or a removed plugin takes effect on the next call, including
inside a hook that is already running. Grants are intersected with what the
manifest requests. `ExportFile` needs `write:export` only: the host copies the
original into the user-chosen export folder, after strict path sanitisation
(no `..`, `.`, absolute paths, empty components, NUL, backslash, colon or
control characters; symlinks are never followed), so the plugin never gets
the bytes or the real path.

## Example

```rust,ignore
use photovault_plugin_host::{Registry, HostLimits, manifest::abi::Runner};
use std::sync::Arc;

let registry = Arc::new(Registry::open(data_dir.join("plugins"))?);

// Install screen.
let request = registry.inspect(&bytes)?;          // manifest, fingerprint, permissions
registry.install(&bytes, approved_permissions)?;   // pins the publisher key

// Run a button.
let mut plugin = registry.instantiate(
    "dev.photovault.auto-albums",
    library.clone(),                // your impl of Library
    HostLimits::default(),
    Runner::Desktop,
    None,                           // progress callback
)?;
let output = plugin.run_on_demand("run")?;

// After each upload, and periodically.
let dispatcher = photovault_plugin_host::Dispatcher {
    registry: registry.clone(),
    library,
    limits: HostLimits::default(),
    runner: Runner::Desktop,
    progress: None,
};
dispatcher.dispatch_upload(file_id);
dispatcher.run_due(photovault_plugin_host::runtime::now_micros());
```

## Tests

```sh
cargo test -p photovault-plugin-host
```

The runtime tests build the probe plugin in `tests/fixtures/probe/` (its own
workspace, `wasm32-unknown-unknown`, needs `rustup target add
wasm32-unknown-unknown`) and check permission denial and grant for every host
method, revocation on a running instance, timeouts, fuel, memory limits, HTTP
host restrictions (against a local server, no internet needed), export path
escapes and KV persistence.

## Build dependencies

No system C libraries beyond libsodium. Bundled C/assembly built from source
by dependencies: `ring` (TLS for plugin HTTP, via `rustls`), `zstd-sys`
(Wasmtime's compilation cache), Wasmtime's small platform helpers and fiber
assembly, and libsodium itself (`libsodium-sys-stable`).
