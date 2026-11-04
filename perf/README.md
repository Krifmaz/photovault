# Photovault performance tooling

This directory exists to show that both Android apps scroll and animate at the phone's
full refresh rate: **120 Hz, so 8.33 ms per frame, on a real phone, with release builds**.
It holds the tools and pass criteria for that check. It holds no app code: the apps are
built by their own tracks, and everything here addresses them by package name.

| App | Package (release) | Toolkit |
| --- | --- | --- |
| Photovault (APK 1) | `app.photovault` | Flutter, `mobile/apps/photos` |
| Photovault Native (APK 2) | `app.photovault.native` | Kotlin + Jetpack Compose, `native/android` |

## Contents

| Path | What it is |
| --- | --- |
| `android/` | Standalone Gradle project (AGP 8.13, Kotlin 2.2, androidx.benchmark 1.4.1, UiAutomator 2.3). It builds test APKs only. |
| `android/macrobenchmark/` | `com.android.test` module: startup, timeline-scroll and viewer benchmarks for either app (`-e targetPackage`) |
| `android/baselineprofile/` | `com.android.test` module: Baseline Profile and startup profile generator for the native app |
| `android/journeys/` | The UI journeys and selectors, shared by both modules |
| `android/target-stub/` | Empty placeholder app. `com.android.test` requires a target project; the test APKs instrument themselves and drive the real app. |
| `flutter/` | `integration_test` frame-budget test and driver for the Flutter app ([flutter/README.md](flutter/README.md)) |
| `scripts/device-120hz.sh` | One-shot measurement on a phone: refresh rate, installs, macrobenchmarks for both apps, gfxinfo, report |
| `scripts/collect-baseline-profile.sh` | Generates the native app's profiles and copies them into `native/android/app/src/main/` |
| `scripts/perf_report.py` | Parsers and report writer used by the scripts (standard-library Python 3) |
| `results/` | `<date>-<device>.md` reports, committed. Raw dumps go to `results/raw/` (git-ignored). |
| `../.github/workflows/perf.yml` | CI: checks, builds and lints this project; optional emulator smoke run |

## What each piece measures

### Macrobenchmark (`android/macrobenchmark`), both apps

| Test | Journey | Native metrics | Flutter metrics |
| --- | --- | --- | --- |
| `StartupBenchmark.startupCold` / `startupWarm` | Launch until the timeline grid is on screen | `StartupTimingMetric`: timeToInitialDisplayMs, and timeToFullDisplayMs if the app calls `reportFullyDrawn()` | same |
| `TimelineScrollBenchmark.timelineFling` | 3 flings down and 3 up on the timeline grid | `FrameTimingMetric`: frameDurationCpuMs, frameOverrunMs (P50/P90/P95/P99) | `TraceSectionMetric` (see below) |
| `ViewerBenchmark.openViewerAndSwipe` | Open the first photo (shared-element transition), swipe 5 photos forward and 5 back, go back | `FrameTimingMetric` | `TraceSectionMetric` |

**Why Flutter is different.** `FrameTimingMetric` reads HWUI's RenderThread and the
SurfaceFlinger FrameTimeline for the app's window. Flutter builds frames on its own UI
thread and rasterizes on its own raster thread into its own surface, so those frames are
invisible to it. For the Flutter app the scroll and viewer benchmarks record:

* `vsyncCallbacks` (`Choreographer#doFrame%`, count mode): how many vsync callbacks the
  Flutter embedding received during the journey. For the same journey this is about
  twice as high at 120 Hz as at 60 Hz, so a drop shows the app fell back to a lower
  rate or stalled.
* With `-e flutterTrace true` and a **profile** build: the engine's per-frame sections
  `GPURasterizer::Draw` (raster) and `Animator::BeginFrame` (UI), average and max. The app
  is launched with the engine's `trace-systrace` intent extra. Release builds compile the
  Dart timeline out, so these sections do not exist there.

The authoritative Flutter build and raster percentiles come from `flutter/`.

Instrumentation arguments (`-e name value`):

| Argument | Default | Meaning |
| --- | --- | --- |
| `targetPackage` | `app.photovault.native` | App under test |
| `appKind` | from the package (`app.photovault.native*` is native, anything else Flutter) | `native` or `flutter` |
| `iterations` | `10` | Iterations per benchmark |
| `compilation` | `partial` | `none`, `partial` (Baseline Profile if the APK ships one) or `full` |
| `flutterTrace` | `false` | See above |
| `requireTimeline` | `true` | `false` for a smoke run without a signed-in library: startup to first frame only, scroll and viewer tests skipped |

Run it by hand (the script below does this for you):

