-- v13: unified document indexing (v1, lexical-only) — the source
-- registry mirror, extracted documents, ordered chunks, and the BM25
-- lexical index over them; plus `edges.rel` extended with the two new
-- deterministic link kinds.
--
-- `source_roots` mirrors `sources.toml` (the durable source of truth
-- written by `src/source/registry.rs`); this table only carries
-- ephemeral state (status, timestamps) reconciled from the TOML at open.
-- `source_files` is one row per discovered candidate under a source.
-- `documents` is one logical document per source file (the id survives
-- content edits at the same path; a rename with an unchanged,
-- source-unique content hash keeps the id, otherwise delete+create —
-- application-level, not enforced here). `document_chunks` holds the
-- ordered passages a document was split into. `document_fts` is the
-- lexical index over those passages: one row per chunk, tokenized with
-- the same per-connection `identifier` tokenizer as `code_fts` (since
-- its 0004 rebuild) and `edge_fts` (since 0012) — registered in
-- `store::connection::open` before migrations run, so it is available to
-- this CREATE. No FTS triggers: refresh is an explicit DELETE + INSERT
-- driven by the index writer, matching every other FTS table in this
-- schema.
--
-- Deletion cascades top-down through FK `ON DELETE CASCADE` (the
-- `memory_tags` precedent from 0002): dropping a `source_roots` row
-- removes its `source_files`, which removes their `documents`, which
-- removes their `document_chunks`. `document_fts`, being a virtual
-- table, cannot carry a FK — the writer/unindex path must delete its
-- rows explicitly, the same way `memory_fts`/`code_fts` are synced.
--
-- The `edges` table is REBUILT (create-copy-drop-rename) because its
-- `rel` CHECK enumerates the allowed kinds and SQLite cannot alter a
-- CHECK. This mirrors the 0008 rebuild exactly: same column list (incl.
-- the v6 `weight` column), same PRIMARY KEY, both indexes recreated
-- inside the same transaction (migrate::apply). No table FK-references
-- `edges`, so the drop/rename is safe under `PRAGMA foreign_keys=ON`.
-- All existing rows (incl. weight + created_at) are copied forward
-- verbatim.
--
-- New rel kinds: 'member_of_source' (file -> source, filtering/explain
-- only — deliberately excluded from `graph_route::ALLOWED_RELS`, so the
-- existing graph-expansion walk is byte-identical to today) and
-- 'references_document' (the resolved, typed form of an explicit
-- backtick-fenced mention once its target has a `documents` row:
-- memory -> document, or document -> document for a resolved markdown
-- link). Both are materialized by the link deriver added in a later
-- step; this migration only widens the CHECK to accept them.

CREATE TABLE source_roots (
    id             TEXT NOT NULL PRIMARY KEY,   -- 32-hex lowercase (128-bit)
    canonical_path TEXT NOT NULL UNIQUE,
    kind           TEXT NOT NULL CHECK (kind IN ('file','dir')),
    repo           TEXT,
    status         TEXT NOT NULL DEFAULT 'active'
                       CHECK (status IN ('active','unreachable')),
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);

CREATE TABLE source_files (
    id             TEXT NOT NULL PRIMARY KEY,   -- 32-hex lowercase (128-bit)
    source_id      TEXT NOT NULL
                       REFERENCES source_roots(id) ON DELETE CASCADE,
    relative_path  TEXT NOT NULL,               -- relative to source_roots.canonical_path
    classification TEXT NOT NULL
                       CHECK (classification IN ('document','ignored','unsupported')),
    size           INTEGER NOT NULL,            -- bytes, from the discovery walk
    mtime          INTEGER NOT NULL,            -- nanoseconds since UNIX epoch; fast fingerprint
    sha256         TEXT,                        -- NULL until the fingerprint changes and is confirmed
    status         TEXT NOT NULL DEFAULT 'pending'
                       CHECK (status IN
                           ('pending','indexed','stale','too_large','error',
                            'unsupported','deleted')),
    error          TEXT,                        -- typed diagnostic; NULL when status has none
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    UNIQUE (source_id, relative_path)
);
-- (source_id, relative_path) above already indexes source_id as its
-- leftmost column; the extra index here serves the status-scoped scans
-- doctor/prune-style tooling runs (e.g. "list every 'error' row").
CREATE INDEX idx_source_files_status ON source_files(status);

CREATE TABLE documents (
    id             TEXT NOT NULL PRIMARY KEY,   -- 32-hex lowercase (128-bit)
    source_file_id TEXT NOT NULL UNIQUE
                       REFERENCES source_files(id) ON DELETE CASCADE,
    title          TEXT NOT NULL,               -- first heading, else file stem
    repo           TEXT,
    revision_hash  TEXT NOT NULL,               -- content identity for rename/edit detection
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);

CREATE TABLE document_chunks (
    document_id  TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL,             -- 0-based order within the document
    heading_path TEXT NOT NULL DEFAULT '',
    char_start   INTEGER NOT NULL,
    char_end     INTEGER NOT NULL,
    line_start   INTEGER NOT NULL,
    line_end     INTEGER NOT NULL,
    simhash      INTEGER NOT NULL,             -- 64-bit SimHash bit pattern (src/simhash.rs)
    text         TEXT NOT NULL,                -- the passage itself
    PRIMARY KEY (document_id, ordinal)
);

-- One row per chunk; document_id/ordinal are the UNINDEXED payload that
-- locates the citation (mirrors the code_fts `symbol_id UNINDEXED`
-- pattern). path_tokens carries the raw relative path — like code_fts,
-- pre-lowercasing would destroy the camelCase/word boundaries the
-- `identifier` tokenizer itself splits on.
CREATE VIRTUAL TABLE document_fts USING fts5(
    document_id UNINDEXED,
    ordinal     UNINDEXED,
    title,
    headings,
    passage,
    path_tokens,
    tokenize = 'identifier'
);

CREATE TABLE edges_v13 (
    src_kind   TEXT NOT NULL,
    src_id     TEXT NOT NULL,
    dst_kind   TEXT NOT NULL,
    dst_id     TEXT NOT NULL,
    rel        TEXT NOT NULL CHECK (rel IN
               ('in_repo','authored_by','tagged',
                'references_file','references_symbol',
                'relates_to','supersedes','conflicts_with','derived_from',
                'co_changed','imports','co_activated',
                'member_of_source','references_document')),
    weight     INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    PRIMARY KEY (src_kind, src_id, dst_kind, dst_id, rel)
);
INSERT INTO edges_v13(src_kind,src_id,dst_kind,dst_id,rel,weight,created_at)
SELECT src_kind,src_id,dst_kind,dst_id,rel,weight,created_at FROM edges;
DROP TABLE edges;
ALTER TABLE edges_v13 RENAME TO edges;
CREATE INDEX idx_edges_src ON edges(src_kind, src_id, rel);
CREATE INDEX idx_edges_dst ON edges(dst_kind, dst_id, rel);
