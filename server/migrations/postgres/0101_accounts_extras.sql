-- Custom domains: canonical (IDNA ASCII, lower-case) value, unique across users
-- (museum remote_store.canonical_value + remote_store_custom_domain_canonical_unique_idx).
ALTER TABLE remote_store ADD COLUMN canonical_value TEXT;
CREATE UNIQUE INDEX remote_store_custom_domain_canonical_unique_idx ON remote_store(canonical_value) WHERE key_name = 'customDomain';

-- Admins granted with `photovault admin mark-admin` (in addition to config `admins`).
ALTER TABLE users ADD COLUMN is_admin BOOLEAN NOT NULL DEFAULT FALSE;

-- Column the legacy-kit SRP code expects (museum srp_sessions.updated_at).
ALTER TABLE srp_sessions ADD COLUMN updated_at BIGINT NOT NULL DEFAULT 0;
