-- Per-user encrypted entities (people, location tags, …), mirroring museum's entity_key / entity_data.
CREATE TABLE entity_key (
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    encrypted_key TEXT NOT NULL,
    header TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, type)
);

CREATE TABLE entity_data (
    id TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL,
    type TEXT NOT NULL,
    encrypted_data TEXT,
    header TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    CONSTRAINT fk_entity_key_user_id_and_type FOREIGN KEY (user_id, type)
        REFERENCES entity_key (user_id, type) ON DELETE CASCADE,
    CONSTRAINT entity_data_state_constraint CHECK (
        (is_deleted = TRUE AND encrypted_data IS NULL) OR (is_deleted = FALSE AND encrypted_data IS NOT NULL))
);
CREATE INDEX entity_data_updated_at_time_index ON entity_data (user_id, updated_at);
