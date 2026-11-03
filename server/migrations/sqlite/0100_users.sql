-- Users & accounts workstream (range 0100-0199). Mirrors museum's table and column names.

-- Extra user columns (museum: is_two_factor_enabled, email_mfa, family_admin_id, source,
-- delete_feedback). Self-hosted subscription data lives on the user row; storage = storage_quota.
ALTER TABLE users ADD COLUMN is_two_factor_enabled BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE users ADD COLUMN email_mfa BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE users ADD COLUMN family_admin_id BIGINT;
ALTER TABLE users ADD COLUMN source TEXT;
ALTER TABLE users ADD COLUMN delete_feedback TEXT;
ALTER TABLE users ADD COLUMN subscription_product_id TEXT NOT NULL DEFAULT 'free';
ALTER TABLE users ADD COLUMN subscription_expiry BIGINT NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN subscription_payment_provider TEXT NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN subscription_transaction_id TEXT NOT NULL DEFAULT 'none';
ALTER TABLE users ADD COLUMN subscription_attributes TEXT NOT NULL DEFAULT '{}';
CREATE INDEX users_family_admin_id_idx ON users(family_admin_id);

CREATE TABLE key_attributes (
    user_id BIGINT PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
    kek_salt TEXT NOT NULL,
    encrypted_key TEXT NOT NULL,
    key_decryption_nonce TEXT NOT NULL,
    public_key TEXT NOT NULL,
    encrypted_secret_key TEXT NOT NULL,
    secret_key_decryption_nonce TEXT NOT NULL,
    mem_limit BIGINT NOT NULL,
    ops_limit BIGINT NOT NULL,
    master_key_encrypted_with_recovery_key TEXT,
    master_key_decryption_nonce TEXT,
    recovery_key_encrypted_with_master_key TEXT,
    recovery_key_decryption_nonce TEXT,
    creation_time BIGINT NOT NULL
);

CREATE TABLE otts (
    email_hash TEXT NOT NULL,
    ott TEXT NOT NULL,
    app TEXT NOT NULL DEFAULT 'photos',
    creation_time BIGINT NOT NULL,
    expiration_time BIGINT NOT NULL,
    wrong_attempt BIGINT NOT NULL DEFAULT 0,
    CONSTRAINT unique_otts_emailhash_app_ott UNIQUE (email_hash, app, ott)
);

CREATE TABLE two_factor (
    user_id BIGINT PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
    encrypted_two_factor_secret TEXT NOT NULL,
    two_factor_secret_decryption_nonce TEXT NOT NULL,
    two_factor_secret_hash TEXT NOT NULL,
    recovery_encrypted_two_factor_secret TEXT NOT NULL,
    recovery_two_factor_secret_decryption_nonce TEXT NOT NULL
);

CREATE TABLE temp_two_factor (
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    encrypted_two_factor_secret TEXT NOT NULL,
    two_factor_secret_decryption_nonce TEXT NOT NULL,
    two_factor_secret_hash TEXT NOT NULL UNIQUE,
    creation_time BIGINT NOT NULL,
    expiration_time BIGINT NOT NULL
);

CREATE TABLE two_factor_sessions (
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL UNIQUE,
    creation_time BIGINT NOT NULL,
    expiration_time BIGINT NOT NULL,
    wrong_attempt BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE two_factor_used_codes (
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    used_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, code_hash)
);

