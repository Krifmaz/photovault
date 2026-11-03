# photovault-plugin-pdk

The guest SDK for **Photovault plugins**: small WebAssembly modules that
automate your photo library. Plugins run in [Extism](https://extism.org) on
your own devices (phone, desktop, web app) or on your compute node, **never on
the server**, which only ever holds ciphertext. A plugin sees decrypted
metadata (and, only if you allow it, thumbnails or originals) through a small
host API, and every call is checked against the permissions you approved when
you installed it.

Photovault is based on Ente (ente.com). Licence: AGPL-3.0-only.

## What is in the box

| Module / item              | What it does                                                                                                      |
|----------------------------|-------------------------------------------------------------------------------------------------------------------|
| `host`                     | Typed host calls: `list_files`, `files` (walks every page), `get_file`, `list_albums`, `add_tags`, `ensure_album`, `add_to_album`, `remove_from_album`, `trash_files`, `export_file`, `kv_get` / `kv_set` / `kv_delete`, `progress`, `read_thumbnail`, `read_original`. All return `Result<_, HostError>`. |
| `photovault_plugin!`       | Exports your hook functions as `on_upload`, `on_schedule`, `on_demand`.                                           |
| `HookInputExt`             | `input.settings_as::<MySettings>()`, `input.action()`, `input.uploaded_file()`.                                    |
| `info!`, `warn!`, ...      | Logging to the plugin log on the device.                                                                          |
| `http`                     | Outbound HTTP for plugins granted `network` hosts (thin layer over Extism's `http_request`).                       |
| `path`                     | `sanitize_component` and friends for safe export paths.                                                           |
| `time`                     | UTC dates from Ente's microsecond timestamps.                                                                     |
| `testing` (native only)    | `MockHost`, input builders and `run_hook`, so plugin logic is tested with plain `cargo test`.                      |
| `abi`, `settings`, `Manifest` | The wire types and manifest schema, re-exported from `photovault-plugin-manifest`.                            |

## Write your first plugin

We will write a plugin that tags every uploaded video with `video`.

### 1. Create the crate

```sh
cargo new --lib tag-videos
```

`Cargo.toml`:

```toml
[package]
name = "tag-videos"
version = "0.1.0"
edition = "2024"
license = "AGPL-3.0-only"

[lib]
crate-type = ["cdylib", "rlib"]   # cdylib = the .wasm, rlib = native tests

[dependencies]
photovault-plugin-pdk = { path = "../photovault/plugins/sdk/guest/pdk" }
serde = { version = "1", features = ["derive"] }

[dev-dependencies]
serde_json = "1"

[profile.release]
opt-level = "z"
lto = true
strip = true
panic = "abort"
```

`.cargo/config.toml`, so `cargo build` targets Wasm by default:

```toml
[build]
target = "wasm32-unknown-unknown"
```

(`rustup target add wasm32-unknown-unknown` once.)

### 2. Write the manifest

`plugin.toml` declares who you are, what you need and which hooks you
implement. The user sees the permissions on install and can revoke them later.

```toml
[plugin]
id = "com.example.tag-videos"
name = "Tag videos"
version = "0.1.0"
description = "Adds the tag `video` (or one you choose) to every uploaded video."
api_version = 1

[permissions]
read = ["metadata"]
write = ["tags"]

[hooks]
on_upload = true

[[settings]]
key = "tag"
label = "Tag"
type = "text"
default = "video"
```

Settings are declarative: the app renders the page with its own widgets
(`toggle`, `text`, `number`, `select`, `album`) and passes the values, with
defaults filled in, to every hook.

### 3. Write the hook

`src/lib.rs`:

```rust
use photovault_plugin_pdk::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Settings {
    tag: String,
}

/// Called once for each uploaded file.
pub fn handle_upload(input: &HookInput) -> Result<HookOutput> {
    let settings: Settings = input.settings_as()?;
    let Some(file) = input.uploaded_file()? else {
        return Ok(HookOutput::default());
    };
    if file.file_type != FileType::Video || file.tags.contains(&settings.tag) {
        return Ok(HookOutput::default());
    }
    pdk::host::add_tags(file.id, &[settings.tag.as_str()])?;
    pdk::info!("tagged {}", file.title);
    Ok(HookOutput {
        summary: Some(format!("Tagged {}", file.title)),
        changed: 1,
    })
}

photovault_plugin_pdk::photovault_plugin! {
    on_upload: handle_upload,
}
```

A hook is a plain function `fn(&HookInput) -> Result<HookOutput>`. Any error
(`?`, `bail!`, `.context(..)`) ends the run and is shown in the plugin's
activity log; host errors carry a machine-readable `ErrorCode` such as
`PermissionDenied`.

### 4. Test it natively

All host calls go through a backend. In Wasm that is the real Extism host; in
native builds it is whatever you install on the test thread, normally
`testing::MockHost` (in-memory files, albums and KV; enforces permissions;
records writes):

```rust
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use photovault_plugin_pdk::testing::*;

    const MANIFEST: &str = include_str!("../plugin.toml");

    #[test]
    fn tags_videos_only() {
        let host = MockHost::new().install();
        host.grant_manifest(MANIFEST); // exactly the manifest's permissions
        let video = FileInfo { file_type: FileType::Video, ..file(1, "clip.mp4") };
        host.add_file(video.clone());

        let input = on_upload(&video)
            .with_settings(settings_from_manifest(MANIFEST, serde_json::json!({})));
        let out = run_hook(handle_upload, &input).unwrap();

        assert_eq!(out.changed, 1);
        assert_eq!(host.file(1).unwrap().tags, ["video"]);
    }
}
```

```sh
cargo test --target x86_64-unknown-linux-gnu   # or your host triple
```

`run_hook` sends the input and output through JSON exactly as the Wasm export
does, `settings_from_manifest` builds the settings the app would pass
(validating your overrides against `plugin.toml`), and the mock answers with the
real wire format, so what passes here behaves the same in the host.

Useful `MockHost` knobs: `deny(permission)`, `set_page_size(n)` (to exercise
pagination), `fail_on("method", error)`, `set_kv_limit(bytes)`,
`route(url, response)` for HTTP, and inspection with `writes()`, `album_files()`,
`trashed()`, `exports()`, `kv()`, `progress()`, `logs()`.

### 5. Build

```sh
cargo build --release
ls target/wasm32-unknown-unknown/release/tag_videos.wasm
```

Install the `.wasm` together with `plugin.toml` from the app's plugin settings
(or the `photovault-plugin` CLI). Released plugins are signed; the app shows
the signer and the permissions before you approve.

## Rules of the sandbox

* No file system, no clock other than `HookInput::now`, no threads.
* Network only to the hosts listed under `[permissions] network`, through
  `pdk::http`.
* Memory and time limits from `[limits]`, clamped by the host.
* Exports go through `host::export_file`: the host copies the decrypted file
  into the folder the user picked; the plugin never sees bytes or real paths.
* Keep state in `kv_set` (per plugin, on the device, size-capped).
* When a long run changes the library (trash, remove from album), collect ids
  first and act after walking the pages, so the pages do not shift under you.

## Other languages

Anything that compiles to WebAssembly and has an
[Extism PDK](https://extism.org/docs/concepts/pdk) (Go/TinyGo, JavaScript,
C, Zig, AssemblyScript, Haskell, .NET, ...) can implement the same ABI:

* export `on_upload`, `on_schedule` and/or `on_demand`, each reading a JSON
  `HookInput` as Extism input and writing a JSON `HookOutput` as output (set an
  Extism error and return non-zero to fail);
* import `photovault_call` (JSON `Request` → JSON `Response`) and
  `photovault_read` (JSON `ReadRequest` → raw bytes) from the
  `extism:host/user` namespace.

The complete wire format, with examples, is documented in the
[`abi` module](../../manifest/src/abi.rs) of `photovault-plugin-manifest`
(`cargo doc -p photovault-plugin-manifest --open`).

## Examples

See [`../examples`](../examples): `auto-albums` (rules → albums),
`screenshot-cleanup` (collect or trash old screenshots) and `export-albums`
(incremental export to a folder).
