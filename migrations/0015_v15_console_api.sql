-- v15: console-API additions — an `index_runs` history table plus two flag
-- columns the learning-loop and repository surfaces need.
--
-- `index_runs` exists for the same reason `eval_runs`/`gc_runs` (v14) do:
-- the console renders a history the binary previously never kept. Every
-- `index-code` run — CLI and HTTP alike — records one row here, including
-- the runs that failed or were cancelled, so `GET /index/runs` and the
-- Overview's "last run" tile have a source. `symbols` is the repo's
-- `code_symbols` row count AFTER the run (a snapshot, not a delta), which
-- is what the tile shows.
--
-- `eval_runs.discarded` lets a `tune`/`bandit` proposal be dismissed
-- without applying it: a proposal is an unapplied run whose knobs differ
-- from the live config, and "discarded" is the only state that cannot be
-- derived from what is already stored.
--
-- `repo_marker.archived` records the console's "archive" action — stop
-- indexing this repo, keep its memories searchable, delete nothing.
--
-- Additive class: one new table and two defaulted columns; no existing row
-- is touched. A failed pre-migration snapshot warns rather than refusing.

CREATE TABLE index_runs (
    id            TEXT PRIMARY KEY,
    repo          TEXT NOT NULL,
    root_path     TEXT,
    mode          TEXT NOT NULL CHECK (mode IN ('full', 'incremental')),
    started_at    TEXT NOT NULL,
    finished_at   TEXT NOT NULL,
    duration_ms   INTEGER NOT NULL,
    files_indexed INTEGER NOT NULL,
    symbols       INTEGER NOT NULL,
    outcome       TEXT NOT NULL CHECK (outcome IN ('ok', 'error', 'cancelled')),
    error         TEXT
);

CREATE INDEX idx_index_runs_started ON index_runs(started_at DESC);

ALTER TABLE eval_runs ADD COLUMN discarded INTEGER NOT NULL DEFAULT 0;

ALTER TABLE repo_marker ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;
