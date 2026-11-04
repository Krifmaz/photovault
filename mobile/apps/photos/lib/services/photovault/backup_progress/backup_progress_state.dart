// PHOTOVAULT: backup progress (notification + Photos banner), pure Dart.
//
// BackupProgressTracker turns the sync events the app already fires
// (SyncStatusUpdate, upload bytes, which file is uploading) into one
// BackupProgressSnapshot. BackupProgressStrings formats it. Both are free of
// Flutter and platform code so they can be unit tested.
import "dart:collection" show ListQueue;
import "dart:math" as math;

import "package:intl/intl.dart";
import "package:photos/events/sync_status_update_event.dart";

enum BackupPhase {
  /// Nothing to show.
  idle,

  /// Files are queued and uploading.
  uploading,

  /// The user tapped Pause.
  pausedByUser,

  /// Wi-Fi only is on and the phone is on mobile data.
  waitingForWifi,

  /// The server could not be reached.
  cantReachServer,

  /// Any other error that stopped the backup.
  error,

  /// The session finished. Shown for a few seconds.
  complete,
}

class BackupProgressSnapshot {
  const BackupProgressSnapshot({
    this.phase = BackupPhase.idle,
    this.completed = 0,
    this.total = 0,
    this.currentFile,
    this.bytesPerSecond,
    this.timeLeft,
    this.errorSummary,
    this.errorDetails,
    this.wifiOnly = true,
  });

  final BackupPhase phase;
  final int completed;
  final int total;
  final String? currentFile;
  final double? bytesPerSecond;
  final Duration? timeLeft;

  /// Short reason, shown after "Backup paused: ".
  final String? errorSummary;

  /// Full error text the user can copy from the Backup status screen.
  final String? errorDetails;
  final bool wifiOnly;

  bool get isUploading => phase == BackupPhase.uploading;

  bool get isPaused =>
      phase == BackupPhase.pausedByUser ||
      phase == BackupPhase.waitingForWifi ||
      phase == BackupPhase.cantReachServer ||
      phase == BackupPhase.error;

  bool get isVisible => phase != BackupPhase.idle;

  /// 0..1, or null while the total is unknown.
  double? get fraction =>
      total <= 0 ? null : (completed / total).clamp(0.0, 1.0).toDouble();

  BackupProgressSnapshot copyWith({bool? wifiOnly}) => BackupProgressSnapshot(
    phase: phase,
    completed: completed,
    total: total,
    currentFile: currentFile,
    bytesPerSecond: bytesPerSecond,
    timeLeft: timeLeft,
    errorSummary: errorSummary,
    errorDetails: errorDetails,
    wifiOnly: wifiOnly ?? this.wifiOnly,
  );

  @override
  bool operator ==(Object other) =>
      other is BackupProgressSnapshot &&
      other.phase == phase &&
      other.completed == completed &&
      other.total == total &&
      other.currentFile == currentFile &&
      other.bytesPerSecond == bytesPerSecond &&
      other.timeLeft == timeLeft &&
      other.errorSummary == errorSummary &&
      other.errorDetails == errorDetails &&
      other.wifiOnly == wifiOnly;

  @override
  int get hashCode => Object.hash(
    phase,
    completed,
    total,
    currentFile,
    bytesPerSecond,
    timeLeft,
    errorSummary,
    errorDetails,
    wifiOnly,
  );

  @override
  String toString() =>
      "BackupProgressSnapshot($phase, $completed/$total, $currentFile)";
}

/// Folds sync events into a [BackupProgressSnapshot].
///
/// One backup session can span several upload batches: RemoteSyncService
/// re-runs itself while files remain and restarts its counters each time. The
/// tracker adds the batches up so the count keeps going ("446 of 2,026")
/// instead of starting over.
class BackupProgressTracker {
  BackupProgressTracker({
    this.rateWindow = const Duration(seconds: 20),
    DateTime Function()? clock,
  }) : _clock = clock ?? DateTime.now;

