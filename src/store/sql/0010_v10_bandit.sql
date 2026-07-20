-- v10: bandit_arms — discrete-arm Beta posteriors for `comemory bandit`.
-- Arms are seeded from the live `[tune]` cartesian grid; orphan rows from
-- retired grid points are retained but ignored by propose.

CREATE TABLE bandit_arms (
    arm_id     TEXT PRIMARY KEY,
    rrf_k      REAL NOT NULL,
    decay      REAL NOT NULL,
    mmr_lambda REAL NOT NULL,
    bm25_body  REAL NOT NULL,
    bm25_tags  REAL NOT NULL,
    alpha      REAL NOT NULL DEFAULT 1.0,
    beta       REAL NOT NULL DEFAULT 1.0,
    pulls      INTEGER NOT NULL DEFAULT 0,
    last_mrr   REAL,
    updated_at TEXT NOT NULL
);
