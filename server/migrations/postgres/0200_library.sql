-- Files, collections, sharing, trash, object bookkeeping and file data.
-- Table and column names mirror museum so other workstreams can rely on them.

CREATE TABLE collections (
    collection_id BIGSERIAL PRIMARY KEY,
    owner_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    encrypted_key TEXT NOT NULL,
    key_decryption_nonce TEXT NOT NULL,
    name TEXT,
    encrypted_name TEXT,
    name_decryption_nonce TEXT,
    type TEXT NOT NULL,
    attributes TEXT NOT NULL DEFAULT '{}',
    updation_time BIGINT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    magic_metadata TEXT,
    pub_magic_metadata TEXT,
    app TEXT NOT NULL DEFAULT 'photos'
);
CREATE INDEX collections_owner_id_updation_time_idx ON collections(owner_id, updation_time);
CREATE UNIQUE INDEX collections_favorites_constraint_index_v2 ON collections(owner_id, app) WHERE type = 'favorites';
CREATE UNIQUE INDEX collections_uncategorized_constraint_index_v2 ON collections(owner_id, app) WHERE type = 'uncategorized';

CREATE TABLE collection_shares (
    collection_id BIGINT NOT NULL REFERENCES collections(collection_id) ON DELETE CASCADE,
    from_user_id BIGINT NOT NULL,
    to_user_id BIGINT NOT NULL,
    encrypted_key TEXT NOT NULL,
    updation_time BIGINT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    role_type TEXT NOT NULL DEFAULT 'VIEWER',
    magic_metadata TEXT,
    shared_at BIGINT,
    CONSTRAINT unique_collection_shares_cid_from_to UNIQUE (collection_id, from_user_id, to_user_id)
);
CREATE INDEX collection_shares_to_user_id_idx ON collection_shares(to_user_id);

CREATE TABLE files (
    file_id BIGSERIAL PRIMARY KEY,
    owner_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    app TEXT,
    encrypted_metadata TEXT NOT NULL,
    file_decryption_header TEXT NOT NULL,
    thumbnail_decryption_header TEXT NOT NULL,
    metadata_decryption_header TEXT NOT NULL,
    magic_metadata TEXT,
    pub_magic_metadata TEXT,
    info TEXT,
    updation_time BIGINT NOT NULL
);
CREATE INDEX files_owner_id_idx ON files(owner_id);

CREATE TABLE collection_files (
    collection_id BIGINT NOT NULL REFERENCES collections(collection_id) ON DELETE CASCADE,
    file_id BIGINT NOT NULL REFERENCES files(file_id) ON DELETE CASCADE,
    encrypted_key TEXT NOT NULL,
    key_decryption_nonce TEXT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    updation_time BIGINT NOT NULL,
    c_owner_id BIGINT,
    f_owner_id BIGINT,
    created_at BIGINT,
    action_user BIGINT,
    action TEXT,
    CONSTRAINT unique_collection_files_cid_fid UNIQUE (collection_id, file_id)
);
CREATE INDEX collection_files_cid_updation_time_idx ON collection_files(collection_id, updation_time);
CREATE INDEX collection_files_file_id_idx ON collection_files(file_id);

CREATE TABLE object_keys (
    file_id BIGINT NOT NULL REFERENCES files(file_id) ON DELETE CASCADE,
    o_type TEXT NOT NULL,
    object_key TEXT NOT NULL,
    size BIGINT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (file_id, o_type),
    CONSTRAINT object_keys_object_key_key UNIQUE (object_key)
);