  final Duration rateWindow;
  final DateTime Function() _clock;

  BackupPhase _phase = BackupPhase.idle;
  int _base = 0;
  int _batchCompleted = 0;
  int _batchTotal = 0;
  String? _errorSummary;
  String? _errorDetails;
  bool _wifiOnly = true;
  final _uploading = <String, String>{};
  final _bytes = ListQueue<(DateTime, int)>();
  final _done = ListQueue<DateTime>();

  BackupPhase get phase => _phase;

  int get _completed => _base + _batchCompleted;

  int get _total => math.max(_base + _batchTotal, _completed);

  bool get _hasSession =>
      _phase == BackupPhase.uploading || _total > 0 || _phase.isPausedLike;

  set wifiOnly(bool value) => _wifiOnly = value;

  /// Applies a sync status event. [pausedByUser] says whether the user's
  /// Pause is in effect.
  void onSyncStatus(SyncStatusUpdate event, {required bool pausedByUser}) {
    switch (event.status) {
      case SyncStatus.preparingForUpload:
        final total = event.total ?? 0;
        if (total <= 0) return;
        if (_phase == BackupPhase.uploading) {
          // Next batch of the same session.
          _base += _batchCompleted;
        } else {
          _startSession();
        }
        _batchCompleted = 0;
        _batchTotal = total;
        _phase = BackupPhase.uploading;
        _clearError();
        break;
      case SyncStatus.inProgress:
        final total = event.total ?? 0;
        if (total <= 0) return;
        if (_phase != BackupPhase.uploading) {
          _startSession();
          _phase = BackupPhase.uploading;
          _clearError();
        }
        final completed = (event.completed ?? 0).clamp(0, total);
        if (completed > _batchCompleted) {
          final now = _clock();
          for (var i = _batchCompleted; i < completed; i++) {
            _done.add(now);
          }
        }
        _batchCompleted = completed;
        _batchTotal = total;
        break;
      case SyncStatus.paused:
        if (pausedByUser || event.pauseCause == SyncPauseCause.user) {
          _toPaused(BackupPhase.pausedByUser, null, null);
        } else if (!_hasSession) {
          // Offline with nothing to upload: don't bother the user.
          return;
        } else if (event.pauseCause == SyncPauseCause.wifi) {
          _toPaused(BackupPhase.waitingForWifi, null, null);
        } else {
          _toPaused(
            BackupPhase.cantReachServer,
            BackupProgressStrings.cantReachServer,
            event.details ?? event.reason,
          );
        }
        break;
      case SyncStatus.error:
        if (!_hasSession && event.error == null) return;
        _toPaused(
          BackupPhase.error,
          BackupProgressStrings.errorSummary(event.error),
          event.details ?? event.error?.toString() ?? event.reason,
        );
        break;
      case SyncStatus.completedBackup:
        if (event.wasStopped || pausedByUser) {
          if (pausedByUser) {
            _toPaused(BackupPhase.pausedByUser, null, null);
          } else {
            reset();
          }
          return;
        }
        if (_phase == BackupPhase.complete) return;
        if (_phase == BackupPhase.uploading && _completed > 0) {
          _base = _completed;
          _batchCompleted = 0;
          _batchTotal = 0;
          _phase = BackupPhase.complete;
          _uploading.clear();
          _clearError();
        } else if (_phase == BackupPhase.uploading || _phase.isPausedLike) {
          // Nothing (more) was uploaded; a pause/error clears once a sync
          // runs through cleanly.
          if (_phase != BackupPhase.pausedByUser) reset();
        }
        break;
      case SyncStatus.startedFirstGalleryImport:
      case SyncStatus.completedFirstGalleryImport:
      case SyncStatus.applyingRemoteDiff:
        break;
    }
  }

  /// The user tapped Pause. The current files finish, nothing new starts.
  void onPausedByUser() => _toPaused(BackupPhase.pausedByUser, null, null);

