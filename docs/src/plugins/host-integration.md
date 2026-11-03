# Running plugins in an app or the compute node

Apps and the compute node embed `photovault-plugin-host`. They provide one
trait, `Library`, over the library they have **already decrypted locally**;
the host crate does everything else: signature checks, the sandbox,
permission checks on every call, settings, key/value storage, exports and
scheduling.

```rust,ignore
use photovault_plugin_host::{Dispatcher, HostLimits, Registry, manifest::abi::Runner};
use std::sync::Arc;

let registry = Arc::new(Registry::open(data_dir.join("plugins"))?);

// Install screen: show request.manifest, request.fingerprint and
// request.permissions (with descriptions and a "sensitive" flag).
let request = registry.inspect(&bytes)?;
registry.install(&bytes, approved_permissions)?;

// Settings → Plugins
registry.list();                          // enabled, grants, settings, versions
registry.set_enabled(id, false)?;
registry.revoke(id, &perms)?;             // takes effect on the next host call
registry.set_settings(id, values)?;       // validated against the manifest
registry.set_export_root(id, Some(dir))?; // export-albums destination
let report = registry.update(&new_bytes)?; // report.new_permissions need approval
registry.remove(id)?;

// Hooks
let dispatcher = Dispatcher {
    registry: registry.clone(),
    library,                 // Arc<dyn Library>
    limits: HostLimits::default(),
    runner: Runner::Desktop,
    progress: None,
};
dispatcher.dispatch_upload(file_id);        // after each upload
dispatcher.run_due(runtime::now_micros());  // from the app's background timer
dispatcher.run_on_demand(id, "apply_all");  // a button on the plugin page
```

## Per platform

| Runner | How it embeds the host |
| --- | --- |
| Compute node | Directly (Rust) |
| Desktop (Electron) | Native Node module over the host crate |
| Mobile (Flutter) | `flutter_rust_bridge` over the host crate |
| Web | Not yet: needs the Extism JS SDK and a TypeScript port of the permission checks |

The app integrations are small, marked patches in the app forks and land with
the forks (section 4 work); the host crate is ready for them.

## Limits

`HostLimits::default()`: 256 MiB memory, 60 s per hook call, a fuel budget,
500 files per page, 256 MiB per read, 1 MiB key/value store per plugin, 16 MiB
HTTP responses. A plugin's own `[limits]` are clamped to these.
