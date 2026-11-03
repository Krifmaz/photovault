import 'dart:async';
import 'dart:collection';
import 'dart:core';
import 'dart:io';

import "package:dio/dio.dart";
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:intl/intl.dart';
import 'package:logging/logging.dart' as log;
import 'package:package_info_plus/package_info_plus.dart';
import 'package:path/path.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

typedef FutureOrVoidCallback = dynamic Function();

extension SuperString on String {
  Iterable<String> chunked(int chunkSize) sync* {
    var start = 0;

    while (true) {
      final stop = start + chunkSize;
      if (stop > length) break;
      yield substring(start, stop);
      start = stop;
    }

    if (start < length) {
      yield substring(start);
    }
  }
}

extension SuperLogRecord on log.LogRecord {
  String toPrettyString([String? extraLines]) {
    final header = "[$loggerName][$level] [$time]";

    var msg = "$header $message";

    if (error != null) {
      if (error is DioException) {
        final e = error as DioException;
        final String? id = e.requestOptions.headers['x-request-id'] as String?;
        if (id != null) {
          msg += "\n⤷ id: $id ";
        }
        if (e.response?.data != null) {
          msg += "\n⤷ type: ${e.type}\n⤷ error: ${e.response?.data}";
        } else {
          msg += "\n⤷ type: ${e.type}\n⤷ error: $error";
        }
      } else {
        msg += "\n⤷ type: ${error.runtimeType}\n⤷ error: $error";
      }
    }
    if (stackTrace != null) {
      msg += "\n⤷ trace: $stackTrace";
    }

    for (var line in extraLines?.split('\n') ?? []) {
      msg += '\n$header $line';
    }

    return msg;
  }
}

// PHOTOVAULT: telemetry removed. Sentry is gone from this shared package
// (Photos, Auth and Locker). `sentryDsn`, `tunnel`, `sentryRetryDelay`,
// `setUserID` and the "report errors" preference are kept as no-ops so the
// other apps still compile; nothing is ever sent anywhere.
class LogConfig {
  String? sentryDsn;

  String? tunnel;

  Duration sentryRetryDelay;

  // Null disables file logging; empty uses the default directory.
  String? logDirPath;

  int maxLogFiles;

  bool enableInDebugMode;

  FutureOrVoidCallback? body;

  DateFormat? dateFmt;

  String prefix;

  LogConfig({
    this.sentryDsn,
    this.tunnel,
    this.sentryRetryDelay = const Duration(seconds: 30),
    this.logDirPath,
    this.maxLogFiles = 10,
    this.enableInDebugMode = false,
    this.body,
    this.dateFmt,
    this.prefix = "",
  }) {
    dateFmt ??= DateFormat("y-M-d");
  }
}

class SuperLogging {
  static final $ = log.Logger('ente_logging');

  static late LogConfig config;

  static late SharedPreferences _preferences;

  static const keyShouldReportErrors = "should_report_errors";

  static const keyAnonymousUserID = "anonymous_user_id";

  static Future<void> main([LogConfig? appConfig]) async {
    appConfig ??= LogConfig();
    SuperLogging.config = appConfig;

    WidgetsFlutterBinding.ensureInitialized();
    _preferences = await SharedPreferences.getInstance();

    appVersion ??= await getAppVersion();

    final enable = appConfig.enableInDebugMode || kReleaseMode;
    sentryIsEnabled = false; // PHOTOVAULT: telemetry removed
    fileIsEnabled = enable && appConfig.logDirPath != null;

    if (fileIsEnabled) {
      await setupLogDir();
    }

    log.Logger.root.level = kDebugMode ? log.Level.ALL : log.Level.INFO;
    log.Logger.root.onRecord.listen(onLogRecord);

    if (!enable) {
      $.info("detected debug mode; file logging disabled.");
    }
    if (fileIsEnabled) {
      $.info("log file for today: $logFile with prefix ${appConfig.prefix}");
    }

    if (appConfig.body == null) return;

    // PHOTOVAULT: telemetry removed (no SentryFlutter.init).
    await appConfig.body!();
  }

  // PHOTOVAULT: telemetry removed; kept as a no-op for API compatibility.
  static void setUserID(String userID) {}

  static String _lastExtraLines = '';

  static Future onLogRecord(log.LogRecord rec) async {
    String? extraLines = "app version: '$appVersion'\n";
    if (extraLines != _lastExtraLines) {
      _lastExtraLines = extraLines;
    } else {
      extraLines = null;
    }

    final str = "${config.prefix} ${rec.toPrettyString(extraLines)}";

    printLog(str);

    if (fileIsEnabled) {
      fileQueueEntries.add('$str\n');
      if (fileQueueEntries.length == 1) {
        flushQueue();
      }
    }
  }

  static final Queue<String> fileQueueEntries = Queue();
  static bool isFlushing = false;

  static void flushQueue() async {
    if (isFlushing || logFile == null) {
      return;
    }
    isFlushing = true;
    final entry = fileQueueEntries.removeFirst();
    await logFile!.writeAsString(entry, mode: FileMode.append, flush: true);
    isFlushing = false;
    if (fileQueueEntries.isNotEmpty) {
      flushQueue();
    }
  }

  // Long logs are truncated unless chunked.
  // https://github.com/flutter/flutter/issues/22665
  static var logChunkSize = 800;

  static void printLog(String text) {
    text.chunked(logChunkSize).forEach(debugPrint);
  }

  // PHOTOVAULT: telemetry removed; always false.
  static bool sentryIsEnabled = false;

  static bool shouldReportErrors() {
    if (_preferences.containsKey(keyShouldReportErrors)) {
      return _preferences.getBool(keyShouldReportErrors)!;
    } else {
      return kDebugMode;
    }
  }

  static Future<void> setShouldReportErrors(bool value) {
    return _preferences.setBool(keyShouldReportErrors, value);
  }

  static File? logFile;

  static bool fileIsEnabled = false;

  static Future<void> setupLogDir() async {
    var dirPath = config.logDirPath;

    if (dirPath == null || dirPath.isEmpty) {
      final root = await getExternalStorageDirectory();
      dirPath = join(root!.path, 'logs');
    }

    final dir = Directory(dirPath);
    await dir.create(recursive: true);

    final files = <File>[];
    final dates = <File, DateTime>{};

    await for (final file in dir.list()) {
      try {
        final date = config.dateFmt!.parse(basename(file.path));
        dates[file as File] = date;
        files.add(file);
      } on Exception catch (_) {}
    }
    final nowTime = DateTime.now();

    if (files.length > config.maxLogFiles) {
      files.sort(
        (a, b) => (dates[a] ?? nowTime).compareTo((dates[b] ?? nowTime)),
      );

      final extra = files.length - config.maxLogFiles;
      final toDelete = files.sublist(0, extra);

      for (final file in toDelete) {
        try {
          $.fine("deleting log file ${file.path}");
          await file.delete();
        } on Exception catch (_) {}
      }
    }

    logFile = File(
      join(dirPath, "${config.dateFmt!.format(DateTime.now())}.txt"),
    );
  }

  static String? appVersion;

  static Future<String> getAppVersion() async {
    final pkgInfo = await PackageInfo.fromPlatform();
    return "${pkgInfo.version}+${pkgInfo.buildNumber}";
  }
}