  /// The user tapped Resume or Retry. Shows "Preparing backup…" until the
  /// sync says what it uploads (or that nothing is left).
  void onResumed() {
    if (_phase.isPausedLike) {
      reset();
      _phase = BackupPhase.uploading;
    }
  }

  void onFileStarted(String localID, String name) {
    _uploading.remove(localID);
    _uploading[localID] = name;
  }

  void onFileStopped(String localID) {
    _uploading.remove(localID);
  }

  void onBytesSent(int bytes) {
    if (bytes <= 0) return;
    _bytes.add((_clock(), bytes));
  }

  void reset() {
    _phase = BackupPhase.idle;
    _base = 0;
    _batchCompleted = 0;
    _batchTotal = 0;
    _uploading.clear();
    _bytes.clear();
    _done.clear();
    _clearError();
  }

  BackupProgressSnapshot snapshot() {
    final now = _clock();
    _trim(now);
    double? speed;
    Duration? timeLeft;
    if (_phase == BackupPhase.uploading) {
      speed = _bytesPerSecond(now);
      timeLeft = _timeLeft(now);
    }
    return BackupProgressSnapshot(
      phase: _phase,
      completed: _completed,
      total: _total,
      currentFile: _phase == BackupPhase.uploading && _uploading.isNotEmpty
          ? _uploading.values.last
          : null,
      bytesPerSecond: speed,
      timeLeft: timeLeft,
      errorSummary: _errorSummary,
      errorDetails: _errorDetails,
      wifiOnly: _wifiOnly,
    );
  }

  void _startSession() {
    _base = 0;
    _batchCompleted = 0;
    _batchTotal = 0;
    _bytes.clear();
    _done.clear();
  }

  void _toPaused(BackupPhase phase, String? summary, String? details) {
    _phase = phase;
    _errorSummary = summary;
    _errorDetails = details;
    _uploading.clear();
    _bytes.clear();
    _done.clear();
  }

  void _clearError() {
    _errorSummary = null;
    _errorDetails = null;
  }

  void _trim(DateTime now) {
    final cutoff = now.subtract(rateWindow);
    while (_bytes.isNotEmpty && _bytes.first.$1.isBefore(cutoff)) {
      _bytes.removeFirst();
    }
    while (_done.isNotEmpty && _done.first.isBefore(cutoff)) {
      _done.removeFirst();
    }
  }

  double? _bytesPerSecond(DateTime now) {
    if (_bytes.isEmpty) return null;
    final span = now.difference(_bytes.first.$1).inMilliseconds;
    // Need a little history before the number means anything.
    if (span < 1000) return null;
    final sum = _bytes.fold<int>(0, (a, e) => a + e.$2);
    return sum * 1000 / span;
  }

  Duration? _timeLeft(DateTime now) {
    final remaining = _total - _completed;
    if (remaining <= 0) return null;
    if (_done.length < 2) return null;
    final span = now.difference(_done.first).inMilliseconds;
    if (span < 1000) return null;
    final filesPerMs = _done.length / span;
    return Duration(milliseconds: (remaining / filesPerMs).round());
  }
}

extension on BackupPhase {
  bool get isPausedLike =>
      this == BackupPhase.pausedByUser ||
      this == BackupPhase.waitingForWifi ||
      this == BackupPhase.cantReachServer ||
      this == BackupPhase.error;
}

/// The words on the notification and the banner.
class BackupProgressStrings {
  static const channelName = "Backup progress";
  static const channelDescription =
      "Shows backup progress while photos upload. You can turn it off.";
  static const cantReachServer = "can't reach server";
  static const pause = "Pause";
  static const resume = "Resume";
  static const retry = "Retry";
  static const wifiOnlyOn = "Wi-Fi only: on";
  static const wifiOnlyOff = "Wi-Fi only: off";
  static const wifiOnly = "Wi-Fi only";
  static const copyDetails = "Copy error details";
  static const copied = "Error details copied";
  static const preparing = "Preparing backup…";

