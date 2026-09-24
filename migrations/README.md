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
| `0016_v16_sync.snapshot.json` | Initial declared schema baseline at v16 |
| `0019_query_performance.snapshot.json` | Current declared schema, including query indexes and substring search |
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
| `0017_sync_repush.sql` | v17: rewind every `sync_state.pushed_seq` once, so memories the old push filter stranded behind the cursor are re-offered |
| `0018_scheme_path_refs.sql` | v18: delete the `references_*` edges, `code_ref` anchors and `edge_fts` triplets minted from `file:/…`, `./…`, `../…` path expressions (#153) |
| `0019_query_performance.sql` | v19: graph, history, and listing indexes; external-content trigram index with backfill and synchronization triggers |
| `0020_candidate_observations.sql` | v20: candidate observation capture — `candidate_query_observations` (the per-query envelope), `candidate_observations` (each candidate's bounded text and content version) and `candidate_judgments` (reviewed verdicts resolved against them) |
| `0021_activity_log.sql` | v21: the activity feed — `activity_log` (one row per instrumented command run, the SSE cursor being its `AUTOINCREMENT` id) and `gc_runs.activity_rows` |
| `0022_replica_journal.sql` | v22: the `replica-v1` journal (#250) — `replica_stream` (this database's epoch), `replica_payload` (immutable content-addressed bytes), `replica_feed` (the server-ordered acceptance record), `replica_revision`, `replica_operation` (the outgoing outbox), `replica_receipt`, `replica_cursor` and `replica_staged_part`. Its post-apply pass mints the stream epoch |
| `0023_memory_write_intent.sql` | v23: the memory write lifecycle (#251) — `memory_write_intent` (the crash-recovery marker written before the markdown moves and cleared with the mirror) and `memory_needs_embedding` (memories whose imported vector was refused, with the reason) |
| `0024_code_generation.sql` | v24: code-index replication (#252) — `code_generation` (one row per generation of a repo's index, local or pulled, with its parent and activation state) and the pulled projection it activates: `remote_code_file`, `remote_code_symbol` and `remote_code_edge`, all snippet-free by construction |
| `0025_document_revision.sql` | v25: document replication (#253) — the pulled revision cache (`remote_document` with its `staged`/`active`/`superseded` lifecycle, plus `remote_document_chunk`, `remote_document_link` and the `remote_document_fts` index of its own) and `document_share`, the mapping that gives a local document a portable name keyed by canonical repo and normalized repository-relative path, plus `repository_approval`, the label to canonical-repository map the last policy load resolved, kept locally so an offline index run can tell whether a document's repository is approved and what it is called upstream without a policy fetch |
| `0026_replica_events.sql` | v26: shared feedback and activity events (#254) — `replica_device` (this database's device id, minted by the migration's own `INSERT` from SQLite's randomness source), the `event_id` / `device` columns on `feedback_events` and `activity_log` (plus `surface` / `actor` on `feedback_events`) with unique `event_id` indexes, `replica_payload.redaction` (`erased` or `expired`) so retention and purge can be told apart, and `idx_replica_feed_kind_at` so retention reads only the event positions past its cutoff |


When you add a migration, append the next-numbered file and add its row above
— never edit an existing one.