CREATE TABLE two_factor_recovery (
    user_id BIGINT PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
    enable_admin_mfa_reset BOOLEAN NOT NULL DEFAULT TRUE,
    server_passkey_secret_data TEXT,
    server_passkey_secret_nonce TEXT,
    user_passkey_secret_data TEXT,
    user_passkey_secret_nonce TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE TABLE srp_auth (
    user_id BIGINT PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
    srp_user_id TEXT NOT NULL UNIQUE,
    salt TEXT NOT NULL,
    verifier TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE TABLE srp_sessions (
    id TEXT PRIMARY KEY,
    srp_user_id TEXT NOT NULL,
    server_key TEXT NOT NULL,
    srp_a TEXT NOT NULL,
    has_verified BOOLEAN NOT NULL DEFAULT FALSE,
    attempt_count BIGINT NOT NULL DEFAULT 0,
    is_fake BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL
);
CREATE INDEX srp_sessions_srp_user_id_idx ON srp_sessions(srp_user_id, created_at);

CREATE TABLE temp_srp_setup (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    srp_user_id TEXT NOT NULL,
    salt TEXT NOT NULL,
    verifier TEXT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE TABLE passkeys (
    id TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    friendly_name TEXT NOT NULL,
    deleted_at BIGINT,
    created_at BIGINT NOT NULL
);
CREATE INDEX passkeys_user_id_idx ON passkeys(user_id);

-- public_key holds the credential's COSE key (base64); authenticator holds JSON with the sign count.
CREATE TABLE passkey_credentials (
    passkey_id TEXT PRIMARY KEY REFERENCES passkeys(id) ON DELETE CASCADE,
    credential_id TEXT NOT NULL UNIQUE,
    public_key TEXT NOT NULL,
    attestation_type TEXT NOT NULL,
    authenticator_transports TEXT NOT NULL,
    credential_flags TEXT NOT NULL,
    authenticator TEXT NOT NULL,
    rp_id TEXT,
    created_at BIGINT NOT NULL
);

CREATE TABLE webauthn_sessions (
    id TEXT PRIMARY KEY,
    challenge TEXT NOT NULL UNIQUE,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    rp_id TEXT,
    allowed_credential_ids TEXT NOT NULL,
    expires_at BIGINT NOT NULL,
    user_verification_requirement TEXT NOT NULL,
    extensions TEXT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE TABLE passkey_login_sessions (
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL UNIQUE,
    creation_time BIGINT NOT NULL,
    expiration_time BIGINT NOT NULL,
    token_fetch_cnt BIGINT NOT NULL DEFAULT 0,
    verified_at BIGINT,
    token_data TEXT
);

CREATE TABLE families (
    id TEXT PRIMARY KEY,
    admin_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    member_id BIGINT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('SELF', 'CLOSED', 'INVITED', 'ACCEPTED', 'DECLINED', 'REVOKED', 'REMOVED', 'LEFT')),
    token TEXT UNIQUE,
    storage_limit BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CONSTRAINT uidx_families_member_mapping UNIQUE (admin_id, member_id)
);
CREATE INDEX families_member_id_idx ON families(member_id);

CREATE TABLE storage_bonus (
    bonus_id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    user_id BIGINT NOT NULL,
    storage BIGINT NOT NULL,
    valid_till BIGINT NOT NULL DEFAULT 0,
    revoke_reason TEXT,
    is_revoked BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX storage_bonus_user_id_idx ON storage_bonus(user_id);

CREATE TABLE referral_codes (
    code TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at BIGINT NOT NULL
);
CREATE INDEX referral_codes_user_id_idx ON referral_codes(user_id);

CREATE TABLE referral_tracking (
    invitor_id BIGINT NOT NULL,
    invitee_id BIGINT NOT NULL UNIQUE,
    plan_type TEXT NOT NULL,
    invitee_on_paid_plan BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL
);

CREATE TABLE remote_store (
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    key_name TEXT NOT NULL,
    key_value TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, key_name)
);

CREATE TABLE push_tokens (
    fcm_token TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    apns_token TEXT,
    platform TEXT NOT NULL DEFAULT 'ios',
    session_token_hash TEXT,
    notification_public_key TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    last_notified_at BIGINT NOT NULL
);
CREATE INDEX push_tokens_session_token_hash_idx ON push_tokens(session_token_hash);

-- Accounts scheduled for data deletion (museum's data_cleanup). The data itself is
-- removed by the files/trash workstream's cleanup job once stage_schedule_time passes.
CREATE TABLE data_cleanup (
    user_id BIGINT PRIMARY KEY,
    stage TEXT NOT NULL DEFAULT 'scheduled',
    stage_schedule_time BIGINT NOT NULL,
    stage_attempt_count BIGINT NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT '',
    email_hash TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE TABLE discount_coupons (
    provider_name TEXT NOT NULL,
    code TEXT NOT NULL,
    claimed_by_user_id BIGINT,
    claimed_at BIGINT,
    sent_count BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CONSTRAINT discount_coupons_provider_code_unique UNIQUE (provider_name, code)
);

CREATE TABLE public_paste_tokens (
    id TEXT PRIMARY KEY,
    access_token TEXT NOT NULL UNIQUE,
    encrypted_data TEXT NOT NULL,
    decryption_header TEXT NOT NULL,
    encrypted_paste_key TEXT NOT NULL,
    encrypted_paste_key_nonce TEXT NOT NULL,
    kdf_nonce TEXT NOT NULL,
    kdf_mem_limit BIGINT NOT NULL,
    kdf_ops_limit BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE TABLE events (
    id TEXT NOT NULL,
    event TEXT NOT NULL,
    app TEXT NOT NULL,
    platform TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    user_id BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CONSTRAINT events_id_event_idx UNIQUE (id, event)
);