  static String count(int n, [String? locale]) =>
      NumberFormat.decimalPattern(locale).format(n);

  static String title(BackupProgressSnapshot s, [String? locale]) {
    switch (s.phase) {
      case BackupPhase.uploading:
        if (s.total <= 0) return preparing;
        return "Backing up ${count(math.min(s.completed + 1, s.total), locale)}"
            " of ${count(s.total, locale)}";
      case BackupPhase.complete:
        final n = s.completed;
        return "Backup complete: ${count(n, locale)} ${n == 1 ? "photo" : "photos"}";
      case BackupPhase.pausedByUser:
        return "Backup paused";
      case BackupPhase.waitingForWifi:
        return "Backup paused: waiting for Wi-Fi";
      case BackupPhase.cantReachServer:
      case BackupPhase.error:
        return "Backup paused: ${s.errorSummary ?? cantReachServer}";
      case BackupPhase.idle:
        return "";
    }
  }

  /// Second line: the file being uploaded, or what to do next.
  static String? text(BackupProgressSnapshot s, [String? locale]) {
    switch (s.phase) {
      case BackupPhase.uploading:
        return s.currentFile;
      case BackupPhase.pausedByUser:
        return s.total > 0
            ? "${count(s.completed, locale)} of ${count(s.total, locale)} backed up. Tap Resume to continue."
            : "Tap Resume to continue.";
      case BackupPhase.waitingForWifi:
        return "Backup continues on Wi-Fi.";
      case BackupPhase.cantReachServer:
      case BackupPhase.error:
        return "Tap Retry, or open the app for details.";
      case BackupPhase.complete:
      case BackupPhase.idle:
        return null;
    }
  }

  /// "1.2 MB/s · 3 min left", or null when neither is known yet.
  static String? speedAndTimeLeft(BackupProgressSnapshot s) {
    final parts = <String>[
      if (s.bytesPerSecond != null) speed(s.bytesPerSecond!),
      if (s.timeLeft != null) timeLeft(s.timeLeft!),
    ];
    return parts.isEmpty ? null : parts.join(" · ");
  }

  static String speed(double bytesPerSecond) {
    const units = ["B/s", "KB/s", "MB/s", "GB/s"];
    var value = bytesPerSecond;
    var unit = 0;
    while (value >= 1000 && unit < units.length - 1) {
      value /= 1000;
      unit++;
    }
    final digits = unit == 0 || value >= 100 ? 0 : 1;
    return "${value.toStringAsFixed(digits)} ${units[unit]}";
  }

  static String timeLeft(Duration d) {
    if (d.inSeconds < 60) return "under a minute left";
    final minutes = (d.inSeconds / 60).ceil();
    if (minutes < 60) return "$minutes min left";
    final h = minutes ~/ 60;
    final m = minutes % 60;
    return m == 0 ? "$h h left" : "$h h $m min left";
  }

  static String errorSummary(Object? error) {
    final name = error?.runtimeType.toString() ?? "";
    switch (name) {
      case "StorageLimitExceededError":
        return "server storage is full";
      case "NoActiveSubscriptionError":
        return "no active storage plan";
      case "DeviceStorageFullError":
        return "phone storage is full";
      case "NoMediaLocationAccessError":
        return "photo location access needed";
      default:
        return "something went wrong";
    }
  }

  /// The copyable text on the Backup status screen.
  static String errorReport(BackupProgressSnapshot s, DateTime at) {
    return [
      title(s),
      "Time: ${at.toIso8601String()}",
      if (s.total > 0) "Progress: ${s.completed} of ${s.total}",
      if (s.errorDetails != null && s.errorDetails!.isNotEmpty)
        "Details: ${s.errorDetails}",
    ].join("\n");
  }
}
