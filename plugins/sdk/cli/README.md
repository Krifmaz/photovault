# photovault-plugin

Command-line tool for Photovault plugin authors: create a signing key, pack a
manifest and module into a signed `.pvplugin`, and verify or inspect packages.

Photovault is based on Ente (ente.com). AGPL-3.0-only.

```sh
cargo install --path plugins/sdk/cli

# Once: create your publisher key. Keep it private and backed up; users'
# devices pin it on first install and refuse updates signed by another key.
photovault-plugin keygen --out ~/.config/photovault/plugin.key

# Build your plugin for Wasm, then sign it.
cargo build --release --target wasm32-unknown-unknown
photovault-plugin pack \
  --manifest plugin.toml \
  --wasm target/wasm32-unknown-unknown/release/my_plugin.wasm \
  --key ~/.config/photovault/plugin.key \
  --out my-plugin.pvplugin

# Check it, and see what users will be asked to approve.
photovault-plugin verify my-plugin.pvplugin
photovault-plugin inspect my-plugin.pvplugin
```

| Command   | What it does |
|-----------|--------------|
| `keygen --out FILE` | Writes a new Ed25519 secret key (mode 0600, never overwrites) and prints the public key and fingerprint. |
| `pack --manifest --wasm --key --out` | Validates `plugin.toml`, checks the module is Wasm, signs both and writes the package. |
| `verify PKG` | Checks format, manifest and signature; exits non-zero on any problem. |
| `inspect PKG` | Verifies, then prints the manifest summary, hooks, limits, fingerprint and each permission with its description (`!` marks sensitive ones). |

The package format is described in [`../host/README.md`](../host/README.md).
The tool never runs anything through a shell.
