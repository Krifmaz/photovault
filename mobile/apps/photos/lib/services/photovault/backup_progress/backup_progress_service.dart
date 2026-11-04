// PHOTOVAULT: backup progress notification and Photos banner.
//
// Listens to the sync events, keeps a [BackupProgressSnapshot] for the banner
// and the Backup status screen, and drives the Android notification through
// the "app.photovault/backup_progress" channel (BackupProgressBridge.kt).
// While a backup uploads, Android runs it as a dataSync foreground service so
// it keeps going in the background and with the screen off.
import "dart:async";
import "dart:io";

import "package:flutter/foundation.dart";
import "package:flutter/services.dart";
import "package:logging/logging.dart";
import "package:permission_handler/permission_handler.dart";
import "package:photos/core/event_bus.dart";
import "package:photos/events/backup_updated_event.dart";
import "package:photos/events/sync_status_update_event.dart";
import "package:photos/models/backup/backup_item_status.dart";
import "package:photos/module/upload/service/file_uploader.dart";
import "package:photos/service_locator.dart";
import "package:photos/services/app_navigation_service.dart";
import "package:photos/services/photovault/backup_progress/backup_progress_state.dart";
import "package:photos/services/sync/sync_service.dart";
import "package:photos/ui/settings/backup/backup_status_screen.dart";
import "package:shared_preferences/shared_preferences.dart";

class BackupProgressService {
  BackupProgressService._();

  static final instance = BackupProgressService._();

  static const channel = MethodChannel("app.photovault/backup_progress");
  static const _permissionAskedKey =
      "photovault.backup_notification_permission_asked";
  static const _completeVisibleFor = Duration(seconds: 5);
  static const _minUpdateInterval = Duration(seconds: 1);

  final _logger = Logger("BackupProgressService");
  final _tracker = BackupProgressTracker();

  /// What the banner and the Backup status screen show.
  final ValueNotifier<BackupProgressSnapshot> progress = ValueNotifier(
    const BackupProgressSnapshot(),
  );

  /// When the last error happened, for the copyable report.
  DateTime? lastErrorAt;

  late SharedPreferences _prefs;
  bool _initialized = false;
  Timer? _tick;
  Timer? _completeTimer;
  DateTime _lastPushed = DateTime.fromMillisecondsSinceEpoch(0);
  Map<String, Object?>? _lastSpec;

  bool get isPausedByUser => backupSettings.isBackupPausedByUser();

  /// Foreground app only: the background engine has no notification channel
  /// and Android does not let it start a foreground service.
  Future<void> init(SharedPreferences prefs) async {
    if (_initialized) return;
    _initialized = true;
    _prefs = prefs;
    _tracker.wifiOnly = !backupSettings.shouldBackupOverMobileData();
    if (isPausedByUser) {
      _tracker.onPausedByUser();
    }
    Bus.instance.on<SyncStatusUpdate>().listen(_onSyncStatus);
    Bus.instance.on<BackupUpdatedEvent>().listen(_onBackupUpdated);
    FileUploader.instance.onBytesSent = _tracker.onBytesSent;
    if (Platform.isAndroid) {
      channel.setMethodCallHandler(_onNativeCall);
      unawaited(_consumeLaunchAction());
    }
    _publish(force: true);
  }

  // Actions from the notification, the banner and the Backup status screen.

  Future<void> pause() async {
    _tracker.onPausedByUser();
    _publish(force: true);
    await SyncService.instance.pauseBackup();
  }

  Future<void> resume() async {
    _tracker.onResumed();
    _publish(force: true);
    await SyncService.instance.resumeBackup();
  }

  Future<void> retry() => resume();

  Future<void> toggleWifiOnly() async {
    final allowMobile = !backupSettings.shouldBackupOverMobileData();
    await backupSettings.setBackupOverMobileData(allowMobile);
    _tracker.wifiOnly = !allowMobile;
    if (allowMobile && _tracker.phase == BackupPhase.waitingForWifi) {
      _tracker.onResumed();
    }
    _publish(force: true);
    if (allowMobile && !isPausedByUser) {
      unawaited(SyncService.instance.sync());
    }
  }

  void _onSyncStatus(SyncStatusUpdate event) {
    final before = _tracker.phase;
    _tracker.onSyncStatus(event, pausedByUser: isPausedByUser);
    final after = _tracker.phase;
    if (after != before) {
      _logger.info("backup $before -> $after");
      if (after == BackupPhase.cantReachServer ||
          after == BackupPhase.error) {
        lastErrorAt = DateTime.now();
      }
      if (after == BackupPhase.uploading) {
        unawaited(_askNotificationPermissionOnce());
      }
    }
    _publish(force: after != before);
  }

  void _onBackupUpdated(BackupUpdatedEvent event) {
    for (final localID in event.removedLocalIDs) {
      _tracker.onFileStopped(localID);
    }
    event.upserts.forEach((localID, item) {
      if (item.status == BackupItemStatus.uploading) {
        if (item.progressPercent == null) {
          _tracker.onFileStarted(localID, item.file.displayName);
        }
      } else {
        _tracker.onFileStopped(localID);
      }
    });
    _publish();
  }

