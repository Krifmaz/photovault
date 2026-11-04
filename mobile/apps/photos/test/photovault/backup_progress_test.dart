// PHOTOVAULT: backup progress notification + Photos banner.
import "package:ente_components/ente_components.dart";
import "package:flutter/material.dart";
import "package:flutter_test/flutter_test.dart";
import "package:photos/core/errors.dart";
import "package:photos/events/sync_status_update_event.dart";
import "package:photos/services/photovault/backup_progress/backup_progress_service.dart";
import "package:photos/services/photovault/backup_progress/backup_progress_state.dart";
import "package:photos/ui/photovault/backup_progress_banner.dart";

class _Clock {
  DateTime now = DateTime(2026, 11, 4, 15);
  void advance(Duration d) => now = now.add(d);
}

void main() {
  late _Clock clock;
  late BackupProgressTracker tracker;

  void status(
    SyncStatus s, {
    int? completed,
    int? total,
    bool wasStopped = false,
    SyncPauseCause? cause,
    String? details,
    Error? error,
    bool pausedByUser = false,
  }) {
    tracker.onSyncStatus(
      SyncStatusUpdate(
        s,
        completed: completed,
        total: total,
        wasStopped: wasStopped,
        pauseCause: cause,
        details: details,
        error: error,
      ),
      pausedByUser: pausedByUser,
    );
  }

  setUp(() {
    clock = _Clock();
    tracker = BackupProgressTracker(clock: () => clock.now);
  });

  group("tracker", () {
    test("counts, current file, speed and time left", () {
      status(SyncStatus.preparingForUpload, total: 2026);
      tracker.onFileStarted("a", "IMG_2041.HEIC");
      for (var i = 1; i <= 445; i++) {
        tracker.onBytesSent(2000000);
        clock.advance(const Duration(milliseconds: 40));
        status(SyncStatus.inProgress, completed: i, total: 2026);
      }
      final s = tracker.snapshot();
      expect(s.phase, BackupPhase.uploading);
      expect(s.completed, 445);
      expect(s.total, 2026);
      expect(s.currentFile, "IMG_2041.HEIC");
      expect(s.bytesPerSecond, isNotNull);
      expect(s.timeLeft, isNotNull);
      expect(BackupProgressStrings.title(s, "en"), "Backing up 446 of 2,026");
      expect(BackupProgressStrings.text(s), "IMG_2041.HEIC");
      expect(BackupProgressStrings.speedAndTimeLeft(s), contains("MB/s"));
      expect(BackupProgressStrings.speedAndTimeLeft(s), contains("left"));
    });

    test("batches of one session add up", () {
      status(SyncStatus.preparingForUpload, total: 1000);
      status(SyncStatus.inProgress, completed: 1000, total: 1000);
      // RemoteSyncService runs again for the files that remain.
      status(SyncStatus.preparingForUpload, total: 1026);
      status(SyncStatus.inProgress, completed: 26, total: 1026);
      final s = tracker.snapshot();
      expect(s.completed, 1026);
      expect(s.total, 2026);
    });

    test("complete shows the session total", () {
      status(SyncStatus.preparingForUpload, total: 2026);
      status(SyncStatus.inProgress, completed: 2026, total: 2026);
      status(SyncStatus.completedBackup);
      final s = tracker.snapshot();
      expect(s.phase, BackupPhase.complete);
      expect(
        BackupProgressStrings.title(s, "en"),
        "Backup complete: 2,026 photos",
      );
      // The second completedBackup from SyncService changes nothing.
      status(SyncStatus.completedBackup);
      expect(tracker.snapshot().phase, BackupPhase.complete);
    });

    test("a sync with nothing to upload stays idle", () {
      status(SyncStatus.completedBackup);
      expect(tracker.snapshot().isVisible, isFalse);
      status(SyncStatus.paused, cause: SyncPauseCause.network);
      expect(tracker.snapshot().isVisible, isFalse);
    });

    test("network failure during backup: can't reach server + details", () {
      status(SyncStatus.preparingForUpload, total: 10);
      status(
        SyncStatus.paused,
        cause: SyncPauseCause.network,
        details: "type: connectionError\nhost: photos.example.com",
      );
      final s = tracker.snapshot();
      expect(s.phase, BackupPhase.cantReachServer);
      expect(s.isPaused, isTrue);
      expect(
        BackupProgressStrings.title(s),
        "Backup paused: can't reach server",
      );
      final report = BackupProgressStrings.errorReport(
        s,
        DateTime.utc(2026, 11, 4, 15),
      );
      expect(report, contains("Backup paused: can't reach server"));
      expect(report, contains("photos.example.com"));
      expect(report, contains("2026-11-04T15:00:00.000Z"));
    });

    test("other errors name the reason", () {
      status(SyncStatus.preparingForUpload, total: 10);
      status(SyncStatus.error, error: StorageLimitExceededError());
      final s = tracker.snapshot();
      expect(s.phase, BackupPhase.error);
      expect(
        BackupProgressStrings.title(s),
        "Backup paused: server storage is full",
      );
    });

    test("pause, resume, Wi-Fi", () {
      status(SyncStatus.preparingForUpload, total: 10);
      tracker.onPausedByUser();
      expect(tracker.snapshot().phase, BackupPhase.pausedByUser);
      // The stopped sync's events keep it paused.
      status(SyncStatus.completedBackup, wasStopped: true, pausedByUser: true);
      status(SyncStatus.paused, cause: SyncPauseCause.user, pausedByUser: true);
      expect(tracker.snapshot().phase, BackupPhase.pausedByUser);

      tracker.onResumed();
      var s = tracker.snapshot();
      expect(s.phase, BackupPhase.uploading);
      expect(BackupProgressStrings.title(s), BackupProgressStrings.preparing);

      status(SyncStatus.preparingForUpload, total: 4);
      status(SyncStatus.paused, cause: SyncPauseCause.wifi);
      s = tracker.snapshot();
      expect(s.phase, BackupPhase.waitingForWifi);
      expect(BackupProgressStrings.title(s), "Backup paused: waiting for Wi-Fi");

      // Resumed with nothing left to upload: goes away.
      tracker.onResumed();
      status(SyncStatus.completedBackup);
      expect(tracker.snapshot().isVisible, isFalse);
    });
  });

  group("strings", () {
    test("speed", () {
      expect(BackupProgressStrings.speed(512), "512 B/s");
      expect(BackupProgressStrings.speed(850000), "850 KB/s");
      expect(BackupProgressStrings.speed(1234567), "1.2 MB/s");
    });

    test("time left", () {
      expect(
        BackupProgressStrings.timeLeft(const Duration(seconds: 30)),
        "under a minute left",
      );
      expect(
        BackupProgressStrings.timeLeft(const Duration(minutes: 2, seconds: 1)),
        "3 min left",
      );
      expect(
        BackupProgressStrings.timeLeft(const Duration(hours: 1, minutes: 5)),
        "1 h 5 min left",
      );
      expect(
        BackupProgressStrings.timeLeft(const Duration(hours: 2)),
        "2 h left",
      );
    });

    test("one photo", () {
      const s = BackupProgressSnapshot(
        phase: BackupPhase.complete,
        completed: 1,
        total: 1,
      );
      expect(BackupProgressStrings.title(s), "Backup complete: 1 photo");
    });
  });

  group("notification spec", () {
    test("uploading: foreground, progress bar, Pause + Wi-Fi only", () {
      const s = BackupProgressSnapshot(
        phase: BackupPhase.uploading,
        completed: 445,
        total: 2026,
        currentFile: "IMG_2041.HEIC",
        bytesPerSecond: 1200000,
        timeLeft: Duration(minutes: 3),
        wifiOnly: true,
      );
      final spec = BackupProgressService.notificationSpec(s);
      expect(spec["foreground"], isTrue);
      expect(spec["showProgress"], isTrue);
      expect(spec["progress"], 445);
      expect(spec["max"], 2026);
      expect(spec["text"], "IMG_2041.HEIC");
      expect(spec["subText"], "1.2 MB/s · 3 min left");
      final actions = (spec["actions"] as List).cast<Map<String, String>>();
      expect(actions.map((a) => a["id"]), ["pause", "wifi"]);
      expect(actions.last["label"], BackupProgressStrings.wifiOnlyOn);
    });

    test("paused: Resume; error: Retry; neither is foreground", () {
      final paused = BackupProgressService.notificationSpec(
        const BackupProgressSnapshot(phase: BackupPhase.pausedByUser),
      );
      expect(paused["foreground"], isFalse);
      expect(
        (paused["actions"] as List).map((a) => (a as Map)["id"]),
        ["resume", "wifi"],
      );
      final failed = BackupProgressService.notificationSpec(
        const BackupProgressSnapshot(
          phase: BackupPhase.cantReachServer,
          errorSummary: BackupProgressStrings.cantReachServer,
          wifiOnly: false,
        ),
      );
      expect(failed["foreground"], isFalse);
      expect(failed["title"], "Backup paused: can't reach server");
      final actions = (failed["actions"] as List).cast<Map<String, String>>();
      expect(actions.map((a) => a["id"]), ["retry", "wifi"]);
      expect(actions.last["label"], BackupProgressStrings.wifiOnlyOff);
    });

    test("complete auto-dismisses and has no actions", () {
      final spec = BackupProgressService.notificationSpec(
        const BackupProgressSnapshot(
          phase: BackupPhase.complete,
          completed: 2026,
          total: 2026,
        ),
      );
      expect(spec["title"], "Backup complete: 2,026 photos");
      expect(spec["autoDismissMs"], greaterThan(0));
      expect(spec["actions"], isEmpty);
      expect(spec["foreground"], isFalse);
    });

    test("idle clears the notification", () {
      expect(
        BackupProgressService.notificationSpec(const BackupProgressSnapshot()),
        {"phase": "idle"},
      );
    });
  });

  group("Photos banner", () {
    testWidgets("shows progress and pauses", (tester) async {
      final progress = ValueNotifier(const BackupProgressSnapshot());
      var paused = 0;
      var tapped = 0;
      await tester.pumpWidget(
        MaterialApp(
          theme: ComponentTheme.lightTheme(app: ComponentApp.photos),
          home: Scaffold(
            body: BackupProgressBanner(
              progress: progress,
              onTap: () => tapped++,
              onPause: () => paused++,
              onResume: () {},
            ),
          ),
        ),
      );
      expect(find.byKey(const ValueKey("backup-progress-banner")), findsNothing);

      progress.value = const BackupProgressSnapshot(
        phase: BackupPhase.uploading,
        completed: 445,
        total: 2026,
        currentFile: "IMG_2041.HEIC",
      );
      await tester.pump();
      expect(find.text("Backing up 446 of 2,026"), findsOneWidget);
      expect(find.text("IMG_2041.HEIC"), findsOneWidget);
      final bar = tester.widget<LinearProgressIndicator>(
        find.byKey(const ValueKey("backup-progress-bar")),
      );
      expect(bar.value, closeTo(445 / 2026, 1e-9));

      await tester.tap(find.byKey(const ValueKey("backup-progress-pause")));
      expect(paused, 1);
      await tester.tap(find.text("Backing up 446 of 2,026"));
      expect(tapped, 1);

      progress.value = const BackupProgressSnapshot(
        phase: BackupPhase.cantReachServer,
        errorSummary: BackupProgressStrings.cantReachServer,
      );
      await tester.pump();
      expect(find.text("Backup paused: can't reach server"), findsOneWidget);
      expect(
        find.byKey(const ValueKey("backup-progress-resume")),
        findsOneWidget,
      );
    });
  });
}
