-- Core identity tables shared by every workstream. Extend in later migrations.
CREATE TABLE users (
    user_id INTEGER PRIMARY KEY AUTOINCREMENT,
    encrypted_email TEXT NOT NULL,
    email_decryption_nonce TEXT NOT NULL,
    email_hash TEXT NOT NULL UNIQUE,
    creation_time BIGINT NOT NULL,
    storage_quota BIGINT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE tokens (
    token_hash TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    app TEXT NOT NULL DEFAULT 'photos',
    creation_time BIGINT NOT NULL,
    last_used_at BIGINT NOT NULL,
    ip TEXT NOT NULL DEFAULT '',
    user_agent TEXT NOT NULL DEFAULT '',
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX tokens_user_id_idx ON tokens(user_id);