  void _publish({bool force = false}) {
    final snapshot = _tracker.snapshot();
    progress.value = snapshot;
    _scheduleTick(snapshot);
    _scheduleCompleteHide(snapshot);
    final now = DateTime.now();
    if (!force && now.difference(_lastPushed) < _minUpdateInterval) {
      return;
    }
    _lastPushed = now;
    unawaited(_pushToNotification(snapshot));
  }

  /// Speed and time left move on their own; refresh them every second.
  void _scheduleTick(BackupProgressSnapshot snapshot) {
    if (snapshot.isUploading) {
      _tick ??= Timer.periodic(_minUpdateInterval, (_) => _publish());
    } else {
      _tick?.cancel();
      _tick = null;
    }
  }

  void _scheduleCompleteHide(BackupProgressSnapshot snapshot) {
    if (snapshot.phase != BackupPhase.complete) {
      _completeTimer?.cancel();
      _completeTimer = null;
      return;
    }
    _completeTimer ??= Timer(_completeVisibleFor, () {
      _completeTimer = null;
      if (_tracker.phase == BackupPhase.complete) {
        _tracker.reset();
        _publish(force: true);
      }
    });
  }

  Future<void> _pushToNotification(BackupProgressSnapshot s) async {
    if (!Platform.isAndroid) return;
    final spec = notificationSpec(s);
    if (mapEquals(spec, _lastSpec)) return;
    _lastSpec = spec;
    try {
      await channel.invokeMethod<void>("show", spec);
    } on MissingPluginException {
      // No Activity engine (e.g. tests).
    } catch (e, s) {
      _logger.warning("Could not update the backup notification", e, s);
    }
  }

  /// What the native side draws. Pure, so tests can check it.
  @visibleForTesting
  static Map<String, Object?> notificationSpec(BackupProgressSnapshot s) {
    if (!s.isVisible) {
      return {"phase": "idle"};
    }
    final detail = BackupProgressStrings.speedAndTimeLeft(s);
    final text = BackupProgressStrings.text(s);
    final actions = <Map<String, String>>[
      if (s.phase == BackupPhase.uploading)
        {"id": "pause", "label": BackupProgressStrings.pause},
      if (s.phase == BackupPhase.pausedByUser ||
          s.phase == BackupPhase.waitingForWifi)
        {"id": "resume", "label": BackupProgressStrings.resume},
      if (s.phase == BackupPhase.cantReachServer ||
          s.phase == BackupPhase.error)
        {"id": "retry", "label": BackupProgressStrings.retry},
      if (s.phase != BackupPhase.complete)
        {
          "id": "wifi",
          "label": s.wifiOnly
              ? BackupProgressStrings.wifiOnlyOn
              : BackupProgressStrings.wifiOnlyOff,
        },
    ];
    return {
      "phase": s.phase.name,
      "title": BackupProgressStrings.title(s),
      "text": text,
      "subText": s.phase == BackupPhase.uploading ? detail : null,
      "bigText": [
        ?text,
        if (s.phase == BackupPhase.uploading) ?detail,
      ].join("\n"),
      "progress": s.completed,
      "max": s.total,
      "indeterminate": s.phase == BackupPhase.uploading && s.total <= 0,
      "showProgress": s.phase == BackupPhase.uploading,
      "foreground": s.phase == BackupPhase.uploading,
      "autoDismissMs": s.phase == BackupPhase.complete
          ? _completeVisibleFor.inMilliseconds
          : 0,
      "actions": actions,
      "channelName": BackupProgressStrings.channelName,
      "channelDescription": BackupProgressStrings.channelDescription,
      "resumingTitle": BackupProgressStrings.preparing,
    };
  }

  Future<dynamic> _onNativeCall(MethodCall call) async {
    _logger.info("notification action ${call.method}");
    switch (call.method) {
      case "pause":
        await pause();
        break;
      case "resume":
        await resume();
        break;
      case "retry":
        await retry();
        break;
      case "wifi":
        await toggleWifiOnly();
        break;
      case "openBackupStatus":
        _openBackupStatus();
        break;
    }
    return null;
  }

  /// The app was opened by tapping the notification.
  Future<void> _consumeLaunchAction() async {
    try {
      final opened = await channel.invokeMethod<bool>("consumeLaunchAction");
      if (opened == true) {
        // Let the home screen build first.
        Future.delayed(const Duration(milliseconds: 600), _openBackupStatus);
      }
    } on MissingPluginException {
      // Not running inside MainActivity.
    } catch (e) {
      _logger.warning("consumeLaunchAction failed", e);
    }
  }

  void _openBackupStatus() {
    // pushPage waits for the navigator on a cold start.
    unawaited(
      AppNavigationService.instance.pushPage(const BackupStatusScreen()),
    );
  }

  /// Android 13+ needs POST_NOTIFICATIONS for the progress to show. Ask once,
  /// the first time a backup starts with the app open.
  Future<void> _askNotificationPermissionOnce() async {
    if (!Platform.isAndroid) return;
    if (_prefs.getBool(_permissionAskedKey) ?? false) return;
    try {
      if (await Permission.notification.isGranted) return;
      await _prefs.setBool(_permissionAskedKey, true);
      await Permission.notification.request();
    } catch (e) {
      _logger.warning("notification permission request failed", e);
    }
  }
}
