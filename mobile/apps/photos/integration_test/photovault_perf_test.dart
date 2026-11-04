// Photovault frame-budget test for the Flutter app (APK 1, app.photovault).
//
// Copy to mobile/apps/photos/integration_test/photovault_perf_test.dart, together
// with perf/flutter/test_driver/photovault_perf_driver.dart, and run it in profile
// mode on a real 120 Hz phone (see perf/flutter/README.md for the exact command):
//
//   flutter drive --profile --flavor independent \
//     --driver=test_driver/photovault_perf_driver.dart \
//     --target=integration_test/photovault_perf_test.dart
//
// It scrolls the home gallery and opens the photo viewer, records every frame's
// build (UI thread) and raster (raster thread) time from the engine's FrameTiming
// reports, and checks p90/p99 of both against the frame budget (8.33 ms = 120 Hz).
// Results go to the driver, which writes them under build/photovault_perf/ and
// exits non-zero when a scenario is over budget.
//
// Self-contained: depends only on flutter, flutter_test, integration_test and the
// app's own `package:photos/main.dart` (change that import if the package is renamed).
//
// Options (all --dart-define, all optional):
//   PERF_FRAME_BUDGET_MS=8.33       frame budget in ms
//   PERF_REQUIRE_120HZ=true         fail when the display reports < 119 Hz
//   PERF_GALLERY_ID=...             Semantics identifier of the home gallery scroll view
//                                   (default ente.photos.gallery.home_gallery.scroll, the
//                                   identifier the inherited gallery code sets)
//   PERF_TIMELINE=false             also record a timeline with traceAction (adds some
//                                   overhead; the driver then writes a timeline summary)
//   PERF_EMAIL / PERF_PASSWORD      sign in with this account if the app starts signed out.
//                                   Prefer signing in by hand once instead: defines end
//                                   up in the build.
//   PERF_FLINGS=3                   number of down+up fling pairs
//   PERF_SWIPES=5                   number of viewer swipes each way

import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:photos/main.dart' as app;

final double kFrameBudgetMs = double.parse(
  const String.fromEnvironment('PERF_FRAME_BUDGET_MS', defaultValue: '8.33'),
);
const bool kRequire120Hz = bool.fromEnvironment(
  'PERF_REQUIRE_120HZ',
  defaultValue: true,
);
const String kGalleryId = String.fromEnvironment(
  'PERF_GALLERY_ID',
  defaultValue: 'ente.photos.gallery.home_gallery.scroll',
);
const bool kTimeline = bool.fromEnvironment('PERF_TIMELINE');
const String kEmail = String.fromEnvironment('PERF_EMAIL');
const String kPassword = String.fromEnvironment('PERF_PASSWORD');
const int kFlings = int.fromEnvironment('PERF_FLINGS', defaultValue: 3);
const int kSwipes = int.fromEnvironment('PERF_SWIPES', defaultValue: 5);

/// Key under which results are sent to the driver.
const String kReportKey = 'photovault_frames';

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  // Real vsync-driven frames, exactly as the app runs outside tests.
  binding.framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.fullyLive;

  testWidgets('Photovault gallery and viewer stay within the frame budget', (
    tester,
  ) async {
    final originalOnError = FlutterError.onError;
    final results = <String, Object?>{};
    try {
      app.main();
      // The app installs its own error handlers; keep app-side errors (network,
      // thumbnails) from failing a performance run, but print them.
      FlutterError.onError = FlutterError.dumpErrorToConsole;

      final refreshRate =
          ui.PlatformDispatcher.instance.views.first.display.refreshRate;
      results['refreshRateHz'] = refreshRate;
      results['budgetMs'] = kFrameBudgetMs;
      debugPrint(
        'photovault_perf: display refresh rate $refreshRate Hz, budget $kFrameBudgetMs ms',
      );

      await _reachGallery(tester);
      await _idle(tester, const Duration(seconds: 3)); // let thumbnails load

      Future<void> scenarios() async {
        results['gallery_scroll'] = await _measure(
          binding,
          tester,
          'gallery_scroll',
          () => _flingGallery(tester),
        );
        results['viewer_open_swipe'] = await _measure(
          binding,
          tester,
          'viewer_open_swipe',
          () => _openViewerAndSwipe(tester),
        );
      }

      if (kTimeline) {
        await binding.traceAction(scenarios, reportKey: 'photovault_timeline');
      } else {
        await scenarios();
      }

      final refreshOk = !kRequire120Hz || refreshRate >= 119.0;
      results['refreshRateOk'] = refreshOk;
      if (!refreshOk) {
        debugPrint(
          'photovault_perf: FAIL display reports $refreshRate Hz; force the peak '
          'refresh rate (perf/scripts/device-120hz.sh) or pass '
          '--dart-define=PERF_REQUIRE_120HZ=false',
        );
      }
      results['passed'] =
          refreshOk &&
          (results['gallery_scroll']! as Map)['passed'] == true &&
          (results['viewer_open_swipe']! as Map)['passed'] == true;
    } finally {
      FlutterError.onError = originalOnError;
      binding.reportData = <String, dynamic>{
        ...?binding.reportData,
        kReportKey: results,
      };
    }
    // The pass/fail verdict is applied by the driver, so that the numbers are
    // written even when a scenario is over budget (a failed test sends no data
    // to integrationDriver's callback).
  });
}

