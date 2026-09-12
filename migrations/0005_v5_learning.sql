-- v5: learning loop — feedback provenance, mined expansions, query-log
-- duration, and the removal of the never-written search_stats table.
--
-- memories.simhash / code_symbols.simhash are recomputed in Rust by
-- `migrate::rehash_simhashes` (run-once, keyed '0005_simhash_rehash' in
-- schema_meta) because simhash::tokens changed casing/folding in M2 and
-- SQLite cannot run the SipHash-based SimHash in SQL.

CREATE TABLE feedback_events (
    id        INTEGER PRIMARY KEY,
    query_id  TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    verdict   TEXT NOT NULL CHECK (verdict IN ('used','irrelevant')),
    at        TEXT NOT NULL
);
CREATE INDEX idx_feedback_events_query ON feedback_events(query_id);

CREATE TABLE query_expansions (
    term       TEXT NOT NULL,
    expansion  TEXT NOT NULL,
    support    INTEGER NOT NULL DEFAULT 1,
    last_mined TEXT NOT NULL,
    PRIMARY KEY (term, expansion)
);

ALTER TABLE retrieval_log ADD COLUMN duration_ms INTEGER;

-- Dead since v0.2 unification: zero writers, zero readers, fully
-- redundant with retrieval_log.
DROP TABLE search_stats;
