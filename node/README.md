# photovault-node

The optional Photovault compute node. **Based on Ente (ente.com).**

It runs on a machine you own (for example a desktop with a GPU), signs in to
your account like any other device, works through your whole library and
uploads the results **in Ente's own encrypted formats**. Stock Ente apps and
the Photovault forks pick the results up automatically, exactly as if another
of your devices had done the work. Without a node, everything works the normal
Ente way: the apps index on their own.

| Result | Ente file-data type | Format |
| --- | --- | --- |
| Faces + search (CLIP) | `mldata` | gzipped JSON `{face, clip, …}`, encrypted with the file key (libsodium secretstream blob) |
| Streamable video | `vid_preview` | AES-128 HLS stream (one-time key) + gzipped playlist JSON `{type: "hls_video", …}` encrypted with the file key |

The server never sees plaintext: all decryption, inference and transcoding
happen on the node.

## How it works

1. `photovault-node login` signs in with SRP (email code and TOTP 2FA
   supported, passkey-only accounts are not), decrypts your master key on the
   node and stores the session at `<data_dir>/session.json` (mode 0600). The
   node shows up in the app's list of active sessions, where you can revoke it.
2. `run` syncs the library, skips files that already have current indexes
   (same rule as the apps: remote `version >= 1` is kept), downloads and
   decrypts each remaining file, indexes it, merges the result into the
   existing `mldata` (unknown keys from newer clients are preserved) and
   uploads it. Videos also get a streamable HLS preview.
3. Progress is saved per file version in `<data_dir>/state.json`, so changed
   files are redone and a restart picks up where it left off.

### Pausing while you use the machine

The node pauses when any of these hold, with hysteresis so it does not flap:
CPU load above a threshold, GPU load above a threshold (NVIDIA via
`nvidia-smi`, AMD via sysfs, Intel estimated), a game or launcher is running
(Steam games, Proton/Wine, gamescope, common launchers; configurable), the
machine is on battery, or quiet hours. A pause stops in-flight video
transcodes immediately and frees GPU memory; ML items in progress finish.

## Install and run

```sh
cargo build --release -p photovault-node --features ml
photovault-node login          # server URL, email, password, code
photovault-node gpu            # what was detected
photovault-node run            # or: once
photovault-node status
```

Requirements: libsodium, `ffmpeg`/`ffprobe` on `PATH` (for video), and for
faces/search, ONNX Runtime 1.31 or later (loaded at run time from
`ORT_DYLIB_PATH` or the system library path). Models are downloaded from
Ente's model host on first run into `<data_dir>/models`.

### GPU acceleration

Set `[gpu] provider` to `auto` (default), `cuda`, `rocm`, `migraphx`,
`webgpu` or `cpu`. Every GPU path falls back to CPU if the provider is missing
or fails Ente's own self-test for a model.

| GPU | Install |
| --- | --- |
| NVIDIA | ONNX Runtime GPU (CUDA 12) build with `libonnxruntime_providers_cuda.so`; NVIDIA driver, CUDA 12 runtime, cuDNN 9, cuBLAS |
| AMD | ONNX Runtime built with MIGraphX; ROCm + MIGraphX (upstream ONNX Runtime has dropped its ROCm provider, so MIGraphX is the practical path) |
| Any Vulkan GPU | ONNX Runtime with the WebGPU provider (Dawn on Vulkan; D3D12 on Windows) |
| None | ONNX Runtime CPU build |

Without the `ml` cargo feature the node builds with no ONNX Runtime at all
and does streamable video only.

## Configuration

`photovault-node config` prints the effective configuration. The file lives at
`~/.config/photovault-node/config.toml` (override with `--config` or
`PHOTOVAULT_NODE_CONFIG`); most keys also have `PHOTOVAULT_NODE_*` environment
overrides. Sections: `tasks` (faces, clip, video), `gpu`, `concurrency`,
`busy` (thresholds, games, quiet hours), `video` (ffmpeg paths, limits),
`network` (bandwidth limit), `retry`, `log` (text/json). Every field is
documented in [`src/config.rs`](src/config.rs).

Keep `faces` and `clip` both on: the apps expect a file's `mldata` to carry
both indexes.

## Faithfulness to Ente

- **Crypto**: libsodium via `libsodium-sys-stable`. Tests reproduce
  ciphertexts from Ente's own Rust core byte for byte (Argon2id, KDF,
  secretbox, sealed box, blobs, 4 MiB chunked streams) and match Ente's SRP
  vectors (`tests/crypto_ente_vectors.rs`, `tests/api_login.rs`).
- **ML**: Ente's own Rust ML crates are vendored unmodified in
  [`vendor/ente/`](vendor/ente) at the revision in `UPSTREAM_REV`, with one
  marked patch adding desktop GPU providers ([`PATCHES.md`](vendor/ente/PATCHES.md)).
  Re-sync with `scripts/sync-ente-ml.sh <ente-checkout>`.
- **mldata**: fixtures built from the web and mobile writers; our output is
  validated with re-implementations of the web (zod) and mobile readers
  (`tests/ml_mldata.rs`).
- **Video**: same ffmpeg arguments and decisions as the Ente desktop app
  (unit test against a copy of its command); end-to-end tests decrypt and play
  the generated streams (`tests/video_hls.rs`).
- **End to end**: `tests/backend_e2e.rs` runs a full pass against a mock
  museum and decrypts every upload; `tests/server_e2e.rs` does the same
  against the real Photovault server (sign-up, upload like the apps, node
  pass, then fetch and decrypt every result through the API).

## Known limitations

- Real-model inference has not yet been compared with Ente's golden outputs
  in CI (needs network access to the model host and ONNX Runtime). Run
  `ORT_DYLIB_PATH=… cargo test -p photovault-node --features ml --test ml_inference -- --ignored`.
- CUDA, MIGraphX and WebGPU paths are compiled but not yet tested on real GPUs.
- Ente defines no index-flag bits for CUDA/MIGraphX, so those runs report
  only the "Rust runtime" bit.
- Passkey and recovery-key sign-in are not supported; use password + TOTP.

## Licence

AGPL-3.0, the same as Ente.
