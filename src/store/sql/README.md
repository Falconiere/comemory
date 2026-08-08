# store/sql/

**What belongs here:** versioned schema migration DDL — one numbered `.sql`
file per schema version, applied in order by `store::migrate`. Filenames are
`NNNN_<slug>.sql`; the numeric prefix is the migration order and the version
stamped into `schema_meta`.

**What does NOT belong here:** hand-run ad-hoc SQL, or an edit to an already-
shipped migration file. Every schema change is a *new* numbered file appended
to this directory — migrations are immutable once released, because
`store::migrate` is idempotent and re-applies only what a given database
hasn't seen yet. This is machine-enforced, not just convention:
`scripts/migration-check.sh` (wired into `check-all.sh`) diffs every
already-released file here against its content at the first release tag
that shipped it and fails the build on any drift.

## Contents

One line per file:

| File | Purpose |
| --- | --- |
| `0001_schema_meta.sql` | The `schema_meta` key/value table itself (bootstraps version tracking) |
| `0002_v2_tables.sql` | v2: `memories`, `memory_fts`, `memory_vec`, `code_symbols`, `code_fts`, `code_vec`, `edges` |
| `0003_stats_tables.sql` | Stats tables migrated from the old `stats.db` (v0.2 unification): `retrieval_log` |
| `0004_v4_rank.sql` | v4: rank-blend core — access tracking, memory simhash, identifier-tokenized FTS |
| `0005_v5_learning.sql` | v5: learning loop — feedback provenance, mined expansions, query-log duration |
| `0006_v6_code_graph.sql` | v6: code graph — extended edge kinds + weight, materialized PageRank, cAST chunk parents |
| `0007_v7_repo_root.sql` | v7: persist the absolute working-tree root for `comemory serve` file resolution |
| `0008_v8_reinforcement.sql` | v8: auto-reinforcement — `co_activated` edge kind, `feedback_events.provenance` |
| `0009_v9_code_refs.sql` | v9: versioned-pointer code references — the `code_ref` side table |
| `0010_v10_bandit.sql` | v10: `bandit_arms` — discrete-arm Beta posteriors for `comemory bandit` |
| `0011_v11_memory_rank.sql` | v11: `memories.rank_score` — PageRank over the live-memory graph |
| `0012_v12_edge_fts.sql` | v12: `edge_fts` — the FTS5 triplet index over `edges` for `comemory edges` |
| `0013_v13_documents.sql` | v13: unified document indexing — source registry mirror, documents, chunks, BM25 index |

When you add a migration, append the next-numbered file and add its row above
— never edit an existing one.
