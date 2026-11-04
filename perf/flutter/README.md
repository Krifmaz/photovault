# Flutter frame-budget test (APK 1, `app.photovault`)

Macrobenchmark's `FrameTimingMetric` cannot see Flutter frames: Flutter builds frames on
its own UI thread and rasterizes them on its own raster thread into a surface that
HWUI does not draw. So the frame numbers for the Flutter app come from this test, which
reads the engine's own per-frame `FrameTiming` reports while it scrolls the gallery and
opens the photo viewer, in `--profile` mode on a real phone.

| File | Copy to (fork track) |
| --- | --- |
| `integration_test/photovault_perf_test.dart` | `mobile/apps/photos/integration_test/photovault_perf_test.dart` |
| `test_driver/photovault_perf_driver.dart` | `mobile/apps/photos/test_driver/photovault_perf_driver.dart` |

Both files are self-contained. The test imports only `flutter`, `flutter_test`,
`integration_test` (already dev dependencies of the app) and `package:photos/main.dart`;
change that one import if the app's Dart package is renamed. They do not replace the
inherited `home_gallery_scroll_test.dart` / `perf_driver.dart`.

## What it measures

Two scenarios, each recorded separately:

| Scenario | Journey |
| --- | --- |
| `gallery_scroll` | Fling the home gallery down 3 times and back up 3 times (6000 px/s) |
| `viewer_open_swipe` | Tap the first photo (hero transition), swipe 5 photos forward and 5 back, system back |

For each: frame count, p50/p90/p99/worst **build** time (UI thread) and **raster** time
(raster thread), and the share of frames over budget.

**Pass:** for both scenarios, build p90 and p99 and raster p90 and p99 are all ≤ the
budget (default 8.33 ms = one frame at 120 Hz), and the display reports ≥ 119 Hz. The test
records the verdict; the driver writes the numbers and then exits 1 on a failure, so the
numbers are kept even when the run fails.

Output, relative to `mobile/apps/photos/`:

* `build/photovault_perf/photovault_perf.json`: all numbers, plus Flutter's own
  `watchPerformance` summary per scenario (`<scenario>_flutter_summary`)
* `build/photovault_perf/photovault_timeline.timeline_summary.json` (only with
  `PERF_TIMELINE=true`)
* a table on stdout

## Device preparation

1. A 120 Hz phone with USB debugging enabled. Force the peak refresh rate first:
   `perf/scripts/device-120hz.sh --no-macro --no-gfxinfo --keep-refresh` sets it and
   leaves it set (or set "Smooth display" / "High refresh rate" in the phone's settings).
2. Install the **profile** build once with `flutter drive` (below). A profile build is
   signed with the debug key, so uninstall a release `app.photovault` first.
3. Open the app, sign in to the test account (or a server with a test library of a few
   thousand photos), grant photo access, and wait until the home gallery shows
   thumbnails. `flutter drive` reinstalls without clearing data, so this stays.
   `--dart-define=PERF_EMAIL=... --dart-define=PERF_PASSWORD=...` can sign in
   automatically, but the values are compiled into the build; prefer signing in by hand.

## Run

From `mobile/apps/photos/`, with the phone connected (`flutter devices` for its id):

```sh
flutter drive --profile --flavor independent \
  --driver=test_driver/photovault_perf_driver.dart \
  --target=integration_test/photovault_perf_test.dart \
  --dart-define=endpoint=https://photos.example.lan \
  -d <device-id>
```

`--flavor independent` is the flavor whose application id is `app.photovault`
(contracts/apps.json). `--dart-define=endpoint=` is the app's default server; use the same
value as the release build. Exit code 0 means both scenarios are within budget at 120 Hz.

Options, each as `--dart-define=NAME=value`:

| Name | Default | Meaning |
| --- | --- | --- |
| `PERF_FRAME_BUDGET_MS` | `8.33` | Frame budget in ms |
| `PERF_REQUIRE_120HZ` | `true` | Fail when the display reports < 119 Hz |
| `PERF_GALLERY_ID` | `ente.photos.gallery.home_gallery.scroll` | `Semantics.identifier` around the home gallery's scroll view |
| `PERF_TIMELINE` | `false` | Also record a timeline with `traceAction` (adds overhead) |
| `PERF_FLINGS` | `3` | Fling pairs in `gallery_scroll` |
| `PERF_SWIPES` | `5` | Swipes each way in `viewer_open_swipe` |
| `PERF_EMAIL`, `PERF_PASSWORD` | empty | Sign in when the app starts signed out |

## What the test relies on in the app

* The home gallery's scroll view is wrapped in `Semantics(identifier: ...)` with the value
  `ente.photos.gallery.home_gallery.scroll` (set by the inherited gallery code from the
  `home_gallery` tag prefix). If the fork renames it, pass `PERF_GALLERY_ID`, and keep the
  Macrobenchmark selector in `perf/android/journeys/.../Journeys.kt` in step (it already
  accepts a `photovault.` spelling).
* Gallery tiles wrap their thumbnail in a `Hero`; the viewer is a `PageView`.
* Sign-in keys (only for `PERF_EMAIL`): `signInButton`, `logInButton`,
  `passwordInputField`, `verifyPasswordButton`.

## Why profile mode, and its limits

Debug builds are JIT-compiled and many times slower; release builds strip the
VM service that `flutter drive` needs. Profile is AOT-compiled like release, with tracing
kept, so it is the closest measurable build. It still differs slightly from release
(timeline hooks, service extension), so a profile build within budget is a strong but not
perfect proxy. The startup numbers for the release APK come from the Macrobenchmark
module (`StartupBenchmark` with `targetPackage=app.photovault`).
