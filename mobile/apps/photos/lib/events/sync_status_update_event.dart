import "package:logging/logging.dart";
import 'package:photos/events/event.dart';

class SyncStatusUpdate extends Event {
  static final _logger = Logger("SyncStatusUpdate");

  final SyncStatus status;
  final int? completed;
  final int? total;
  final bool wasStopped;
  @override
  final String reason;
  final Error? error;

  /// PHOTOVAULT: why a [SyncStatus.paused] backup paused.
  final SyncPauseCause? pauseCause;

  /// PHOTOVAULT: full error text for the Backup status screen.
  final String? details;
  late int timestamp;

  SyncStatusUpdate(
    this.status, {
    this.completed,
    this.total,
    this.wasStopped = false,
    this.reason = "",
    this.error,
    this.pauseCause,
    this.details,
  }) {
    _logger.info("Creating sync status: " + status.toString());
    timestamp = DateTime.now().microsecondsSinceEpoch;
  }
}

enum SyncStatus {
  startedFirstGalleryImport,
  completedFirstGalleryImport,
  applyingRemoteDiff,
  preparingForUpload,
  inProgress,
  paused,
  completedBackup,
  error,
}

/// PHOTOVAULT: backup progress notification.
enum SyncPauseCause { user, wifi, network }
