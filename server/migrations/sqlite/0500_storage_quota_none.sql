-- Photovault: `storage_quota = 0` now means "no quota" (the storage backend's free
-- space is the limit). Accounts still on the old fixed 100 TiB default move to that.
UPDATE users SET storage_quota = 0 WHERE storage_quota = 109951162777600;
