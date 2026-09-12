-- v16: cloud sync tables — append-only `sync_log`, per-workspace cursors
-- in `sync_state`, and first-push binding / secret-override records in
-- `sync_binding`. See `docs/toolu/specs/2026-09-02-memory-sync-design.md`
-- (platform repo) for the protocol.
--
-- Additive class: three new tables plus an optional `memory_vector_model`
-- schema_meta key seeded empty (callers / config set the real model). A
-- failed pre-migration snapshot warns rather than refusing.

CREATE TABLE sync_log (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    op           TEXT NOT NULL CHECK (op IN ('upsert', 'tombstone', 'restore')),
    memory_id    TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    at           TEXT NOT NULL,
    origin       TEXT NOT NULL CHECK (origin IN ('local', 'sync'))
);

CREATE INDEX idx_sync_log_memory ON sync_log(memory_id);
CREATE INDEX idx_sync_log_origin_seq ON sync_log(origin, seq);

CREATE TABLE sync_state (
    workspace_id TEXT PRIMARY KEY,
    api_url      TEXT NOT NULL,
    pulled_seq   INTEGER NOT NULL DEFAULT 0,
    pushed_seq   INTEGER NOT NULL DEFAULT 0,
    last_sync_at TEXT
);

CREATE TABLE sync_binding (
    memory_id             TEXT PRIMARY KEY,
    workspace_id          TEXT NOT NULL,
    secret_override_rule  TEXT,
    secret_override_at    TEXT
);

INSERT INTO schema_meta(key, value) VALUES ('memory_vector_model', '')
ON CONFLICT(key) DO NOTHING;
