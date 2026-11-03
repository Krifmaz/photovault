-- Trusted contacts ("Legacy"), mirroring museum's emergency_contact / emergency_recovery.
CREATE TABLE emergency_contact (
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    emergency_contact_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    state TEXT NOT NULL CHECK (state IN ('INVITED', 'ACCEPTED', 'REVOKED', 'DELETED', 'CONTACT_LEFT', 'CONTACT_DENIED')),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    notice_period_in_hrs BIGINT NOT NULL,
    -- Owner's recovery key encrypted to the contact's public key (opaque ciphertext).
    encrypted_key TEXT,
    CONSTRAINT chk_user_id_not_equal_emergency_contact_id CHECK (user_id <> emergency_contact_id),
    CONSTRAINT chk_encrypted_key_null CHECK (
        (state IN ('REVOKED', 'DELETED', 'CONTACT_LEFT', 'CONTACT_DENIED') AND encrypted_key IS NULL) OR
        (state NOT IN ('REVOKED', 'DELETED', 'CONTACT_LEFT', 'CONTACT_DENIED') AND encrypted_key IS NOT NULL)),
    CONSTRAINT unique_user_emergency_contact UNIQUE (user_id, emergency_contact_id)
);
CREATE INDEX idx_emergency_contact_id ON emergency_contact(emergency_contact_id);

CREATE TABLE emergency_recovery (
    id TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    emergency_contact_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('WAITING', 'REJECTED', 'RECOVERED', 'STOPPED', 'READY')),
    wait_till BIGINT,
    next_reminder_at BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE UNIQUE INDEX idx_emergency_recovery_limit_active_recovery
    ON emergency_recovery(user_id, emergency_contact_id, status) WHERE status IN ('WAITING', 'READY');
CREATE INDEX idx_emergency_recovery_user_id ON emergency_recovery(user_id);
CREATE INDEX idx_emergency_recovery_contact_id ON emergency_recovery(emergency_contact_id);
CREATE INDEX idx_emergency_recovery_next_reminder_at ON emergency_recovery(next_reminder_at);
