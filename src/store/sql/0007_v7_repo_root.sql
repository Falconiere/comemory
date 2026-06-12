-- v7: persist the absolute working-tree root captured at index time so the
-- `comemory serve` web UI can resolve a `file:<repo>:<path>` graph node id back
-- to a real file on disk. Nullable: pre-v7 repos (and rows whose root could not
-- be canonicalized) leave this NULL, and `comemory serve` then requires an
-- explicit `--root <repo>=<path>` override. Mirrors the v6 `ADD COLUMN`
-- precedent (repo_marker.last_mined_commit), so the migration stays idempotent.
ALTER TABLE repo_marker ADD COLUMN root_path TEXT;