// ------------------------------------------------------------------ journeys

Finder _galleryRoot() => find.byWidgetPredicate(
  (w) => w is Semantics && w.properties.identifier == kGalleryId,
  description: 'Semantics(identifier: $kGalleryId)',
);

Finder _galleryScrollable() => find
    .descendant(of: _galleryRoot(), matching: find.byType(Scrollable))
    .first;

Future<void> _reachGallery(WidgetTester tester) async {
  if (await _pumpUntil(tester, _galleryRoot(), const Duration(seconds: 20))) {
    return;
  }
  final signIn = find.byKey(const ValueKey('signInButton'));
  if (signIn.evaluate().isNotEmpty &&
      kEmail.isNotEmpty &&
      kPassword.isNotEmpty) {
    await tester.tap(signIn);
    await _idle(tester, const Duration(seconds: 1));
    await tester.enterText(find.byType(TextFormField).first, kEmail);
    await tester.tap(find.byKey(const ValueKey('logInButton')));
    await _pumpUntil(
      tester,
      find.byKey(const ValueKey('passwordInputField')),
      const Duration(seconds: 15),
    );
    await tester.enterText(
      find.byKey(const ValueKey('passwordInputField')),
      kPassword,
    );
    await tester.tap(find.byKey(const ValueKey('verifyPasswordButton')));
    // Key derivation and the first sync can take a while.
    if (await _pumpUntil(tester, _galleryRoot(), const Duration(seconds: 90))) {
      return;
    }
  }
  fail(
    'Home gallery (Semantics identifier "$kGalleryId") did not appear. Sign in on '
    'the device once (and grant photo access) before running, or pass '
    'PERF_EMAIL/PERF_PASSWORD; see perf/flutter/README.md.',
  );
}

Future<void> _flingGallery(WidgetTester tester) async {
  final scrollable = _galleryScrollable();
  for (var i = 0; i < kFlings; i++) {
    await tester.fling(
      scrollable,
      const Offset(0, -2500),
      6000,
      warnIfMissed: false,
    );
    await _idle(tester, const Duration(milliseconds: 1500));
  }
  for (var i = 0; i < kFlings; i++) {
    await tester.fling(
      scrollable,
      const Offset(0, 2500),
      6000,
      warnIfMissed: false,
    );
    await _idle(tester, const Duration(milliseconds: 1500));
  }
}

Future<void> _openViewerAndSwipe(WidgetTester tester) async {
  final pagesBefore = find.byType(PageView).evaluate().length;
  // Gallery tiles wrap their thumbnail in a Hero (for the open transition).
  final tile = find.descendant(of: _galleryRoot(), matching: find.byType(Hero));
  expect(tile, findsWidgets, reason: 'no photo tiles in the gallery');
  await tester.tap(tile.first, warnIfMissed: false);
  final opened = await _pumpUntil(
    tester,
    find.byType(PageView),
    const Duration(seconds: 5),
    minCount: pagesBefore + 1,
  );
  expect(opened, isTrue, reason: 'photo viewer (PageView) did not open');
  await _idle(tester, const Duration(milliseconds: 800)); // hero flight
  final pager = find.byType(PageView).last;
  for (var i = 0; i < kSwipes; i++) {
    await tester.fling(pager, const Offset(-400, 0), 2000, warnIfMissed: false);
    await _idle(tester, const Duration(milliseconds: 700));
  }
  for (var i = 0; i < kSwipes; i++) {
    await tester.fling(pager, const Offset(400, 0), 2000, warnIfMissed: false);
    await _idle(tester, const Duration(milliseconds: 700));
  }
  await tester.binding.handlePopRoute(); // system back: close the viewer
  await _idle(tester, const Duration(milliseconds: 800));
}

// ------------------------------------------------------------- measurement

