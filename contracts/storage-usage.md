# Storage numbers

## Rule (server)

* `used` = bytes the user has stored (family members: the family's total, as Ente does).
* `free` = local storage: `statvfs(PHOTOVAULT_STORAGE_PATH)`: `f_bavail * f_frsize`, cached 10 s.
  S3: `PHOTOVAULT_STORAGE_LIMIT - used` if set, else unlimited.
* `quota` = per-user quota (admin CLI) else `PHOTOVAULT_DEFAULT_QUOTA`; `0` / unset means none.
* `limit = min(quota, used + free)` (either side dropped when it is "none"/unlimited).
* Unlimited (S3 with no `PHOTOVAULT_STORAGE_LIMIT` and no quota) is sent to Ente's endpoints as
  **100 TiB** (`109951162777600`). Ente's own 102.4 TB placeholder must never appear.

## Ente endpoints: unchanged shapes, real values

* `GET /users/details/v2`: `usage` = used, `subscription.storage` = limit, `subscription.productID` = `"free"`,
  `storageBonus` = 0, `familyData` as today.
* `GET /billing/subscription`: `subscription.storage` = limit.
* `GET /billing/plans/v2`, `/billing/user-plans`: `"plans": []`.

Stock Ente apps and the fork read these, so "X of Y" is right everywhere with no client change.

## `GET /photovault/usage` (auth: `X-Auth-Token`), additive

For the fork's storage card, web Settings and photovault-core (`storage_usage()`); same numbers, explicit.

```json
{ "usedBytes": 52428800000, "limitBytes": 3800000000000, "freeBytes": 3747571200000,
  "unlimited": false, "limitSource": "disk", "quotaBytes": null }
```

`limitSource`: `"quota"`, `"default-quota"`, `"disk"`, `"storage-limit"`, `"unlimited"`.
When `unlimited` is true, `limitBytes` is `null`. Apps show "`used` of `limit`" (or "`used` used") with no plan names.
If the endpoint returns 404 (stock museum), clients fall back to `/users/details/v2`.
