-- Durability (section 2): object checksums, scrub state, replicas, events.
-- Times are microseconds since the epoch. Keys are the opaque object keys
-- (already ciphertext objects); nothing here reveals content.

-- One row per object in the primary store.
-- state: 'ok' (hashed and last read matched), 'unhashed' (seen, not yet
-- read), 'corrupt' (read did not match), 'missing' (gone without a delete),
-- 'deleted' (deleted through the server; replicas kept for retention).
CREATE TABLE durability_objects (
    object_key TEXT PRIMARY KEY,
    size BIGINT NOT NULL,
    sha256 TEXT,
    state TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    verified_at BIGINT NOT NULL DEFAULT 0,
    deleted_at BIGINT,
    repaired_at BIGINT,
    last_error TEXT,
    updated_at BIGINT NOT NULL,
    -- Last catalog sweep that listed the object in storage.
    seen_at BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX durability_objects_state ON durability_objects (state, verified_at);

-- One row per object copy on a replication target.
-- state: 'ok', 'corrupt', 'missing'.
CREATE TABLE durability_replicas (
    target TEXT NOT NULL,
    object_key TEXT NOT NULL,
    size BIGINT NOT NULL,
    sha256 TEXT NOT NULL,
    state TEXT NOT NULL,
    copied_at BIGINT NOT NULL,
    verified_at BIGINT NOT NULL,
    last_error TEXT,
    PRIMARY KEY (target, object_key)
);
CREATE INDEX durability_replicas_verify ON durability_replicas (target, verified_at);

-- Things worth telling the operator: shown on the status page, sent as alerts.
-- severity: 'info', 'warning', 'critical'.
CREATE TABLE durability_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at BIGINT NOT NULL,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    subject TEXT NOT NULL,
    detail TEXT NOT NULL,
    alerted BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX durability_events_at ON durability_events (at);

-- Last run of each background job (sweep, scrub, replicate:<target>, …).
-- stats is a JSON object.
CREATE TABLE durability_jobs (
    job TEXT PRIMARY KEY,
    started_at BIGINT NOT NULL,
    finished_at BIGINT,
    ok BOOLEAN NOT NULL DEFAULT TRUE,
    stats TEXT NOT NULL DEFAULT '{}',
    error TEXT
);