CREATE TABLE trash (
    file_id BIGINT PRIMARY KEY REFERENCES files(file_id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL,
    collection_id BIGINT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    is_restored BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    delete_by BIGINT NOT NULL
);
CREATE INDEX trash_user_id_updated_at_idx ON trash(user_id, updated_at);
CREATE INDEX trash_delete_by_idx ON trash(delete_by);

CREATE TABLE temp_objects (
    object_key TEXT NOT NULL,
    expiration_time BIGINT NOT NULL,
    upload_id TEXT,
    is_multipart BOOLEAN NOT NULL DEFAULT FALSE,
    bucket_id TEXT,
    user_id BIGINT,
    app TEXT,
    purpose TEXT,
    content_length BIGINT,
    content_md5 TEXT,
    client TEXT,
    created_at BIGINT NOT NULL
);
CREATE INDEX temp_objects_object_key_idx ON temp_objects(object_key);
CREATE INDEX temp_objects_expiration_time_idx ON temp_objects(expiration_time);

CREATE TABLE queue (
    queue_id BIGSERIAL PRIMARY KEY,
    queue_name TEXT NOT NULL,
    item TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    CONSTRAINT unique_queue_name_item UNIQUE (queue_name, item)
);

CREATE TABLE IF NOT EXISTS usage (
    user_id BIGINT PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
    storage_consumed BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE file_data (
    file_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    data_type TEXT NOT NULL,
    size BIGINT NOT NULL,
    latest_bucket TEXT NOT NULL,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    pending_sync BOOLEAN NOT NULL DEFAULT TRUE,
    sync_locked_till BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    obj_id TEXT,
    obj_nonce TEXT,
    obj_size BIGINT,
    PRIMARY KEY (file_id, data_type)
);
CREATE INDEX file_data_user_id_updated_at_idx ON file_data(user_id, updated_at);

CREATE TABLE public_collection_tokens (
    id BIGSERIAL PRIMARY KEY,
    collection_id BIGINT NOT NULL REFERENCES collections(collection_id) ON DELETE CASCADE,
    access_token TEXT NOT NULL,
    is_disabled BOOLEAN NOT NULL DEFAULT FALSE,
    valid_till BIGINT NOT NULL DEFAULT 0,
    device_limit BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    pw_hash TEXT,
    pw_nonce TEXT,
    mem_limit BIGINT,
    ops_limit BIGINT,
    enable_download BOOLEAN NOT NULL DEFAULT TRUE,
    enable_comment BOOLEAN NOT NULL DEFAULT FALSE,
    enable_collect BOOLEAN NOT NULL DEFAULT FALSE,
    enable_join BOOLEAN NOT NULL DEFAULT TRUE,
    min_role TEXT
);
CREATE UNIQUE INDEX public_active_collection_unique_idx ON public_collection_tokens(collection_id) WHERE is_disabled = FALSE;
CREATE UNIQUE INDEX public_access_tokens_unique_idx ON public_collection_tokens(access_token);

CREATE TABLE public_collection_access_history (
    share_id BIGINT NOT NULL REFERENCES public_collection_tokens(id) ON DELETE CASCADE,
    ip TEXT NOT NULL,
    user_agent TEXT NOT NULL,
    created_at BIGINT NOT NULL DEFAULT 0,
    CONSTRAINT unique_access_sid_ip_ua UNIQUE (share_id, ip, user_agent)
);

CREATE TABLE public_file_tokens (
    id TEXT PRIMARY KEY,
    file_id BIGINT NOT NULL REFERENCES files(file_id) ON DELETE CASCADE,
    owner_id BIGINT NOT NULL,
    app TEXT NOT NULL,
    access_token TEXT NOT NULL,
    valid_till BIGINT NOT NULL DEFAULT 0,
    device_limit BIGINT NOT NULL DEFAULT 0,
    is_disabled BOOLEAN NOT NULL DEFAULT FALSE,
    enable_download BOOLEAN NOT NULL DEFAULT TRUE,
    pw_hash TEXT,
    pw_nonce TEXT,
    mem_limit BIGINT,
    ops_limit BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    encrypted_file_key TEXT,
    encrypted_file_key_nonce TEXT,
    kdf_nonce TEXT,
    kdf_mem_limit BIGINT,
    kdf_ops_limit BIGINT,
    encrypted_share_key TEXT
);
CREATE UNIQUE INDEX public_file_token_unique_idx ON public_file_tokens(access_token) WHERE is_disabled = FALSE;
CREATE INDEX public_file_tokens_owner_id_updated_at_idx ON public_file_tokens(owner_id, updated_at);
CREATE UNIQUE INDEX public_active_file_link_unique_idx ON public_file_tokens(file_id) WHERE is_disabled = FALSE;

CREATE TABLE public_file_tokens_access_history (
    id TEXT NOT NULL REFERENCES public_file_tokens(id) ON DELETE CASCADE,
    ip TEXT NOT NULL,
    user_agent TEXT NOT NULL,
    created_at BIGINT NOT NULL DEFAULT 0,
    CONSTRAINT unique_access_id_ip_ua UNIQUE (id, ip, user_agent)
);

CREATE TABLE collection_actions (
    id TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    actor_user_id BIGINT NOT NULL,
    collection_id BIGINT NOT NULL,
    file_id BIGINT,
    data TEXT,
    action TEXT NOT NULL,
    is_pending BOOLEAN NOT NULL DEFAULT TRUE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX collection_actions_user_id_updated_at_idx ON collection_actions(user_id, updated_at);
CREATE INDEX collection_actions_collection_id_idx ON collection_actions(collection_id);
