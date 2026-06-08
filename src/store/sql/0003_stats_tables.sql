-- ─── stats tables migrated from stats.db (v0.2 unification) ─────────────────
-- Complements the `feedback` table already present from 0002_v2_tables.sql.
CREATE TABLE retrieval_log (
    query_id     TEXT PRIMARY KEY,
    query        TEXT NOT NULL,
    returned_ids TEXT NOT NULL,
    at           TEXT NOT NULL
);

CREATE TABLE repo_marker (
    repo            TEXT PRIMARY KEY,
    last_head       TEXT,
    last_indexed_at TEXT
);

CREATE TABLE index_failures (
    id    INTEGER PRIMARY KEY AUTOINCREMENT,
    ts    TEXT NOT NULL,
    error TEXT NOT NULL
);
