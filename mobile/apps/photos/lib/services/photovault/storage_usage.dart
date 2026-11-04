// PHOTOVAULT: real disk on the storage card (new file).
//
// `GET /photovault/usage` (contracts/storage-usage.md) and what the storage
// card shows from it: the disk that holds the photos ("438 GB of 500 GB
// used"), the account's own photos as a second line, and a warning from 90%
// full. Pure Dart; unit-tested in test/photovault/storage_usage_test.dart.

/// The filesystem holding the photos (or the server's configured budget).
class PhotovaultDisk {
  const PhotovaultDisk({
    required this.totalBytes,
    required this.usedBytes,
    required this.availableBytes,
    required this.usedPercent,
    required this.nearlyFull,
  });

  final int totalBytes;
  final int usedBytes;
  final int availableBytes;
  final double usedPercent;
  final bool nearlyFull;

  static PhotovaultDisk? fromJson(Object? json) {
    if (json is! Map) return null;
    final total = _int(json["totalBytes"]);
    final used = _int(json["usedBytes"]);
    final available = _int(json["availableBytes"]);
    if (total == null || used == null || available == null || total <= 0) {
      return null;
    }
    final percent =
        _num(json["usedPercent"])?.toDouble() ??
        (used + available > 0 ? used / (used + available) * 100 : 0.0);
    return PhotovaultDisk(
      totalBytes: total,
      usedBytes: used,
      availableBytes: available,
      usedPercent: percent,
      nearlyFull:
          json["nearlyFull"] is bool
              ? json["nearlyFull"] as bool
              : percent >= PhotovaultStorageUsage.defaultNearlyFullPercent,
    );
  }
}

/// `GET /photovault/usage`.
class PhotovaultStorageUsage {
  const PhotovaultStorageUsage({
    required this.usedBytes,
    required this.userBytes,
    required this.quotaBytes,
    required this.disk,
  });

  static const defaultNearlyFullPercent = 90.0;

  /// Stored by the account (family members: the family's total).
  final int usedBytes;

  /// Stored by this account alone ("Your photos").
  final int userBytes;

  /// The quota an admin set, or null.
  final int? quotaBytes;

  /// Null on S3 without a configured limit, and on older servers.
  final PhotovaultDisk? disk;

  static PhotovaultStorageUsage? fromJson(Object? json) {
    if (json is! Map) return null;
    final used = _int(json["usedBytes"]);
    if (used == null) return null;
    final quota = _int(json["quotaBytes"]);
    return PhotovaultStorageUsage(
      usedBytes: used,
      userBytes: _int(json["userBytes"]) ?? used,
      quotaBytes: quota != null && quota > 0 ? quota : null,
      disk: PhotovaultDisk.fromJson(json["disk"]),
    );
  }
}

/// The text and bar of the storage card.
class StorageCardView {
  const StorageCardView({
    required this.headline,
    required this.detail,
    required this.diskFraction,
    required this.userFraction,
    required this.warning,
  });

  /// "438 GB of 500 GB used", or "7.4 GB used" without a disk.
  final String headline;

  /// "Your photos: 7.4 GB" (with a quota: "Your photos: 7.4 GB of 50 GB").
  final String detail;

  /// Bar: the whole disk's fill, 0..1.
  final double diskFraction;

  /// Bar: this account's share of the disk, 0..1.
  final double userFraction;

  /// "Server disk is 93% full", or null below 90%.
  final String? warning;

  factory StorageCardView.from(PhotovaultStorageUsage u) {
    final disk = u.disk;
    final mine = u.quotaBytes != null ? u.usedBytes : u.userBytes;
    final detail =
        u.quotaBytes != null
            ? "Your photos: ${formatStorageBytes(mine)} of ${formatStorageBytes(u.quotaBytes!)}"
            : "Your photos: ${formatStorageBytes(mine)}";
    if (disk == null) {
      final q = u.quotaBytes;
      return StorageCardView(
        headline:
            q != null
                ? "${formatStorageBytes(u.usedBytes)} of ${formatStorageBytes(q)} used"
                : "${formatStorageBytes(u.usedBytes)} used",
        detail: detail,
        diskFraction: q != null ? _fraction(u.usedBytes, q) : 0,
        userFraction: q != null ? _fraction(u.usedBytes, q) : 0,
        warning: null,
      );
    }
    return StorageCardView(
      headline:
          "${formatStorageBytes(disk.usedBytes)} of ${formatStorageBytes(disk.totalBytes)} used",
      detail: detail,
      diskFraction: _fraction(disk.usedBytes, disk.totalBytes),
      userFraction: _fraction(u.userBytes, disk.totalBytes),
      warning:
          disk.nearlyFull
              ? "Server disk is ${disk.usedPercent.floor()}% full"
              : null,
    );
  }
}

/// Bytes the way `df -h` and the rest of the app count them (1024-based,
/// labelled GB/TB): "512 MB", "7.4 GB", "1.8 TB".
String formatStorageBytes(int bytes) {
  const units = ["bytes", "KB", "MB", "GB", "TB", "PB"];
  if (bytes < 1024) return "${bytes < 0 ? 0 : bytes} bytes";
  double v = bytes.toDouble();
  int i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  final text =
      v >= 100 || i <= 2
          ? v.round().toString()
          : v.toStringAsFixed(1).replaceAll(RegExp(r"\.0$"), "");
  return "$text ${units[i]}";
}

double _fraction(int part, int whole) =>
    whole <= 0 ? 0 : (part / whole).clamp(0.0, 1.0).toDouble();

int? _int(Object? v) => v is num ? v.toInt() : null;

num? _num(Object? v) => v is num ? v : null;