```sh
gradle -p perf/android :macrobenchmark:assembleRelease
adb install -r perf/android/macrobenchmark/build/outputs/apk/release/macrobenchmark-release.apk
adb shell am instrument -w -r -e package app.photovault.perf.macrobenchmark \
  -e targetPackage app.photovault.native \
  app.photovault.perf.macrobenchmark/androidx.test.runner.AndroidJUnitRunner
# results: /sdcard/Android/media/app.photovault.perf.macrobenchmark/*benchmarkData.json and *.perfetto-trace
```

### Flutter frame test (`flutter/`), Flutter app

An `integration_test` run with `flutter drive --profile` that scrolls the home gallery and
opens the viewer. It records every frame's build and raster time from the engine
(`FrameTiming`, plus Flutter's `watchPerformance` summary) and fails above the 8.33 ms
budget. Exact command and setup: [flutter/README.md](flutter/README.md).

### gfxinfo pass (`scripts/device-120hz.sh`), native app

Independent of Macrobenchmark: `dumpsys gfxinfo <pkg> reset`, then scripted
`adb shell input swipe` flings on the timeline. After each swipe the script runs
`dumpsys gfxinfo <pkg> framestats` (it holds only the last ~120 frames, which is 1 s at
120 Hz) and merges the dumps by IntendedVsync. Reported: the system's janky-frame %
(deadline based, and the legacy figure), the system's p50/p90/p99, and p50/p90/p99/max
from framestats (`FrameCompleted - IntendedVsync`), with the coverage of frames captured.

### Baseline Profile (`android/baselineprofile`), native app

`BaselineProfileRule` runs startup → timeline → fling → open viewer → swipe → back, and
writes a Baseline Profile and a startup profile (`includeInStartupProfile = true`).
`scripts/collect-baseline-profile.sh` runs it and copies the result to:

* `native/android/app/src/main/baseline-prof.txt`
* `native/android/app/src/main/baselineProfiles/startup-prof.txt`

It needs Android 13+ (API 33), or a rooted device on API 28+, and a release build.

## Pass criteria

All on a real 120 Hz phone, release builds, peak refresh rate forced, display reporting
≥ 119 Hz during the run.

| Check | Where it comes from | Pass |
| --- | --- | --- |
| Native scroll and viewer frame time | Macrobenchmark `frameDurationCpuMs` | P99 ≤ 8.33 ms |
| Native deadline misses | Macrobenchmark `frameOverrunMs` (API 31+) | P99 ≤ 0 ms |
| Native jank | gfxinfo `Janky frames` | < 1 % |
| Native frame time | gfxinfo framestats | p99 ≤ 8.33 ms |
| Flutter scroll and viewer | `flutter/` build and raster time | p90 and p99 ≤ 8.33 ms, both threads |
| Cold start, native | Macrobenchmark `timeToInitialDisplayMs` median | ≤ 500 ms |
| Warm start, native | same | ≤ 200 ms |
| Cold start, Flutter | same | ≤ 800 ms |
| Warm start, Flutter | same | ≤ 300 ms |

The startup targets are for a current flagship-class phone with the Baseline Profile
installed (`compilation=partial`). They are targets, not laws of physics: record the device
in the report and adjust them in `scripts/perf_report.py` (`STARTUP_TARGETS_MS`) and here
together if the reference phone changes. The gfxinfo framestats p99 includes GPU
completion, so it is the strictest number here; the deadline-based janky % is the one that
says whether a frame was actually late on screen.

## Running on a phone

```sh
# once: build the test APKs (needs Google's Maven; CI artifacts "perf-test-apks" work too)
gradle -p perf/android assembleRelease

# both apps, release builds
perf/scripts/device-120hz.sh \
  --native-apk path/to/photovault-native-release.apk \
  --flutter-apk path/to/photovault-release.apk
```

The script prints the refresh rates it finds, forces the peak rate
(`settings put system peak_refresh_rate` / `min_refresh_rate`, restored on exit), installs
the APKs, runs the macrobenchmarks for each installed app, runs the gfxinfo pass on the
native app, and writes `perf/results/<date>-<device>.md` with a pass/fail table. It exits 1
when a criterion fails. `--help` lists the options (`--apps native`, `--iterations`,
`--compilation`, `--flutter-trace`, `--no-macro`, `--no-gfxinfo`, `--keep-refresh`, ...).
Then run the Flutter frame test ([flutter/README.md](flutter/README.md)) on the same phone and
add its table to the report.

### Device preparation

* USB debugging on, screen awake and unlocked, battery above 50 %, not charging-throttled,
  cool. Close other apps.
* Both apps signed in to the same test server and account (or the native app in
  local-only mode) with a library big enough to fling through for several seconds: a few
  thousand photos, thumbnails already cached (open the app and scroll once by hand).
* An installed app with a different signing key cannot be replaced
  (`INSTALL_FAILED_UPDATE_INCOMPATIBLE`); uninstall it by hand first, then sign in again.
* Some vendors add their own refresh-rate policy (power saving, per-app lists). The report
  records the rate SurfaceFlinger shows while the native app is on screen; a run below
  119 Hz fails.

## How the app tracks wire this in

### Native track (`native/android`, `app.photovault.native`)

Selectors are Compose test tags, exposed to UiAutomator as resource ids:

```kotlin
// once, on the root composable (e.g. in setContent, around the NavHost)
Modifier.semantics { testTagsAsResourceId = true }

// timeline grid (the LazyVerticalGrid / LazyColumn that scrolls)
Modifier.testTag("timeline_grid")
// every photo cell in that grid
Modifier.testTag("photo_tile")
// the viewer's HorizontalPager
Modifier.testTag("viewer_pager")
```

Also:

* Call `reportFullyDrawn()` (`ReportDrawnWhen { timelineHasPhotos }` in Compose) once the
  timeline shows thumbnails, so startup reports timeToFullDisplayMs.
* Depend on `androidx.profileinstaller:profileinstaller` (Compose pulls it in) so a
  sideloaded install compiles the shipped profile.
* Check in the files `collect-baseline-profile.sh` writes. For the startup profile to
  drive DEX layout, set in `native/android/gradle.properties` (AGP 8.x; R8 must be on for
  release):
  ```properties
  android.experimental.art-profile-r8-rewriting=true
  android.experimental.r8.dex-startup-optimization=true
  ```
* Release builds must not be debuggable (Macrobenchmark refuses debuggable targets).
* A launcher activity that reaches the timeline without extra taps once signed in.

### Fork track (`mobile/apps/photos`, `app.photovault`)

* Copy `flutter/integration_test/photovault_perf_test.dart` and
  `flutter/test_driver/photovault_perf_driver.dart` into the app at the same relative
  paths (details in [flutter/README.md](flutter/README.md)).
* Keep the home gallery's `Semantics(identifier: "ente.photos.gallery.home_gallery.scroll")`
  (inherited code). If it is renamed, `photovault.gallery.home_gallery.scroll` is the
  spelling the Macrobenchmark selector also accepts; pass the new value to the Flutter test
  with `--dart-define=PERF_GALLERY_ID=...`.
* Optional, makes the Macrobenchmark viewer journey exact instead of coordinate based:
  `Semantics(identifier: "photovault.gallery.tile")` on gallery tiles and
  `Semantics(identifier: "photovault.viewer.pager")` around the viewer's `PageView`.
  Flutter exposes `Semantics.identifier` to UiAutomator as the resource id.
* Release flavor `independent` with application id exactly `app.photovault`
  (contracts/apps.json); the profile build for `flutter drive` uses the same flavor.

## CI (`.github/workflows/perf.yml`)

* On pull requests touching `perf/**` and on manual runs: shellcheck, Python and Dart
  syntax checks, then `gradle -p perf/android assemble lint` with Gradle 8.14.3 from
  `gradle/actions/setup-gradle` (no wrapper jar is checked in). The test APKs are
  uploaded as `perf-test-apks`. None of this needs either app, so it passes on its own.
* Manual run with `apk_run_id`: downloads that run's APK artifacts, picks the Photovault
  APKs by package name (aapt2), and runs `device-120hz.sh --emulator --smoke` on an API 34
  x86_64 emulator (KVM). A fresh emulator has no signed-in library, so this measures
  startup to first frame only and checks that the test APK, the selectors and the apps
  install and launch together. Results are uploaded as `perf-emulator-results`.

## Honest limits

* **Emulator numbers are not 120 Hz numbers.** The CI emulator renders at 60 Hz with a
  software GPU on shared runners. It shows that the tooling works and catches crashes; it
  says nothing about frame times. The final check is `device-120hz.sh` plus the Flutter
  test on a real 120 Hz phone with release builds.
* The release APKs are arm64-v8a only. The emulator job relies on the ARM translation in
  the `google_apis` x86_64 image; if an APK will not start there, that is a limit of the
  emulator, not of the app.
* Flutter frame numbers come from a **profile** build (release builds have no VM service
  for `flutter drive`). Profile is AOT-compiled like release and is the closest measurable
  build; Flutter startup on the release APK is still measured by Macrobenchmark.
* UiAutomator turns on accessibility, which makes Flutter build its semantics tree. That
  adds some UI-thread work to Flutter frames during Macrobenchmark runs, one more reason
  the Flutter frame verdict comes from `flutter/`, where semantics stay off.
* Scripted flings are synthetic; human input has different velocities. The journeys are
  chosen to be at least as demanding as normal use: fast flings through uncached
  thumbnails and back-to-back pager swipes.
* One phone is one data point. Record the device, Android build and app versions (the
  report does) and repeat on a mid-range 120 Hz phone before calling it done.
* Nothing in this directory was run against the apps yet: the Kotlin and Gradle code is
  validated by CI, the scripts and report parser against canned `dumpsys` output, and the
  Flutter files by `flutter analyze` against a stub app.
