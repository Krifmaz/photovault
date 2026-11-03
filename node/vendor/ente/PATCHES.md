# Vendored Ente crates

`ml/`, `image/` and `assets/` are Ente's `rust/crates/{ml,image,assets}`
(`ente-ml`, `ente-image`, `ente-assets`) from
[ente-io/ente](https://github.com/ente-io/ente), AGPL-3.0, at the commit in
`UPSTREAM_REV`. Based on Ente (ente.com).

They are path dependencies of `photovault-node`, built only with
`--features ml`, and excluded from the Photovault workspace (root
`Cargo.toml`), so Photovault's clippy settings do not apply to them.

## Updating

```sh
node/scripts/sync-ente-ml.sh /path/to/ente-checkout
```

The script copies the crates' tracked files, replaces every
`*.workspace = true` with the concrete value from Ente's workspace
`Cargo.toml` (sibling Ente crates become `../<crate>` paths; the inherited
`[lints]` table is removed), writes `UPSTREAM_REV`, and re-applies
`patches/*.patch` in order. A patch that no longer applies is reported as a
CONFLICT and the script exits non-zero: fix the code by hand, then regenerate
the patch with `diff -ruN` against a fresh, unpatched sync.

Never edit vendored files without a `// PHOTOVAULT PATCH: <reason>` marker
(`# PHOTOVAULT PATCH:` in TOML) and an entry below.

## Patches

### 0001-desktop-gpu-execution-providers.patch

Desktop GPU execution providers for the compute node.

- `ml/Cargo.toml`: enables `ort`'s `cuda`, `rocm` and `migraphx` features on
  Linux and Windows. With `load-dynamic` these only compile the provider
  registration code; nothing is downloaded or linked.
- `ml/src/onnx/providers.rs`: new `ExecutionProvider::{Cuda, Rocm, MiGraphX}`
  and a process-wide `DesktopExecutionProvider` preference
  (`PlatformDefault`, `Auto`, `Cuda`, `Rocm`, `MiGraphX`, `WebGpu`, `Cpu`).
  `PlatformDefault` (the default) is Ente's upstream behaviour, unchanged.
  Any other value replaces the Linux/Windows provider order:
  `Auto` = CUDA, MIGraphX, ROCm, WebGPU, CPU; `Rocm` = ROCm, MIGraphX, CPU;
  the others = that provider, then CPU. GPU attempts register the GPU
  provider with `error_on_failure` plus CPU for uncovered operators, like
  Ente's WebGPU attempt. Upstream's golden-entry requirement and the WebGPU
  crash-canary quarantine still apply.
- `ml/src/onnx/webgpu.rs`: `model_not_quarantined`, so an explicit WebGPU
  preference skips the enable flag but not the quarantine.
- `ml/src/onnx.rs`: re-exports the preference; `ProviderUsage` records CUDA,
  ROCm and MIGraphX; CUDA/ROCm/MIGraphX sessions run Ente's golden self-test
  (as CoreML sessions do) so a provider that miscomputes falls back; log
  labels for the new providers.
- `ml/src/indexing.rs`: public `DesktopExecutionProvider` and
  `set_desktop_execution_provider`; `AnalyzeImageResult` gains `used_cuda`,
  `used_rocm` and `used_migraphx`.
