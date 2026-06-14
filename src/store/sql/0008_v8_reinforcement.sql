-- v8: auto-reinforcement — extend the `edges.rel` CHECK with
-- 'co_activated' and tag `feedback_events` rows with a `provenance`.
--
-- The edges table is REBUILT (create-copy-drop-rename) because its `rel`
-- CHECK enumerates the allowed kinds and SQLite cannot alter a CHECK.
-- This mirrors the 0006 rebuild exactly: same column list (incl. the v6
-- `weight` column), same PRIMARY KEY, both indexes recreated inside the
-- same transaction (migrate::apply). No table FK-references `edges`, so
-- the drop/rename is safe under `PRAGMA foreign_keys=ON` — matching the
-- 0006 precedent which performed the identical rebuild without toggling
-- the pragma. All existing rows (incl. weight + created_at) are copied
-- forward verbatim.
--
-- node addressing (unchanged from 0006): the new 'co_activated' kind is a
-- memory→file reinforcement edge written by the auto-reinforcement step;
-- dst_id stays the textual qualified `file:<repo>:<path>` form.

CREATE TABLE edges_v8 (
    src_kind   TEXT NOT NULL,
    src_id     TEXT NOT NULL,
    dst_kind   TEXT NOT NULL,
    dst_id     TEXT NOT NULL,
    rel        TEXT NOT NULL CHECK (rel IN
               ('in_repo','authored_by','tagged',
                'references_file','references_symbol',
                'relates_to','supersedes','conflicts_with','derived_from',
                'co_changed','imports','co_activated')),
    weight     INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    PRIMARY KEY (src_kind, src_id, dst_kind, dst_id, rel)
);
INSERT INTO edges_v8(src_kind,src_id,dst_kind,dst_id,rel,weight,created_at)
SELECT src_kind,src_id,dst_kind,dst_id,rel,weight,created_at FROM edges;
DROP TABLE edges;
ALTER TABLE edges_v8 RENAME TO edges;
CREATE INDEX idx_edges_src ON edges(src_kind, src_id, rel);
CREATE INDEX idx_edges_dst ON edges(dst_kind, dst_id, rel);

-- Provenance tag for implicit-vs-manual feedback. NOT NULL DEFAULT
-- 'manual' backfills every existing row and any future insert that omits
-- the column; the auto-reinforcement step writes provenance-tagged
-- implicit feedback (e.g. 'reinforce') under this same column.
ALTER TABLE feedback_events ADD COLUMN provenance TEXT NOT NULL DEFAULT 'manual';
