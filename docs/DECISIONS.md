# Decisions

Calls that are not obvious from the code, newest first.

## Android release build (Photos APK)

- **Version from the tag.** `release.yml` passes the tag without the `v` as
  `--build-name` (versionName) and `major*1000000 + minor*10000 + patch` as
  `--build-number` (versionCode). It rises with every patch release, needs no
  stored counter and is above Ente's last code (2158), so it never looks like a
  downgrade. Limits: minor < 100, patch < 10000; the version job fails loudly
  past them. `pubspec.yaml` carries the same scheme for local builds only.
- **Release key is mandatory for releases.** A release build without the
  `PHOTOVAULT_KEYSTORE_*` secrets fails instead of shipping a debug-signed APK,
  and the certificate is checked against the Photovault key's SHA-256. PR and
  push builds still fall back to the debug key so CI works without secrets.
- **Compressed native libraries** (`useLegacyPackaging = true`). Stored,
  page-aligned libraries let Android map them straight from the APK, but they
  make the arm64 APK about 150 MB of libraries instead of about 63 MB. The APK is
  downloaded from GitHub Releases, so download size wins over the extra install
  space the extracted copies take.
- **arm64 only, split per ABI.** CI builds the Rust library only for
  `aarch64-linux-android`, so an armeabi-v7a APK could not run anyway; the old
  single APK still carried about 60 MB of v7a plugin libraries.
- **Dart debug info split out** (`--split-debug-info`, not `--obfuscate`).
  Symbols are a workflow artifact for decoding stack traces. Obfuscation was
  left off because Ente code uses `runtimeType` names at runtime.
