-- ─── memories ────────────────────────────────────────────────────────────
CREATE TABLE memories (
    id            TEXT    PRIMARY KEY,            -- 8-hex prefix sha256(body)
    slug          TEXT    NOT NULL,
    kind          TEXT    NOT NULL CHECK (kind IN
                          ('decision','bug','convention',
                           'discovery','pattern','note')),
    repo          TEXT,
    author        TEXT,
    quality       INTEGER NOT NULL DEFAULT 3 CHECK (quality BETWEEN 1 AND 5),
    schema        INTEGER NOT NULL DEFAULT 1,
    content_hash  TEXT    NOT NULL,               -- sha256(body.trim_end())
    body          TEXT    NOT NULL,
    created_at    TEXT    NOT NULL,               -- RFC3339
    updated_at    TEXT    NOT NULL,
    deleted_at    TEXT,                           -- soft delete
    md_path       TEXT    NOT NULL                -- relative to data_dir
);
CREATE INDEX idx_memories_repo    ON memories(repo)
    WHERE deleted_at IS NULL;
CREATE INDEX idx_memories_kind    ON memories(kind)
    WHERE deleted_at IS NULL;
CREATE INDEX idx_memories_updated ON memories(updated_at);

CREATE TABLE memory_tags (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    tag       TEXT NOT NULL,
    PRIMARY KEY (memory_id, tag)
);
CREATE INDEX idx_memory_tags_tag ON memory_tags(tag);

-- ─── memory vectors (sqlite-vec) ─────────────────────────────────────────
-- dim configurable via COMEMORY_VECTOR_DIM (default 1024).
-- the caller MUST send vectors of the configured dim; mismatch is
-- a hard error.
CREATE VIRTUAL TABLE memory_vec USING vec0(
    memory_id TEXT PRIMARY KEY,
    embedding FLOAT[1024]
);

-- ─── memory full-text (FTS5, contentless mirror) ─────────────────────────
CREATE VIRTUAL TABLE memory_fts USING fts5(
    memory_id UNINDEXED,
    body,
    tags,
    tokenize = 'porter unicode61 remove_diacritics 2'
);

-- ─── code symbols (BYO-vector or lexical-only) ───────────────────────────
CREATE TABLE code_symbols (
    id          INTEGER PRIMARY KEY,
    repo        TEXT    NOT NULL,
    path        TEXT    NOT NULL,                 -- relative to repo root
    blob_oid    TEXT    NOT NULL,                 -- git blob hash (incremental)
    symbol      TEXT    NOT NULL,                 -- qualified name
    kind        TEXT    NOT NULL,                 -- function/struct/...
    lang        TEXT    NOT NULL,                 -- rust/typescript/...
    line_start  INTEGER NOT NULL,
    line_end    INTEGER NOT NULL,
    snippet     TEXT    NOT NULL,                 -- raw text for FTS + display
    simhash     INTEGER NOT NULL,                 -- 64-bit SimHash of tokens
    indexed_at  TEXT    NOT NULL,
    UNIQUE (repo, path, symbol, line_start)
);
CREATE INDEX idx_code_repo_path ON code_symbols(repo, path);
CREATE INDEX idx_code_blob      ON code_symbols(blob_oid);
CREATE INDEX idx_code_simhash   ON code_symbols(simhash);

CREATE VIRTUAL TABLE code_vec USING vec0(
    symbol_id INTEGER PRIMARY KEY,
    embedding FLOAT[768]
);

CREATE VIRTUAL TABLE code_fts USING fts5(
    symbol_id UNINDEXED,
    symbol,
    snippet,
    path_tokens,                                  -- path split on /._-
    tokenize = 'unicode61 remove_diacritics 2'
);

-- ─── per-file indexing marker for incremental re-index ───────────────────
CREATE TABLE indexed_files (
    repo       TEXT NOT NULL,
    path       TEXT NOT NULL,
    blob_oid   TEXT NOT NULL,
    indexed_at TEXT NOT NULL,
    PRIMARY KEY (repo, path)
);

-- ─── graph edges (replaces kuzu) ─────────────────────────────────────────
-- node addressing:
--   memory:<id>
--   file:<repo>:<path>
--   symbol:<symbol_id>
--   repo:<repo>
--   author:<name>
--   tag:<name>
CREATE TABLE edges (
    src_kind   TEXT NOT NULL,
    src_id     TEXT NOT NULL,
    dst_kind   TEXT NOT NULL,
    dst_id     TEXT NOT NULL,
    rel        TEXT NOT NULL CHECK (rel IN
               ('in_repo','authored_by','tagged',
                'references_file','references_symbol',
                'relates_to','supersedes','conflicts_with','derived_from')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (src_kind, src_id, dst_kind, dst_id, rel)
);
CREATE INDEX idx_edges_src ON edges(src_kind, src_id, rel);
CREATE INDEX idx_edges_dst ON edges(dst_kind, dst_id, rel);

-- ─── stats / feedback (kept from v0.1) ───────────────────────────────────
CREATE TABLE search_stats (
    id          INTEGER PRIMARY KEY,
    query       TEXT NOT NULL,
    hit_count   INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    ran_at      TEXT NOT NULL
);

CREATE TABLE feedback (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    delta     INTEGER NOT NULL,
    given_at  TEXT NOT NULL
);

-- ─── schema metadata seeds ───────────────────────────────────────────────
-- The 'version' key is set by `migrate::run`; only seed vector dims here.
INSERT INTO schema_meta(key, value) VALUES
    ('memory_vector_dim', '1024'),
    ('code_vector_dim',   '768')
ON CONFLICT(key) DO NOTHING;
