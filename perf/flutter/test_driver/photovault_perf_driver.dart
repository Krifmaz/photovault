// Host-side driver for integration_test/photovault_perf_test.dart.
//
// Copy to mobile/apps/photos/test_driver/photovault_perf_driver.dart. It receives
// the frame statistics the test reports, writes them to build/photovault_perf/,
// prints a table, and exits 1 when a scenario is over the frame budget (or the
// display was not running at 120 Hz). See perf/flutter/README.md.

import 'dart:convert';
import 'dart:io';

import 'package:flutter_driver/flutter_driver.dart' as driver;
import 'package:integration_test/integration_test_driver.dart';

const String _outDir = 'build/photovault_perf';

Future<void> main() {
  return integrationDriver(
    responseDataCallback: (Map<String, dynamic>? data) async {
      if (data == null) {
        stderr.writeln('photovault_perf: the test reported no data');
        exit(1);
      }
      await Directory(_outDir).create(recursive: true);

      final timeline = data.remove('photovault_timeline');
      if (timeline is Map<String, dynamic>) {
        final summary = driver.TimelineSummary.summarize(
          driver.Timeline.fromJson(timeline),
        );
        await summary.writeTimelineToFile(
          'photovault_timeline',
          destinationDirectory: _outDir,
          pretty: true,
          includeSummary: true,
        );
      }

      await writeResponseData(
        data,
        testOutputFilename: 'photovault_perf',
        destinationDirectory: _outDir,
      );

      final frames = data['photovault_frames'];
      if (frames is! Map) {
        stderr.writeln('photovault_perf: no photovault_frames in the report');
        exit(1);
      }
      _printTable(frames.cast<String, dynamic>());
      if (frames['passed'] != true) {
        stderr.writeln(
          'photovault_perf: FAIL (over the ${frames['budgetMs']} ms budget or '
          'not at 120 Hz). Details: $_outDir/photovault_perf.json',
        );
        exit(1);
      }
      stdout.writeln('photovault_perf: PASS');
    },
  );
}

void _printTable(Map<String, dynamic> frames) {
  String f(Object? v) => v is num ? v.toStringAsFixed(2) : 'n/a';
  stdout.writeln(
    'Display ${f(frames['refreshRateHz'])} Hz, budget ${f(frames['budgetMs'])} ms',
  );
  stdout.writeln(
    'scenario            frames  build p90  build p99  raster p90  raster p99  over%   result',
  );
  for (final name in const ['gallery_scroll', 'viewer_open_swipe']) {
    final s = frames[name];
    if (s is! Map) {
      stdout.writeln('${name.padRight(20)}(not run)');
      continue;
    }
    stdout.writeln(
      '${name.padRight(20)}'
      '${'${s['frames']}'.padLeft(6)}  '
      '${f(s['buildP90Ms']).padLeft(9)}  '
      '${f(s['buildP99Ms']).padLeft(9)}  '
      '${f(s['rasterP90Ms']).padLeft(10)}  '
      '${f(s['rasterP99Ms']).padLeft(10)}  '
      '${f(s['overBudgetPct']).padLeft(5)}   '
      '${s['passed'] == true ? 'PASS' : 'FAIL'}',
    );
  }
  stdout.writeln(const JsonEncoder.withIndent('  ').convert(frames));
}
