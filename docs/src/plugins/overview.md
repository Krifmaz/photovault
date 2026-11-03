# Plugins

Photovault plugins are small WebAssembly programs that automate things in your
library: sorting photos into albums, cleaning up screenshots, exporting albums
to a folder. They are optional and off until you install one.

## Where plugins run

Plugins run **only on your own devices**: the phone or desktop app (web later), or
your [compute node](../architecture.md). They never run on the server. The
server only holds ciphertext, and nothing about plugins changes that: a plugin
sees decrypted data only because it runs inside an app that already has your
keys, and only the parts you allowed.

## How it fits together

| Piece | Crate / path | Role |
| --- | --- | --- |
| Manifest and ABI | `plugins/sdk/manifest` | `plugin.toml` schema, permissions, settings fields, host/guest wire types |
| Guest SDK | `plugins/sdk/guest/pdk` | What plugin authors use (Rust) |
| Host runtime | `plugins/sdk/host` | Runs plugins in an [Extism](https://extism.org) sandbox, checks permissions, keeps the installed-plugin registry |
| CLI | `plugins/sdk/cli` | `photovault-plugin keygen / pack / verify / inspect` |
| Examples | `plugins/sdk/guest/examples` | Auto albums, screenshot cleanup, export albums |

## Lifecycle

1. **Install.** The app reads a signed `.pvplugin` file, checks the signature,
   and shows the plugin's name, publisher fingerprint and the permissions it
   asks for. You approve all or some of them.
2. **Run.** Hooks fire on upload, on a schedule, or when you press one of the
   plugin's buttons. Each run is sandboxed with memory and time limits.
3. **Manage.** Settings → Plugins lists every plugin with enable/disable,
   update, remove, its settings page and its permissions, any of which you can
   revoke at any time.
4. **Update.** An update must come from the same publisher key and have a
   higher version. New permissions are never granted automatically.

See also: [Manifest](manifest.md), [Permissions and sandbox](security.md),
[Host/guest ABI](abi.md).
