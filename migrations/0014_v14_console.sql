-- v14: console-compatibility history tables — `eval_runs` and `gc_runs`.
--
-- Both exist for the same reason: the console renders a *history* the binary
-- previously never kept. `comemory eval` computed recall/MRR and printed them;
-- nothing recorded the run, so an "Eval runs" table and a recall sparkline had
-- no source. `comemory gc` reported per-run removal counts the same way, so
-- "last gc · 12 days ago" had no source either.
--
-- One row per RUN, never per scored candidate: a `comemory tune` grid scores
-- hundreds of configurations, and writing each would flood the table with rows
-- no surface reads. The winner is what the console's run table shows, so the
-- winner is what is stored, with its knob set snapshotted as JSON.
--
-- `knobs` is JSON text rather than one column per knob deliberately: the
-- `[tune]` grid's shape is config-driven and has already grown twice
-- (graph_hops and graph_seeds joined it in v6), and a schema that must migrate
-- every time a knob is added is a schema that discourages adding knobs.
--
-- Additive class: two new tables, no existing table touched, nothing dropped
-- or rewritten. A failed pre-migration snapshot warns rather than refusing.

CREATE TABLE eval_runs (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL CHECK (kind IN ('eval', 'tune', 'bandit')),
    at            TEXT NOT NULL,
    golden_pairs  INTEGER NOT NULL,
    k             INTEGER NOT NULL,
    recall        REAL NOT NULL,
    mrr           REAL NOT NULL,
    knobs         TEXT NOT NULL,
    applied       INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_eval_runs_at ON eval_runs(at DESC);

CREATE TABLE gc_runs (
    id            TEXT PRIMARY KEY,
    at            TEXT NOT NULL,
    removed       INTEGER NOT NULL,
    log_rows      INTEGER NOT NULL,
    event_rows    INTEGER NOT NULL,
    bytes_freed   INTEGER NOT NULL
);

CREATE INDEX idx_gc_runs_at ON gc_runs(at DESC);
