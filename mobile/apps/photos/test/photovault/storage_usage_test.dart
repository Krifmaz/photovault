// PHOTOVAULT: real disk on the storage card.

import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/storage_usage.dart";

const _gib = 1024 * 1024 * 1024;

Map<String, Object?> _usage({
  int used = 0,
  int? user,
  int? quota,
  Map<String, Object?>? disk,
}) => {
  "usedBytes": used,
  "userBytes": user ?? used,
  "limitBytes": disk?["totalBytes"],
  "freeBytes": disk?["availableBytes"],
  "unlimited": disk == null && quota == null,
  "limitSource": "disk",
  "quotaBytes": quota,
  "disk": disk,
};

Map<String, Object?> _disk(int total, int used, int available) => {
  "totalBytes": total,
  "usedBytes": used,
  "availableBytes": available,
  "usedPercent": used / (used + available) * 100,
  "nearlyFull": used / (used + available) * 100 >= 90,
  "source": "disk",
};

void main() {
  test("shows the whole disk and the account's photos separately", () {
    final u = PhotovaultStorageUsage.fromJson(
      _usage(
        used: (7.4 * _gib).round(),
        disk: _disk(500 * _gib, 300 * _gib, 200 * _gib),
      ),
    )!;
    final v = StorageCardView.from(u);
    expect(v.headline, "300 GB of 500 GB used");
    expect(v.detail, "Your photos: 7.4 GB");
    expect(v.diskFraction, closeTo(0.6, 1e-9));
    expect(v.userFraction, closeTo(7.4 / 500, 1e-3));
    expect(v.warning, isNull);
  });

  test("warns from 90% full", () {
    final full = StorageCardView.from(
      PhotovaultStorageUsage.fromJson(
        _usage(disk: _disk(100 * _gib, 93 * _gib, 7 * _gib)),
      )!,
    );
    expect(full.warning, "Server disk is 93% full");
    final edge = StorageCardView.from(
      PhotovaultStorageUsage.fromJson(
        _usage(disk: _disk(100 * _gib, 90 * _gib, 10 * _gib)),
      )!,
    );
    expect(edge.warning, "Server disk is 90% full");
    final roomy = StorageCardView.from(
      PhotovaultStorageUsage.fromJson(
        _usage(disk: _disk(100 * _gib, 89 * _gib, 11 * _gib)),
      )!,
    );
    expect(roomy.warning, isNull);
  });

  test("an admin quota is shown on the second line", () {
    final v = StorageCardView.from(
      PhotovaultStorageUsage.fromJson(
        _usage(
          used: 2 * _gib,
          quota: 50 * _gib,
          disk: _disk(500 * _gib, 100 * _gib, 400 * _gib),
        ),
      )!,
    );
    expect(v.headline, "100 GB of 500 GB used");
    expect(v.detail, "Your photos: 2 GB of 50 GB");
  });

  test("no disk (S3 without a limit): only the account's photos", () {
    final v = StorageCardView.from(
      PhotovaultStorageUsage.fromJson(_usage(used: 3 * _gib))!,
    );
    expect(v.headline, "3 GB used");
    expect(v.detail, "Your photos: 3 GB");
    expect(v.warning, isNull);
    expect(v.diskFraction, 0);
  });

  test("an older server without disk or userBytes still parses", () {
    final u = PhotovaultStorageUsage.fromJson({
      "usedBytes": 5,
      "limitBytes": 10,
      "freeBytes": 5,
      "unlimited": false,
      "limitSource": "disk",
      "quotaBytes": null,
    })!;
    expect(u.disk, isNull);
    expect(u.userBytes, 5);
    expect(PhotovaultStorageUsage.fromJson("nope"), isNull);
    expect(PhotovaultStorageUsage.fromJson({"x": 1}), isNull);
  });

  test("nearlyFull falls back to usedPercent when missing", () {
    final d = PhotovaultDisk.fromJson({
      "totalBytes": 100,
      "usedBytes": 95,
      "availableBytes": 5,
    })!;
    expect(d.nearlyFull, isTrue);
    expect(d.usedPercent, 95);
  });

  test("formats bytes like the rest of the app", () {
    expect(formatStorageBytes(0), "0 bytes");
    expect(formatStorageBytes(512 * 1024 * 1024), "512 MB");
    expect(formatStorageBytes((7.4 * _gib).round()), "7.4 GB");
    expect(formatStorageBytes(69 * _gib), "69 GB");
    expect(formatStorageBytes(500 * _gib), "500 GB");
    expect(formatStorageBytes((1.8 * 1024 * _gib).round()), "1.8 TB");
  });
}
