# store/

**What belongs here:** the central SQLite layer backing `comemory.db` —
connection setup (PRAGMAs, migrations, `sqlite-vec` auto-extension), the
versioned migration runner, the declared schema (`schema*.rs`), FTS5 helpers for both the memory and code legs,
`vec0` insert/KNN with dim guards, the `edges` FTS triplet index, batched
per-memory metadata, the identifier tokenizer, and row CRUD for memories,
code symbols, code references, documents, and the source-registry mirror.

**What does NOT belong here:** ranking or business logic, and — since #177 —
any call INTO a domain. `store/` only reads and writes rows; `retrieval/`
decides what to query and how to combine results, and `domains::graph`
decides what edges mean. Derived inputs arrive as owned/borrowed row data
(`CreatedWindow`, `MemoryLinks`), and a passive domain model may be named
only as a TYPE — never a function — under the `passive_store_models`
allowlist in `scripts/architecture-policy.json`.

Runtime queries use the schema's generated table and column builders through
`orm.rs`. See [the query inventory](../../docs/guides/runtime-orm.md) for the
remaining unsupported SQL and its upstream issues. `stats_counts::Corpus`
replaces the old table/predicate string pair for repo-scoped counters.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `bandit_arms.rs` | `NewArm` | `bandit_arms` row CRUD: `seed`/`load`/`record_outcome` behind `domains::learning::evaluation::bandit`'s Thompson sampling; the knob grid and win/loss decision stay in `domains::learning::evaluation::bandit` |
| `busy.rs` | `is_locked` | Whether an `Error` wraps SQLite's `SQLITE_BUSY` / `SQLITE_LOCKED` — the only place outside `errors.rs` that inspects a `rusqlite::Error` variant |
| `candidate_dataset.rs` | `snapshot` | The bulk windowed read behind `comemory export-dataset`: all three candidate-observation tables in ONE read transaction, so a capture landing mid-read cannot hand back a header without its candidates. Deliberately never selects `locator_json`, so a display field cannot reach a training record |
| `candidate_judgments.rs` | `upsert_all` | Reviewed relevance verdicts against a captured observation, keyed `(observation_id, candidate_ref)` so a re-judgment replaces rather than accumulates; the declared `CHECK`s refuse a grade, provenance or domain outside their vocabulary |
| `candidate_observations.rs` | `insert` | The persisted #208 candidate observation contract: one header plus every candidate in one transaction, the batched read back, `redact_memory` (the purge's blank-the-passage-keep-the-row rule) and `evict_unjudged_before` (retention, which never evicts a judged observation) |
| `code_feedback.rs` | `SymbolIdentity` | `code_feedback` row CRUD: `own_identity`/`parent_identity` lookups plus the `used`/`irrelevant` upserts and code-tagged `feedback_events` insert; moved out of `domains::learning::code_feedback`, which still owns the chunk-to-parent resolution rule and the transaction boundary |
| `code_graph_edges.rs` | `EdgeQuery` | The dynamic, paginated file→file `edges` window behind `comemory graph` / `GET /api/v1/graph`: relation set + weight floor + optional repo scope, windowed, plus its `total` count; split out of `edges.rs` to stay under the 300-line ceiling |
| `code_graph_nodes.rs` | `NodeRow` | `code_symbols` node aggregation behind `comemory graph`'s node assembly: `fetch_nodes` (whole graph), `fetch_nodes_for_pairs` (chunked, windowed), `fetch_node` (one file), plus `cites_file_predicate` — the shared "memory cites this file" `edges` predicate `domains::graph::graph_nodes`'s `cited_by` also reuses; also `top_symbols` and `citing_memories` — the file-detail reads behind `domains::graph::graph_nodes::detail`'s `top_symbols`/`cited_by` lists |
| `code_ref.rs` | `CodeRefRow` | `code_ref` side table: version-anchor store for explicit code references, plus `for_rel_live` — the live-memory symbol-ref scan behind `maintenance::retention::stale_code` |
| `code_sync.rs` | `parent_symbols_for_file` | The snippet-free projection reads behind `domains::sync::code` (`parent_symbols_for_file`, `import_targets`, `co_changed_pairs`) and the `code_sync:<repo>` cursor in `schema_meta` — what the last push left, so a repeat push can skip the network |
| `code_row.rs` | `CodeSymbolRow` | `code_symbols` row upserts (insert, refresh, delete-by-file), plus `distinct_paths_for_repo`, `parent_snippets` (the re-embed scan behind `maintenance::reembed`'s code leg), `count_for_repo`/`any_indexed` (the `index_runs` count and the `search-code` empty-index probe), the `rank_score` bulk writer behind `domains::graph::materialize`, `find_by_address` (the `references_symbol` edge resolver behind `retrieval::code_ref_collect`), `parent_identity` (chunk-coalescing lookup behind `retrieval::code_rerank`), and `symbol_row_exists` (the ghost-reference resolve check behind `maintenance::retention::stale_code`) |
| `code_text.rs` | `CodeText` | Batched `code_symbols` read of the identity triple, the `blob_oid` content version, the line range and the snippet — the fields `code_rerank::CodeReranked` does not carry, read by the offline benchmark |
| `code_signals.rs` | `Signals` | Per-symbol ranking signals: the `code_symbols` + `code_feedback` join behind `retrieval::code_prior`'s four-prior scorer, re-exported from `code_prior` so its own callers are unaffected |
| `connection.rs` | `open`, `open_read_only`, `write_transaction` | Connection open: serialized PRAGMAs and migrations, `sqlite-vec` auto-extension registration; `write_transaction` reserves SQLite's writer before reading; `open_read_only` is a plain read-only open with no migration, behind `maintenance::doctor`'s forward-compat fallback |
| `doctor_probes.rs` | `live_memory_hashes` | The remaining `comemory doctor` health-check SQL: `live_memory_hashes` (mirror-parity stored hashes), `repo_roots` (the repo-root inventory), and `live_memory_count` |
| `document_fts.rs` | `DocumentFtsHit` | `document_fts` insert/delete helpers + the BM25 MATCH query leg |
| `documents.rs` | `DocumentUpsert` | `documents` + `document_chunks` row CRUD, plus `document_id_in_source` and `document_ids_for_repo_path`, the `(source, path)` / `(repo, path)` document-id lookups behind `domains::graph::doc_link` |
| `edge_fts.rs` | `EdgeFtsHit` | FTS5 triplet index over `edges`: rendering, refresh, and the `comemory edges` lexical ladder |
| `edges.rs` | `insert` | `edges` table CRUD: typed upserts, weighted accumulation, outgoing neighbors, the `supersedes_chain` recursive walk, delete-by-node, the code-graph/`memory_rank` weighted-edge queries, `co_changed`/`imports` scoped deletes, `count_by_rel` (behind `maintenance::overview`'s edge totals), the `file_neighbor_rows` one-hop query, and `insert_memory_references` (the three reference-edge kinds a memory row owes, written from the `store::MemoryLinks` the caller derived); every `domains::graph` algorithm calls this rather than owning its own SQL |
| `edges_neighbors.rs` | `file_neighbor_rows` | The bounded file-neighbor CTE, kept separate from edge CRUD to respect the file-size ceiling |
| `edges_retrieval.rs` | `expand_memory_seeds` | Retrieval-side `edges` reads split out of `edges.rs` to stay under the 300-line ceiling: the memory graph-expansion walk (`retrieval::graph_route`), the context-bundle relation walk (`retrieval::bundle`), the working-set co-change affinity sum (`retrieval::code_prior`), the live-supersede lookup (`retrieval::rerank`, reused by `domains::memories::show`), and `direct_reference_edges` (`comemory show`'s depth-1 reference read) |
| `embed.rs` | `to_vec_blob` | f32 ↔ `vec0` BLOB encoding plus the per-table dim guards |
| `fts.rs` | `CodeFtsHit` | FTS5 insert/search helpers for the code leg |
| `fts_memory.rs` | `MemoryFtsHit` | Memory-leg FTS5 ladder (strict → relaxed → subtoken → expanded) behind `run_memory_match` |
| `gc_learning.rs` | `evict_before` | `retrieval_log` / `feedback_events` eviction past the retention window, behind `comemory gc`'s `sweep_learning` |
| `memory_list.rs` | `ListRow` | Paginated listing of live memories with ordered creation indexes and trigram candidates for literal substring filters |
| `memory_meta.rs` | `MemoryMeta` | Batched per-memory metadata: path, repo, kind, tags, references; also the smaller `memories`-table reads `ids_matching_kind` (ANN kind post-filter, `retrieval::router`), `kind_and_body` (`retrieval::bundle`), `rank_signals` (`retrieval::rerank`), `keeper_stats` (`maintenance::consolidation::keeper`), and `fetch_extra` (the single-row body/quality/timestamps/access/rank_score read behind `comemory show`) |
| `memory_purge.rs` | `purge_memory` | One-transaction hard delete of a **soft-deleted** memory's mirror rows (`memories`, tags, FTS, vec, touching edges, `code_ref`, `feedback` + memory-target `feedback_events`; a live row is refused), plus `expired_deleted_ids` — the `deleted_at`-past-retention scan behind `comemory gc`'s zombie-row pass — `soft_delete` — the `comemory delete` mirror write (stamp `deleted_at`, drop FTS/vec, delete touching edges) called inside `domains::memories::delete::mirror_soft_delete`'s transaction — and `trashed_with_hash` — the "already trashed under this hash" probe behind `domains::sync::exchange::import_state` |
| `memory_signals.rs` | `bump_access` | Memory activation updates and materialized graph-rank writes; callers own the transaction |
| `memory_row.rs` | `insert` | `memories` row upserts and their edge materialization, taking the body's reference targets as a borrowed `store::MemoryLinks` rather than deriving them (`domains::memories::mirror` does that and is the only caller) and emitting them through `edges::insert_memory_references`; the outgoing-edge wipe carries relation-edge timestamps and the mined `co_activated` edges (the one memory-sourced kind with no markdown source) across every re-mirror; also `live_ids` and `live_bodies` (the re-embed scan behind `maintenance::reembed`'s memory leg) and `count_created_since` (the repo-scoped "saves" count behind `domains::learning::recall_status`'s window report) |
| `orm.rs` | `execute` | Store-private execution of generated statements; cached preparation, owned reads, native SQLite errors and caller-owned transactions |
| `migrate.rs` | `CURRENT_VERSION` | Versioned, idempotent schema migrations plus `schema_meta`; loops over the `MIGRATIONS` slice declared in `migrate/list.rs` |
| `schema.rs` | `registry` | The declared schema: `registry()` assembles the `#[table]` / `#[fts5_table]` / `#[vec0_table]` structs from the `schema_*.rs` siblings into a toolu-orm `SchemaRegistry` (what `examples/migrations.rs` diffs into the next `migrations/*.sql`), plus `DECLARED_TABLES`; the colocated fidelity test proves the registry identical to the database the frozen chain builds |
| `schema_core.rs` | `SchemaMeta` | Declared `schema_meta` + `edge_fts` |
| `schema_code.rs` | `CodeSymbols` | Declared `code_symbols`, `code_fts`, `code_vec`, `indexed_files`, `repo_marker` |
| `schema_documents.rs` | `Documents` | Declared `source_roots`, `source_files`, `documents`, `document_chunks`, `document_fts` |
| `schema_graph.rs` | `Edges` | Declared `edges`, `code_ref` (composite primary keys) |
| `schema_history.rs` | `EvalRuns` | Declared `eval_runs`, `gc_runs`, `index_runs` (newest-first `DESC` indexes) and the `index_failures` log |
| `schema_journal.rs` | `journal_file` | The `just migration-journal` / `just migration-adopt` operations over `migrations/_journal.json` and the newest snapshot (`adopt`), tested against copies of the shipped files; `examples/migrations.rs` delegates to them |
| `schema_learning.rs` | `Feedback` | Declared `feedback`, `feedback_events`, `code_feedback`, `query_expansions`, `retrieval_log`, `bandit_arms` |
| `schema_memory.rs` | `Memories` | Declared `memories`, `memory_tags`, `memory_fts`, `memory_substring` (external-content trigrams), `memory_vec` |
| `schema_sync.rs` | `SyncState` | Declared `sync_log`, `sync_state`, `sync_binding` |
| `prune_apply.rs` | `count_orphan_memory_edges` | `maintenance::prune`'s own scan (orphan-edge count, correlated stale-code-file list, one memory's display fields) and apply-time cleanup deletes (orphan edges, stale `code_vec`/`code_fts`/`code_symbols` rows, dangling `references_*`/`co_activated` edges, orphan `code_ref` rows) — everything `prune_signals.rs` does not already own |
| `prune_signals.rs` | `SignalCandidate` | The low-quality/zero-incoming-edge scan and the superseded-and-forgotten scan behind `maintenance::retention::low_value`; every comparison operator is load-bearing — `prune --apply` deletes whatever these two queries return |
| `query_expansions.rs` | `NewExpansion` | Mined `(term → expansion)` row CRUD: `delete_all` + `insert` behind `comemory mine --apply`'s replace-all, plus `matching_terms` — the `IN (...)`-list read behind `retrieval::suggest`'s "expansions" list — and `count`/`page`, the console's `domains::learning::console` summary tile and paged expansions list; the tier-4 lexical-ladder read lives in `fts.rs` |
| `repos_inventory.rs` | `RepoMarkerRow` | The `repo_marker` join behind `comemory repos`: one row per indexed repo plus its per-repo file/symbol/memory counters; git-state resolution stays in `domains::code::repos` |
| `schema_meta.rs` | `set_memory_vector_model` | Single-key `schema_meta` writers not already owned by `migrate` or `vector`'s dim guards; behind `config::sync::apply_embed_model`, plus `version` (the required `schema_meta.version` read behind `maintenance::doctor`/`maintenance::stats`), `memory_vector_model` (the required `memory_vector_model` read behind `comemory sync`'s wire vector encode/decode), and the generic keyed `get`/`upsert` behind `cli::lazy_reindex`'s debounce marker |
| `stats_counts.rs` | `scoped_count` | The corpus counters behind `comemory stats`: `scoped_count` (repo-scoped `COUNT(*)`), `count_table` (table-wide `COUNT(*)`), and `db_bytes` (`page_count * page_size`) |
| `simhash_scan.rs` | `SimhashRow` | Bulk `(id, simhash)` scan over live memories, shared by save + consolidate |
| `eval_runs.rs` | `insert` | `eval_runs` writer + newest-first reader — one row per eval/tune/bandit RUN, never per scored candidate |
| `feedback.rs` | `upsert_used` | `feedback` row CRUD: `used`/`irrelevant` upserts plus the one memory-tagged `feedback_events` insert (`insert_event`, which takes `provenance` explicitly — manual, HTTP-implicit, and the auto rewards all go through it); moved out of the feedback recorder, which is `domains::learning::feedback_tracking` since #173 and still owns the provenance choice and the transaction boundary; also `used_query_ids` (the "this query succeeded" scan behind `domains::learning::evaluation::mine`) and `used_events_for_golden` (the feedback-harvest join behind `domains::learning::evaluation::golden`), both taking a `provenance` filter the callers pin to `manual`, `event_counts` (the `feedback_events` totals behind `domains::learning::console::summary`'s header tiles), and `events_since` (the repo-scoped verdict count behind `domains::learning::recall_status`'s window report) and `events_for_query` (the owned `(memory_id, verdict, provenance)` rows for ONE query id, the read `tests/cli_scenario_mcp.rs` uses to assert what provenance actually landed) |
| `gc_runs.rs` | `insert` | `gc_runs` writer — one row per completed `comemory gc` sweep, with bytes freed — plus `newest` (`GcRunRow`), the last-run read behind `GET /api/v1/gc/policy` |
| `repo_drop.rs` | `drop_repo` | Drop every code-index row and file edge for one repo label in one transaction (`DELETE /api/v1/repos/{name}`), memories kept. Rows only — the post-commit derived refresh the dropped edges invalidate belongs to `domains::code::repo_admin::disconnect` |
| `repo_marker.rs` | `last_mined_commit` | `repo_marker.last_mined_commit` read + upsert — the co-change mining cursor behind `domains::graph::materialize`, kept separate from `code_row.rs`'s index-code fields and `repo_marker_roots.rs`'s serve-side reads; also `read_for_lazy_reindex` — the one-row `(last_mined_commit, root_path, archived)` probe behind `cli::lazy_reindex` — `archived` — the one-column flag read behind `domains::code::index_code`'s archived-repo refusal — and `all_repos`/`root_path`/`exists`/`set_archived` behind `domains::graph::graph_recompute` and `domains::code::repo_admin`'s connect/patch/archive lifecycle, and `last_head` behind the code-index manifest and push |
| `repo_marker_roots.rs` | `all_roots` | Reads over `repo_marker.root_path`: `all_roots` enumerates every distinct working-tree root (the `serve` allowed-roots set), `root_path` looks up one repo's stored root, `Ok(None)` distinct from a genuine query `Err` |
| `retrieval_log.rs` | `insert` | `retrieval_log` insert (behind `retrieval::pipeline::log_retrieval`, shared by memory and code searches) plus the raw `returned_ids` window read (source pair, `at` range, optional repo) behind `domains::graph::search_edit`'s search→edit lookback, `queries_excluding_source` — the `(query_id, query, at)` scan behind `domains::learning::evaluation::mine`'s reformulation mining, `prefix_matches` (all rows), `distinct_prefix_matches` — the bounded, Unicode-deduplicated ordered read behind `retrieval::suggest`'s "recent" list, `pending_since` — the unjudged-query `LEFT JOIN feedback_events … IS NULL` scan behind `domains::learning::recall_status`'s "pending" list, and `count_since` — the same window's total tracked-query count (pending ∪ judged) |
| `index_failures.rs` | `record` | `index_failures` row CRUD: append + count + latest-row read, plus the ISO 8601 (UTC) timestamp and the `usize` count clamp that the stats handle used to keep (#173) |
| `indexed_files.rs` | `delete_for_repo` | `indexed_files` cursor reads not already owned by `code_row.rs`'s upsert: the repo-wide cursor wipe (`domains::code::index_code`'s `--mode full` and `code_row::ensure_repo_format`'s format-version gate) and `blob_oid_for` — the per-`(repo, path)` incremental-skip lookup behind `domains::code::index_code::walk` — plus `list_for_repo` / `delete_one`, the manifest read and the removal half of the code-index sync |
| `index_runs.rs` | `insert` | `index_runs` writer + newest-first readers — one row per `index-code` run, outcomes (`ok`/`error`/`cancelled`) included |
| `random_id.rs` | `random_hex` | Shared random-hex id helper, moved out of `serve::security` so non-HTTP callers can use it |
| `rebuild_copy.rs` | `copy_preserved_tables_from_old` | The `ATTACH`-based rebuild preservation copy's entry point: attach `old`, run the code-index/learning/document-domain copy passes, `DETACH` — always, even on a copy failure — as one lifecycle no caller can pair incorrectly; plus `old_table_exists`/`old_column_exists`, the attached-DB schema probes every pass shares, plus `old_table_exists` / `old_column_exists` — the `old.sqlite_master` probes each copy pass uses to skip tables and columns that predate the schema being copied from. |
| `rebuild_copy_code.rs` | `copy_code_tables_inner` | Rebuild preservation copy: `code_symbols`, `indexed_files`, the mined `co_changed`/`imports`/`co_activated` edges, the per-repo `schema_meta`/`repo_marker` cursors, and the `code_fts`/`code_vec` virtual tables |
| `rebuild_copy_documents.rs` | `copy_document_tables_inner` | Rebuild preservation copy: `source_files`, `documents`, `document_chunks`, `document_fts`, in FK/parent-before-child order; `source_roots` is reconciled from `sources.toml` instead, not copied here |
| `rebuild_copy_history.rs` | `copy_history_tables` | Rebuild preservation copy: `eval_runs` (+ v15's `discarded`), `gc_runs`, `index_runs`, and the v16 `sync_log`/`sync_state`/`sync_binding` tables |
| `rebuild_copy_learning.rs` | `copy_learning_tables_inner` | Rebuild preservation copy: the `feedback`/`code_feedback` counters and the `retrieval_log` telemetry, then (via `rebuild_copy_learning_events` and `rebuild_copy_history`) the event/mined and run-history tables |
| `rebuild_copy_learning_events.rs` | `copy_event_and_mined_tables` | Rebuild preservation copy: `feedback_events`, `query_expansions`, `bandit_arms` — split out of `rebuild_copy_learning.rs` to stay under the 300-line ceiling |
| `sync_log.rs` | `append` | Append-only cloud-sync change journal (`upsert`/`tombstone`/`restore`, origin `local`/`sync`); also `backfill_missing_local` — mint local upserts for live memories that never got a log row |
| `sync_manifest.rs` | `live_content_hashes` | The live `content_hash` set behind `GET /sync/manifest`; the caller buckets and digests them |
| `sync_state.rs` | `ensure` | Per-workspace pull/push cursors for cloud sync |
| `sync_binding.rs` | `bind_first` | First-push workspace binding + `--allow-secret` overrides |
| `sources.rs` | `SourceRootUpsert` | `source_roots` row CRUD — the SQLite mirror of `sources.toml` |
| `tokenizer.rs` | — | Parent declaration for the `tokenizer/` folder (see `store/tokenizer/README.md`) |
| `trash_list.rs` | `DeletedMemoryRow` | The `memories` scan behind `GET /api/v1/trash`: every soft-deleted row, newest deletion first; the on-disk join and day-countdown math stay in `domains::memories::trash` |
| `vector.rs` | `MemoryHit` | `vec0` insert and KNN against `memory_vec` / `code_vec`, plus `is_loaded` — whether `sqlite-vec` registered on a connection, behind `comemory doctor`'s check — `replace_memory`/`replace_code` — the delete-then-insert pair behind a re-save (`domains::memories::save`) or re-embed (`maintenance::reembed`) of an id that may already have a vector row — and `memory_embedding_blob` — the raw `memory_vec.embedding` read behind `comemory sync`'s wire vector encode |

`tokenizer/` (FTS5 tokenizer FFI) and `migrate/` (the `MIGRATIONS` slice plus
the migration preflight/snapshot safety net) are documented in their own
`README.md` per the guardrails nested-folder rule. The migration SQL itself
lives at the crate root in `migrations/` (toolu-orm's `migrations_dir`, with
its `_journal.json` and snapshot) — see `migrations/README.md`.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/store.rs` (`pub mod
<name>;`) and callers import concrete paths.

Connection setup serializes WAL initialization and migrations with a sibling
`.open.lock` advisory lock. `connection::write_transaction` reserves the writer
before mirror reads so concurrent memory saves wait instead of failing a
deferred transaction upgrade.
