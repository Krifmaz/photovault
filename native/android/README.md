# Photovault Native (Android)

APK 2 of Photovault: a Kotlin / Jetpack Compose client on top of `photovault-core` (Rust, UniFFI).
Identity and IDs are fixed by [`contracts/apps.json`](../../contracts/apps.json), the core API by
[`contracts/photovault-core.md`](../../contracts/photovault-core.md), the look by
[`branding/encore-tokens.json`](../../branding/encore-tokens.json).

| | |
|---|---|
| applicationId | `app.photovault.native` (debug: `app.photovault.native.debug`) |
| namespace / Kotlin package | `app.photovault.android` (`native` is a Java keyword) |
| minSdk / targetSdk / compileSdk | 28 / 36 / 37 |
| ABIs | arm64-v8a, x86_64 |

## Modules

- `:app` – the Compose app (single activity, Navigation Compose, ViewModels + StateFlow).
  `data/CoreRepository` mirrors the core contract; `data/fake/FakeCore` is an in-memory demo library
  (640 items, placeholder pictures drawn in code) used when the core bindings are missing and by the tests.
- `:core` – Android library for the UniFFI Kotlin (`core/build/generated/uniffi`) and
  `libphotovault_core.so` (`core/build/jniLibs/<abi>/`), both produced by the `core` job of
  `.github/workflows/android-native.yml`. Without them it still builds and `:app` is compiled with FakeCore only
  (`BuildConfig.REAL_CORE` says which; Gradle logs it at configuration time).
- `:baselineprofile` – reserved for the perf track (Baseline Profile + Macrobenchmark).
  `app/src/main/baseline-prof.txt` is a placeholder until then; `androidx.profileinstaller` is in the app.

## Build

Android builds run in GitHub Actions (`android-native.yml`, job `apk`):
`./gradlew testReleaseUnitTest lintRelease assembleRelease`, artifact `photovault-native-apk`.

- `-Pphotovault.defaultServer=https://photos.example.com` prefills the connect screen (`BuildConfig.DEFAULT_SERVER_URL`).
- `-Pphotovault.versionName=0.1.0-beta -Pphotovault.versionCode=1`.
- Release signing reads `PV_KEYSTORE`, `PV_KEYSTORE_PASSWORD`, `PV_KEY_ALIAS`, `PV_KEY_PASSWORD`
  (set by CI); without them the release build is signed with the debug key.

## Demo library (FakeCore)

Any address connects except hosts containing `unreachable` (shows the "Can't reach … home Wi-Fi?" error),
`notphotovault`, or `compat` (a compatible server without Photovault info). Any non-empty password signs in
(`wrong` is rejected); add `+otp`, `+totp`, `+passkey` or `+recovery` to the email to reach those steps.
The code `123456` is accepted. Videos show an error since there are no real files.

## Notes

- `app/src/realCore` holds `RealCore`, the adapter from the generated `app.photovault.core.Core` to
  `CoreRepository`, and the `CoreProvider` that picks it at runtime: once `libphotovault_core.so` loads and
  core opens its store, the app uses the real core. If loading fails, a debug build falls back to FakeCore and
  a release build to `UnavailableCore` (every call reports the problem). `-Pphotovault.demo=true` forces
  FakeCore. CI passes `-Pphotovault.requireRealCore=true`, so a release APK without the real core fails the
  build, and it checks that both ABIs' `libphotovault_core.so` are in the APK.
- Core is opened on a background thread at process start (`AppGraph.warmUp`), since that decrypts the local
  index; MainActivity resolves the start screen on `Dispatchers.IO`.
- CoreError fields named `message` in Rust are `detail` in Kotlin (`core/uniffi.toml`); the adapter maps
  every variant, including the HTTP status of `Server`.
- Core sends the email code itself when `login` answers `EmailOtt`. "Use recovery key" sends a code to the
  typed email and verifies it without a password, so core answers `NeedsRecoveryKey` and the recovery screen
  opens (core only accepts a recovery key after the server has authorized the account).

- Thumbnails come from core's `thumbnail(id)` through a Coil `Fetcher` on `Dispatchers.IO`, decoded at cell
  size into Coil's memory cache. Decrypted thumbnails are never written to Coil's disk cache (core keeps its
  own cache encrypted at rest). Viewer originals are decrypted into app-private `cache/viewer`, at most six
  are kept, and they are deleted when the viewer closes.
- Pinch-to-zoom scales the grid in its graphics layer and only changes the column count (2/3/4 by day,
  6 by month, 10 by year) when a threshold is crossed, so a pinch does not recompose the grid.
- Videos (contract v1.2 "video"): `data/VideoRepository` asks core for a stream of every video (`video_stream`);
  core answers with an HLS playlist on its loopback server (127.0.0.1, random port, secret token), decrypting
  Ente's streamable segments in memory, and the viewer plays it with Media3's HLS module. Without a stream the
  original is downloaded and played by the same player. One ExoPlayer (`ui/viewer/video`) serves all pages: only
  the settled page attaches a Compose `PlayerSurface` (TextureView, so it follows the pager and dismiss
  transforms); controls fade after 3 s, mute sticks across videos, audio focus is handled by ExoPlayer, and the
  player is released (and core's streams ended with `stop_streams`) when the viewer closes.
