# migrations/

**What belongs here:** the versioned schema migration DDL — one numbered
`NNNN_<slug>.sql` per schema version, applied in order by `store::migrate` —
plus toolu-orm's bookkeeping for the generate loop: `_journal.json` (order +
`sha256:…` per file) and `<newest>.snapshot.json` (the declared schema as of
that migration, what the next `just migration <name>` diffs against). The
numeric prefix is the migration order and the version stamped into
`schema_meta`. This directory moved here from under `src/store/` in v0.29
(toolu-orm adoption); the files themselves did not change.

**What does NOT belong here:** hand-run ad-hoc SQL, or an edit to an
already-shipped migration file. Every schema change is a *new* numbered file
appended to this directory — migrations are immutable once released, because
`store::migrate` is idempotent and re-applies only what a given database
hasn't seen yet. This is machine-enforced twice: `scripts/migration-check.sh`
(wired into `check-all.sh`) diffs every already-released file against its
content at the first release tag that shipped it (under either path), and
`migration_integrity_journal_matches_migrations` fails when a file's bytes no
longer hash to its `_journal.json` entry.

## Authoring a migration

| The table is… | Do |
| --- | --- |
| declared in `src/store/schema_*.rs` | edit the struct, then `just migration <name>` — writes `NNNN_<name>.sql` + `NNNN_<name>.snapshot.json` + a journal entry, or prints `no schema change` |
| hand-SQL (not in `store::schema::registry()`) | write `NNNN_<name>.sql` by hand (`--> statement-breakpoint` between statements is optional — the runner uses `execute_batch`), then `just migration-journal migrations/NNNN_<name>.sql` |
| newly declared after being hand-SQL | `just migration-adopt` restates the newest snapshot from the registry so the next generate does not try to `CREATE` it |

Then, for either kind: append the `Migration { key, sql: include_str!, class,
post, markers }` entry to `store::migrate::list::MIGRATIONS`, bump
`CURRENT_VERSION`, and add the row below. The `migration_integrity_*`,
`schema_fidelity_*` and `schema_drift_*` suites fail until all of it agrees.
See `docs/guides/schema-migrations.md` for the full walkthrough and the list
of tables that stay hand-SQL until upstream toolu-orm issues land.

## Contents

One line per file:

| File | Purpose |
| --- | --- |
| `_journal.json` | toolu-orm journal: every migration in apply order with its SHA-256 (`sha256:<hex>` of the file bytes) |
| `0016_v16_sync.snapshot.json` | The declared schema (`store::schema::registry()`) as of v16 — the baseline the next generate diffs against |
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
| `0014_v14_console.sql` | v14: console history tables — `eval_runs` (one row per eval/tune/bandit run) and `gc_runs` |
| `0015_v15_console_api.sql` | v15: console API — `index_runs` history, `eval_runs.discarded` (dismissed proposal), `repo_marker.archived` |
| `0016_v16_sync.sql` | v16: cloud sync — `sync_log`, `sync_state`, `sync_binding`, `memory_vector_model` schema_meta key |

When you add a migration, append the next-numbered file and add its row above
— never edit an existing one.
