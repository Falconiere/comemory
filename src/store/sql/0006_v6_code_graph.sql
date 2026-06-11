-- v6: code graph — extended edge kinds + weight, materialized PageRank,
-- cAST chunk parents, logged search filters, code feedback.
--
-- The edges table is REBUILT (create-copy-drop-rename) because its `rel`
-- CHECK enumerates the allowed kinds and SQLite cannot alter a CHECK.
-- Index recreation is part of the same transaction (migrate::apply).
--
-- node addressing (unchanged from 0002_v2_tables.sql): dst_id is always
-- the textual qualified form so the writer (`cross_link::extract_and_emit`)
-- and the reader (`bundle::code_ref_lookup`) agree without needing a
-- `code_symbols` row to already exist:
--   memory:<id>
--   file:<repo>:<path>
--   symbol:<repo>:<path>:<symbol>
--   repo:<repo>
--   author:<name>
--   tag:<name>

CREATE TABLE edges_v6 (
    src_kind   TEXT NOT NULL,
    src_id     TEXT NOT NULL,
    dst_kind   TEXT NOT NULL,
    dst_id     TEXT NOT NULL,
    rel        TEXT NOT NULL CHECK (rel IN
               ('in_repo','authored_by','tagged',
                'references_file','references_symbol',
                'relates_to','supersedes','conflicts_with','derived_from',
                'co_changed','imports')),
    weight     INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    PRIMARY KEY (src_kind, src_id, dst_kind, dst_id, rel)
);
INSERT INTO edges_v6(src_kind,src_id,dst_kind,dst_id,rel,created_at)
SELECT src_kind,src_id,dst_kind,dst_id,rel,created_at FROM edges;
DROP TABLE edges;
ALTER TABLE edges_v6 RENAME TO edges;
CREATE INDEX idx_edges_src ON edges(src_kind, src_id, rel);
CREATE INDEX idx_edges_dst ON edges(dst_kind, dst_id, rel);

ALTER TABLE code_symbols ADD COLUMN rank_score REAL NOT NULL DEFAULT 0.0;
ALTER TABLE code_symbols ADD COLUMN parent_id INTEGER;

ALTER TABLE retrieval_log ADD COLUMN repo TEXT;
ALTER TABLE retrieval_log ADD COLUMN kind TEXT;
ALTER TABLE retrieval_log ADD COLUMN source TEXT NOT NULL DEFAULT 'search';

ALTER TABLE feedback_events ADD COLUMN target_kind TEXT NOT NULL DEFAULT 'memory';

-- Per-symbol feedback counters, the code-side sibling of `feedback`
-- (memories). No FK to code_symbols(id) — re-indexing may drop/recreate
-- symbol rows while feedback history persists.
CREATE TABLE code_feedback (
    symbol_id        INTEGER PRIMARY KEY,
    used_count       INTEGER NOT NULL DEFAULT 0,
    irrelevant_count INTEGER NOT NULL DEFAULT 0,
    last_used        TEXT
);

ALTER TABLE repo_marker ADD COLUMN last_mined_commit TEXT;

INSERT INTO schema_meta(key, value) VALUES ('code_format_version', '2')
ON CONFLICT(key) DO UPDATE SET value = excluded.value;
