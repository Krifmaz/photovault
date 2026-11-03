-- Auth app (TOTP) entities, mirroring museum's authenticator_key / authenticator_entity.
CREATE TABLE authenticator_key (
    user_id BIGINT PRIMARY KEY NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    encrypted_key TEXT NOT NULL,
    header TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE TABLE authenticator_entity (
    id TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES authenticator_key(user_id) ON DELETE CASCADE,
    encrypted_data TEXT,
    header TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    CONSTRAINT authenticator_entity_state_constraint CHECK (
        (is_deleted = TRUE AND encrypted_data IS NULL) OR (is_deleted = FALSE AND encrypted_data IS NOT NULL))
);
CREATE INDEX authenticator_entity_updated_at_time_index ON authenticator_entity (user_id, updated_at);
