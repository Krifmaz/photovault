-- Encrypted contact cards and their attachments, mirroring museum's contact_entity / user_attachments
-- (single-store: no replication bucket arrays). Byte fields are stored as base64 TEXT.
CREATE TABLE contact_entity (
    id TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    contact_user_id BIGINT NOT NULL,
    profile_picture_attachment_id TEXT,
    encrypted_key TEXT NOT NULL,
    encrypted_data TEXT,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CONSTRAINT contact_entity_state_constraint CHECK (
        (is_deleted = FALSE AND encrypted_data IS NOT NULL) OR
        (is_deleted = TRUE AND encrypted_data IS NULL AND profile_picture_attachment_id IS NULL))
);
CREATE INDEX idx_contact_entity_user_updated_at ON contact_entity (user_id, updated_at);
CREATE UNIQUE INDEX idx_contact_entity_user_contact_user_id ON contact_entity (user_id, contact_user_id);
CREATE INDEX idx_contact_entity_contact_user_id ON contact_entity (contact_user_id);

CREATE TABLE user_attachments (
    attachment_id TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    attachment_type TEXT NOT NULL,
    size BIGINT NOT NULL,
    latest_bucket TEXT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    pending_sync BOOLEAN NOT NULL DEFAULT TRUE,
    sync_locked_till BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX idx_user_attachments_lookup ON user_attachments (user_id, attachment_type, is_deleted);
CREATE INDEX idx_user_attachments_deleted ON user_attachments (is_deleted);

-- Signed upload URLs handed out but not yet committed (museum's temp_objects rows for attachments).
CREATE TABLE user_attachment_uploads (
    object_key TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    attachment_type TEXT NOT NULL,
    attachment_id TEXT NOT NULL,
    content_length BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);
CREATE INDEX idx_user_attachment_uploads_expires_at ON user_attachment_uploads (expires_at);