/// Runs [action] while collecting FrameTimings, and returns the stats as JSON.
/// Also wraps it in [IntegrationTestWidgetsFlutterBinding.watchPerformance] so the
/// standard Flutter frame summary is reported under `<name>_flutter_summary`.
Future<Map<String, Object?>> _measure(
  IntegrationTestWidgetsFlutterBinding binding,
  WidgetTester tester,
  String name,
  Future<void> Function() action,
) async {
  final timings = <ui.FrameTiming>[];
  void onTimings(List<ui.FrameTiming> batch) => timings.addAll(batch);
  SchedulerBinding.instance.addTimingsCallback(onTimings);
  try {
    await binding.watchPerformance(
      action,
      reportKey: '${name}_flutter_summary',
    );
    // The engine reports timings in batches (about once a second in profile
    // mode), flushed on the next rasterized frame. Wait, then draw one frame.
    await Future<void>.delayed(const Duration(milliseconds: 1200));
    SchedulerBinding.instance.scheduleFrame();
    await tester.pump();
    await Future<void>.delayed(const Duration(milliseconds: 300));
  } finally {
    SchedulerBinding.instance.removeTimingsCallback(onTimings);
  }
  final stats = FrameStats(timings, kFrameBudgetMs);
  debugPrint('photovault_perf: $name ${stats.describe()}');
  return stats.toJson();
}

class FrameStats {
  FrameStats(List<ui.FrameTiming> timings, this.budgetMs)
    : build =
          timings.map((t) => t.buildDuration.inMicroseconds / 1000.0).toList()
            ..sort(),
      raster =
          timings.map((t) => t.rasterDuration.inMicroseconds / 1000.0).toList()
            ..sort(),
      overBudget = timings
          .where(
            (t) =>
                t.buildDuration.inMicroseconds / 1000.0 > budgetMs ||
                t.rasterDuration.inMicroseconds / 1000.0 > budgetMs,
          )
          .length;

  final double budgetMs;
  final List<double> build; // sorted
  final List<double> raster; // sorted
  final int overBudget;

  int get count => build.length;

  static double? _pct(List<double> sorted, double p) {
    if (sorted.isEmpty) return null;
    final rank = math.max(1, (p / 100.0 * sorted.length).ceil());
    return sorted[math.min(rank, sorted.length) - 1];
  }

  double? get buildP90 => _pct(build, 90);
  double? get buildP99 => _pct(build, 99);
  double? get rasterP90 => _pct(raster, 90);
  double? get rasterP99 => _pct(raster, 99);

  bool get passed =>
      count > 0 &&
      buildP90! <= budgetMs &&
      buildP99! <= budgetMs &&
      rasterP90! <= budgetMs &&
      rasterP99! <= budgetMs;

  Map<String, Object?> toJson() => {
    'frames': count,
    'budgetMs': budgetMs,
    'buildP50Ms': _pct(build, 50),
    'buildP90Ms': buildP90,
    'buildP99Ms': buildP99,
    'buildWorstMs': build.isEmpty ? null : build.last,
    'rasterP50Ms': _pct(raster, 50),
    'rasterP90Ms': rasterP90,
    'rasterP99Ms': rasterP99,
    'rasterWorstMs': raster.isEmpty ? null : raster.last,
    'overBudgetFrames': overBudget,
    'overBudgetPct': count == 0 ? null : 100.0 * overBudget / count,
    'passed': passed,
  };

  String describe() {
    String f(double? v) => v == null ? 'n/a' : v.toStringAsFixed(2);
    return '${passed ? 'PASS' : 'FAIL'} frames=$count '
        'build p90=${f(buildP90)} p99=${f(buildP99)} ms, '
        'raster p90=${f(rasterP90)} p99=${f(rasterP99)} ms, '
        'over budget ${f(count == 0 ? null : 100.0 * overBudget / count)}% '
        '(budget ${budgetMs.toStringAsFixed(2)} ms)';
  }
}

// ------------------------------------------------------------------ helpers

/// Lets real time pass while the app keeps drawing (fully live frame policy).
Future<void> _idle(WidgetTester tester, Duration d) async {
  await Future<void>.delayed(d);
  await tester.pump();
}

Future<bool> _pumpUntil(
  WidgetTester tester,
  Finder finder,
  Duration timeout, {
  int minCount = 1,
}) async {
  final end = DateTime.now().add(timeout);
  while (DateTime.now().isBefore(end)) {
    await tester.pump();
    if (finder.evaluate().length >= minCount) return true;
    await Future<void>.delayed(const Duration(milliseconds: 200));
  }
  return false;
}
