-- v9: versioned-pointer code references. A new `code_ref` side table stores the
-- version anchor (blob OID + commit + branch) captured at save for each explicit
-- `--ref-file` / `--ref-symbol` link, alongside the existing additive
-- `references_file` / `references_symbol` rows in `edges`. The edge graph is the
-- relationship; this table is the anchor store that lets fetch classify a ref as
-- fresh / stale / ghost / unpinned by a cheap git-blob compare.
--
-- Rebuilt from markdown frontmatter by `memory_row::insert` (so `comemory
-- rebuild` restores anchors for free); intentionally NOT among the
-- `copy_preserved_tables_from_old` set. No `IF NOT EXISTS`: the migration
-- harness (`migrate::apply`) gates this on a `schema_meta` marker inside a
-- transaction, matching the 0007/0008 precedent.
CREATE TABLE code_ref (
    memory_id     TEXT NOT NULL,
    rel           TEXT NOT NULL CHECK (rel IN ('references_file','references_symbol')),
    dst_id        TEXT NOT NULL,        -- <repo>:<path>[:<symbol>]
    pinned_blob   TEXT,                 -- HEAD-tree blob OID at save; NULL = unpinned
    pinned_commit TEXT,                 -- HEAD SHA at save; NULL when no repo
    branch        TEXT,                 -- advisory; NULL ok
    created_at    TEXT NOT NULL,
    PRIMARY KEY (memory_id, rel, dst_id)
);
CREATE INDEX idx_code_ref_dst ON code_ref(dst_id, rel);
