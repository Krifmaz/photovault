-- Public links, memory shares and cast (mirrors museum migrations 37, 78, 84, 103, 119, 125, 131).
-- The link tables themselves and their device histories (public_collection_tokens,
-- public_file_tokens, public_collection_access_history, public_file_tokens_access_history)
-- are created by 0200_library.

CREATE INDEX public_access_share_id_idx ON public_collection_access_history (share_id);

-- Cast sessions ("casting" because CAST is reserved in Postgres).
CREATE TABLE casting (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL,
    public_key TEXT NOT NULL,
    pq_public_key TEXT,
    collection_id BIGINT,
    cast_user BIGINT,
    encrypted_payload TEXT,
    token TEXT,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    last_used_at BIGINT NOT NULL,
    ip TEXT NOT NULL,
    device_name TEXT
);
CREATE UNIQUE INDEX casting_code_unique_idx ON casting (code) WHERE is_deleted = FALSE;
CREATE INDEX casting_token_idx ON casting (token);
CREATE INDEX casting_cast_user_idx ON casting (cast_user);

-- Memory shares (time-limited public links to a set of files).
CREATE TABLE memory_shares (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    type TEXT NOT NULL CHECK (type IN ('share', 'lane')),
    memory_hash TEXT,
    metadata_cipher TEXT,
    metadata_nonce TEXT,
    mem_enc_key TEXT NOT NULL,
    mem_key_decryption_nonce TEXT NOT NULL,
    access_token TEXT NOT NULL UNIQUE,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX memory_shares_user_id_idx ON memory_shares (user_id);

CREATE TABLE memory_share_files (
    id BIGSERIAL PRIMARY KEY,
    memory_share_id BIGINT NOT NULL REFERENCES memory_shares(id) ON DELETE CASCADE,
    file_id BIGINT NOT NULL,
    file_owner_id BIGINT NOT NULL,
    position BIGINT NOT NULL,
    file_enc_key TEXT NOT NULL,
    file_key_decryption_nonce TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    UNIQUE (memory_share_id, position)
);
