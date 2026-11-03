-- Legacy kits, mirroring museum's legacy_kit* tables (UUIDs as TEXT, JSON as TEXT).
CREATE TABLE legacy_kit (
    id TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    -- 1 = 2-of-3 shares.
    variant BIGINT NOT NULL CHECK (variant IN (1)),
    notice_period_in_hrs BIGINT NOT NULL,
    -- Base64(secretbox nonce || MAC || ciphertext) of the user's recovery key.
    encrypted_recovery_blob TEXT NOT NULL,
    -- Base64(X25519 public key) derived from the kit secret.
    auth_public_key TEXT NOT NULL,
    -- Base64(secretbox ...) of owner-only part names and share payloads.
    encrypted_owner_blob TEXT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    deleted_at BIGINT
);
CREATE INDEX idx_legacy_kit_user_id ON legacy_kit(user_id);

CREATE TABLE legacy_kit_challenge (
    id TEXT PRIMARY KEY NOT NULL,
    kit_id TEXT NOT NULL REFERENCES legacy_kit(id) ON DELETE CASCADE,
    -- Base64url(SHA-256(challenge)).
    challenge_hash TEXT NOT NULL,
    expires_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);
CREATE INDEX idx_legacy_kit_challenge_kit_id ON legacy_kit_challenge(kit_id);
CREATE INDEX idx_legacy_kit_challenge_expires_at ON legacy_kit_challenge(expires_at);

CREATE TABLE legacy_kit_recovery_session (
    id TEXT PRIMARY KEY NOT NULL,
    kit_id TEXT NOT NULL REFERENCES legacy_kit(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('WAITING', 'READY', 'BLOCKED', 'CANCELLED', 'RECOVERED')),
    effective_notice_period_in_hrs BIGINT NOT NULL,
    wait_till BIGINT NOT NULL,
    next_reminder_at BIGINT,
    -- JSON array of {usedPartIndexes, ip, userAgent} audit hints.
    initiators TEXT NOT NULL DEFAULT '[]',
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE UNIQUE INDEX idx_legacy_kit_recovery_active_per_kit
    ON legacy_kit_recovery_session(kit_id) WHERE status IN ('WAITING', 'READY');
CREATE INDEX idx_legacy_kit_recovery_user_id ON legacy_kit_recovery_session(user_id);

CREATE TABLE legacy_kit_recovery_session_token (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES legacy_kit_recovery_session(id) ON DELETE CASCADE,
    -- Base64url(SHA-256(raw session token)).
    token_hash TEXT NOT NULL UNIQUE,
    created_at BIGINT NOT NULL
);
CREATE INDEX idx_legacy_kit_recovery_session_token_session_id
    ON legacy_kit_recovery_session_token(session_id);
