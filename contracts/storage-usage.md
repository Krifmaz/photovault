# Storage numbers

The apps show the real disk that holds the photos, like Immich: its size, how
much of it is used (by anything), and the account's own photos as a second
line. No plans, no placeholders; a limit only exists when an admin sets a quota.

## Rule (server)

* `disk` = local storage: `statvfs(PHOTOVAULT_STORAGE_PATH)`, cached 10 s, counted like `df`:
  `total = f_blocks * f_frsize`, `used = (f_blocks - f_bfree) * f_frsize`, `available = f_bavail * f_frsize`,
  `usedPercent = used / (used + available)` (root-reserved space counts as full).
  With `PHOTOVAULT_STORAGE_LIMIT` the budget replaces the disk: `total = min(limit, disk total)`,
  `available = min(limit - everything stored, disk available)`, `used = total - available`.
  S3 without `PHOTOVAULT_STORAGE_LIMIT`: no disk.
* `nearlyFull` = `usedPercent >= 90`. The apps warn then.
* `used` = bytes the user has stored (family members: the family's total, as Ente does).
  `userBytes` = the account's own bytes.
* `quota` = per-user quota (admin CLI) else `PHOTOVAULT_DEFAULT_QUOTA`; `0` / unset means none.
* `limit = min(quota, disk total)` (either side dropped when it is "none").
* `free` = `min(quota - used, disk available)`; just the disk's available space without a quota.
* Uploads are refused (426) when larger than the disk's available space or beyond the quota.
* Unlimited (S3 with no `PHOTOVAULT_STORAGE_LIMIT` and no quota) is sent to Ente's endpoints as
  **100 TiB** (`109951162777600`) only because the stock apps need a number. Ente's own 102.4 TB
  placeholder must never appear, and Photovault's apps never show either.

## Ente endpoints: unchanged shapes, real values

* `GET /users/details/v2`: `usage` = used, `subscription.storage` = limit, `subscription.productID` = `"free"`,
  `storageBonus` = 0, `familyData` as today.
* `GET /billing/subscription`: `subscription.storage` = limit.
* `GET /billing/plans/v2`, `/billing/user-plans`: `"plans": []`.

Stock Ente apps read these and show "your photos of the disk size".

## `GET /photovault/usage` (auth: `X-Auth-Token`), additive

For the fork's storage card, web Settings and photovault-core (`storage_usage()`).

```json
{ "usedBytes": 7400000000, "userBytes": 7400000000,
  "limitBytes": 500000000000, "freeBytes": 61600000000,
  "unlimited": false, "limitSource": "disk", "quotaBytes": null,
  "disk": { "totalBytes": 500000000000, "usedBytes": 438400000000, "availableBytes": 61600000000,
            "usedPercent": 87.7, "nearlyFull": false, "source": "disk" },
  "nearlyFullPercent": 90 }
```

`limitSource`: `"quota"`, `"default-quota"`, `"disk"`, `"storage-limit"`, `"unlimited"`.
When `unlimited` is true, `limitBytes` is `null`.

Apps show:

* headline: "`disk.usedBytes` of `disk.totalBytes` used", bar = disk fill, with the account's share highlighted;
* second line: "Your photos: `userBytes`" (with a quota: "Your photos: `usedBytes` of `quotaBytes`");
* a warning when `disk.nearlyFull`: "Server disk is `usedPercent`% full".
* `disk` null (S3, no limit): "Your photos: `userBytes`" alone.

If the endpoint returns 404 (stock museum), or an older Photovault without `disk`, clients fall back to
`/users/details/v2`.
