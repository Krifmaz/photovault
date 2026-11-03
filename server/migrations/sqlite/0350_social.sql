-- Comments, reactions and anonymous (public-link) identities. Mirrors museum's
-- 114_comments_reactions, with trigger logic moved into application code so the
-- same schema works on SQLite. Ids are app/server supplied nanoid strings
-- (`cmt_…`, `rct_…`, `anon_…`). cipher/nonce are base64 ciphertext produced by
-- the apps with the collection key; the server never sees plaintext.

CREATE TABLE anon_users (
    id TEXT PRIMARY KEY,
    collection_id BIGINT NOT NULL,
    cipher TEXT NOT NULL,
    nonce TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX idx_anon_users_collection ON anon_users (collection_id);

CREATE TABLE comments (
    id TEXT PRIMARY KEY,
    collection_id BIGINT NOT NULL,
    file_id BIGINT NULL,
    parent_comment_id TEXT NULL,
    user_id BIGINT NOT NULL,
    anon_user_id TEXT NULL REFERENCES anon_users (id),
    cipher TEXT NULL,
    nonce TEXT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CONSTRAINT comments_cipher_on_delete CHECK (
        (is_deleted = FALSE AND cipher IS NOT NULL AND nonce IS NOT NULL) OR
        (is_deleted = TRUE AND cipher IS NULL AND nonce IS NULL)
    ),
    CONSTRAINT comments_user_or_anon_chk CHECK (
        (user_id = -1 AND anon_user_id IS NOT NULL) OR
        (user_id > 0 AND anon_user_id IS NULL)
    )
);
CREATE INDEX idx_comments_scope ON comments (collection_id, file_id) WHERE is_deleted = FALSE;
CREATE INDEX idx_comments_parent ON comments (parent_comment_id) WHERE is_deleted = FALSE;
CREATE INDEX idx_comments_user ON comments (user_id) WHERE is_deleted = FALSE;
CREATE INDEX idx_comments_anon ON comments (anon_user_id) WHERE is_deleted = FALSE;
CREATE INDEX idx_comments_collection_updated_at ON comments (collection_id, updated_at);

-- actor_key / unique_key are generated columns in museum; here the server
-- computes them ('U:<user_id>' | 'A:<anon_user_id>', and
-- 'M:<comment_id>' | 'F:<collection_id>/<file_id>' | 'C:<collection_id>').
CREATE TABLE reactions (
    id TEXT PRIMARY KEY,
    collection_id BIGINT NOT NULL,
    file_id BIGINT NULL,
    comment_id TEXT NULL,
    user_id BIGINT NOT NULL,
    anon_user_id TEXT NULL REFERENCES anon_users (id),
    actor_key TEXT NOT NULL,
    unique_key TEXT NOT NULL,
    cipher TEXT NULL,
    nonce TEXT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CONSTRAINT reactions_target_shape CHECK (comment_id IS NULL OR file_id IS NULL),
    CONSTRAINT reactions_cipher_on_delete CHECK (
        (is_deleted = FALSE AND cipher IS NOT NULL AND nonce IS NOT NULL) OR
        (is_deleted = TRUE AND cipher IS NULL AND nonce IS NULL)
    ),
    CONSTRAINT uq_reactions_actor_target UNIQUE (actor_key, unique_key),
    CONSTRAINT reactions_user_or_anon_chk CHECK (
        (user_id = -1 AND anon_user_id IS NOT NULL) OR
        (user_id > 0 AND anon_user_id IS NULL)
    )
);
CREATE INDEX idx_reactions_scope ON reactions (collection_id, file_id, comment_id) WHERE is_deleted = FALSE;
CREATE INDEX idx_reactions_collection_updated_at ON reactions (collection_id, updated_at);
CREATE INDEX idx_reactions_comment ON reactions (comment_id) WHERE is_deleted = FALSE;
CREATE INDEX idx_reactions_user ON reactions (user_id) WHERE is_deleted = FALSE;
CREATE INDEX idx_reactions_anon ON reactions (anon_user_id) WHERE is_deleted = FALSE;
