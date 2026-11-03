# Plugin guest workspace

Everything that is compiled **into** a Photovault plugin (`.wasm`):

* [`pdk/`](pdk): `photovault-plugin-pdk`, the Rust SDK for writing plugins,
  with a "write your first plugin" walkthrough.
* [`examples/`](examples): three complete plugins, each with `plugin.toml`,
  logic in plain functions, a README and native unit tests:
  [`auto-albums`](examples/auto-albums),
  [`screenshot-cleanup`](examples/screenshot-cleanup),
  [`export-albums`](examples/export-albums).

The contract shared with the host (manifest schema, permissions, settings,
ABI) is the `photovault-plugin-manifest` crate in [`../manifest`](../manifest).

This is a separate Cargo workspace (excluded from the root one) that builds
for `wasm32-unknown-unknown` by default:

```sh
cd plugins/sdk/guest
cargo build --release                                   # target/wasm32-unknown-unknown/release/*.wasm
cargo test --target x86_64-unknown-linux-gnu            # native unit tests with MockHost
cargo clippy --all-targets -- -D warnings               # wasm32
cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
```

Photovault is based on Ente (ente.com). Licence: AGPL-3.0-only.
