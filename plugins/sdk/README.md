# plugins/sdk

Photovault's WebAssembly plugin SDK. Plugins run in an [Extism](https://extism.org)
sandbox on the user's devices or compute node, **never on the server**.

| Path | Crate | What |
| --- | --- | --- |
| [`manifest/`](manifest) | `photovault-plugin-manifest` | `plugin.toml` schema, permissions, settings fields, host/guest ABI types |
| [`host/`](host) | `photovault-plugin-host` | Runtime for apps and the compute node: sandbox, permission checks, signed packages, registry, scheduler |
| [`cli/`](cli) | `photovault-plugin` | `keygen`, `pack`, `verify`, `inspect` |
| [`guest/pdk/`](guest/pdk) | `photovault-plugin-pdk` | What plugin authors use (Rust) |
| [`guest/examples/`](guest/examples) | | `auto-albums`, `screenshot-cleanup`, `export-albums` |

`guest/` is its own Cargo workspace that builds for `wasm32-unknown-unknown`.

```sh
# Host side (from the repo root)
cargo test -p photovault-plugin-manifest -p photovault-plugin-host -p photovault-plugin

# Guest side
cd plugins/sdk/guest
cargo build --release                                   # the .wasm files
cargo test --target x86_64-unknown-linux-gnu            # plugin logic, natively
```

`host/tests/examples.rs` builds the three examples, signs and installs them, and
runs them in the real sandbox.

Start with the [PDK walkthrough](guest/pdk/README.md). Full docs:
`docs/src/plugins/`.

Based on Ente (ente.com). AGPL-3.0-only.
