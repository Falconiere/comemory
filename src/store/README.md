# store/

**What belongs here:** the central SQLite layer backing `comemory.db` —
connection setup (PRAGMAs, migrations, `sqlite-vec` auto-extension), the
versioned migration runner, FTS5 helpers for both the memory and code legs,
`vec0` insert/KNN with dim guards, the `edges` FTS triplet index, batched
per-memory metadata, the identifier tokenizer, and row CRUD for memories,
code symbols, code references, documents, and the source-registry mirror.

**What does NOT belong here:** ranking or business logic. `store/` only reads
and writes rows; `retrieval/` decides what to query and how to combine
results, and `graph/` decides what edges mean.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `bandit_arms.rs` | `NewArm` | `bandit_arms` row CRUD: `seed`/`load`/`record_outcome` behind `eval::bandit`'s Thompson sampling; the knob grid and win/loss decision stay in `eval::bandit` |
| `busy.rs` | `is_locked` | Whether an `Error` wraps SQLite's `SQLITE_BUSY` / `SQLITE_LOCKED` — the only place outside `errors.rs` that inspects a `rusqlite::Error` variant |
| `code_feedback.rs` | `SymbolIdentity` | `code_feedback` row CRUD: `own_identity`/`parent_identity` lookups plus the `used`/`irrelevant` upserts and code-tagged `feedback_events` insert; moved out of `stats::code_feedback`, which still owns the chunk-to-parent resolution rule and the transaction boundary |
| `code_graph_edges.rs` | `EdgeQuery` | The dynamic, paginated file→file `edges` window behind `comemory graph` / `GET /api/v1/graph`: relation set + weight floor + optional repo scope, windowed, plus its `total` count; split out of `edges.rs` to stay under the 300-line ceiling |
| `code_graph_nodes.rs` | `NodeRow` | `code_symbols` node aggregation behind `comemory graph`'s node assembly: `fetch_nodes` (whole graph), `fetch_nodes_for_pairs` (chunked, windowed), `fetch_node` (one file), plus `cites_file_predicate` — the shared "memory cites this file" `edges` predicate `api::graph_nodes`'s `cited_by` also reuses |
| `code_ref.rs` | `CodeRefRow` | `code_ref` side table: version-anchor store for explicit code references, plus `for_rel_live` — the live-memory symbol-ref scan behind `prune::stale_code` |
| `code_row.rs` | `CodeSymbolRow` | `code_symbols` row upserts (insert, refresh, delete-by-file), plus `distinct_paths_for_repo`, the `rank_score` bulk writer behind `graph::materialize`, `find_by_address` (the `references_symbol` edge resolver behind `retrieval::code_ref_collect`), `parent_identity` (chunk-coalescing lookup behind `retrieval::code_rerank`), and `symbol_row_exists` (the ghost-reference resolve check behind `prune::stale_code`) |
| `code_signals.rs` | `Signals` | Per-symbol ranking signals: the `code_symbols` + `code_feedback` join behind `retrieval::code_prior`'s four-prior scorer, re-exported from `code_prior` so its own callers are unaffected |
| `connection.rs` | `open` | Connection open: PRAGMAs, migrations, `sqlite-vec` auto-extension registration |
| `document_fts.rs` | `DocumentFtsHit` | `document_fts` insert/delete helpers + the BM25 MATCH query leg |
| `documents.rs` | `DocumentUpsert` | `documents` + `document_chunks` row CRUD, plus the `(source, path)` / `(repo, path)` document-id lookups behind `graph::doc_link` |
| `edge_fts.rs` | `EdgeFtsHit` | FTS5 triplet index over `edges`: rendering, refresh, and the `comemory edges` lexical ladder |
| `edges.rs` | `insert` | `edges` table CRUD: typed upserts, weighted accumulation, outgoing neighbors, the `supersedes_chain` recursive walk, delete-by-node, the code-graph/`memory_rank` weighted-edge queries, `co_changed`/`imports` scoped deletes, and the `file_neighbor_rows` one-hop query; every `graph/` algorithm calls this rather than owning its own SQL |
| `edges_retrieval.rs` | `expand_memory_seeds` | Retrieval-side `edges` reads split out of `edges.rs` to stay under the 300-line ceiling: the memory graph-expansion walk (`retrieval::graph_route`), the context-bundle relation walk (`retrieval::bundle`), the working-set co-change affinity sum (`retrieval::code_prior`), and the live-supersede lookup (`retrieval::rerank`, reused by `api::show`) |
| `embed.rs` | `to_vec_blob` | f32 ↔ `vec0` BLOB encoding plus the per-table dim guards |
| `fts.rs` | `CodeFtsHit` | FTS5 insert/search helpers for the code leg |
| `fts_memory.rs` | `MemoryFtsHit` | Memory-leg FTS5 ladder (strict → relaxed → subtoken → expanded) behind `run_memory_match` |
| `memory_list.rs` | `ListRow` | Paginated listing of live memories |
| `memory_meta.rs` | `MemoryMeta` | Batched per-memory metadata: path, repo, kind, tags, references; also the smaller `memories`-table reads `ids_matching_kind` (ANN kind post-filter, `retrieval::router`), `kind_and_body` (`retrieval::bundle`), `rank_signals` (`retrieval::rerank`), and `keeper_stats` (`consolidate::keeper`) |
| `memory_purge.rs` | `purge_memory` | One-transaction hard delete of a **soft-deleted** memory's mirror rows (`memories`, tags, FTS, vec, touching edges, `code_ref`, `feedback` + memory-target `feedback_events`; a live row is refused), plus `expired_deleted_ids` — the `deleted_at`-past-retention scan behind `comemory gc`'s zombie-row pass — and `soft_delete` — the `comemory delete` mirror write (stamp `deleted_at`, drop FTS/vec, delete touching edges) called inside `cli::delete::mirror_soft_delete`'s transaction |
| `memory_row.rs` | `insert` | `memories` row upserts and their edge materialization; the outgoing-edge wipe carries relation-edge timestamps and the mined `co_activated` edges (the one memory-sourced kind with no markdown source) across every re-mirror; also `live_ids`, the `rank_score` bulk writer, and the chunked access-count bump behind `graph::memory_rank` / `graph::coactivate` |
| `migrate.rs` | `CURRENT_VERSION` | Versioned, idempotent schema migrations plus `schema_meta`; loops over the `MIGRATIONS` slice declared in `migrate/list.rs` |
| `schema.rs` | — | Module-doc placeholder for the v0.2 schema; DDL text lives in `sql/` |
| `prune_signals.rs` | `SignalCandidate` | The low-quality/zero-incoming-edge scan and the superseded-and-forgotten scan behind `prune::low_value`; every comparison operator is load-bearing — `prune --apply` deletes whatever these two queries return |
| `query_expansions.rs` | `NewExpansion` | Mined `(term → expansion)` row CRUD: `delete_all` + `insert` behind `comemory mine --apply`'s replace-all; the tier-4 lexical-ladder read lives in `fts.rs` |
| `schema_meta.rs` | `set_memory_vector_model` | Single-key `schema_meta` writers not already owned by `migrate` or `vector`'s dim guards; behind `config::sync::apply_embed_model`, plus the generic keyed `get`/`upsert` behind `cli::lazy_reindex`'s debounce marker |
| `simhash_scan.rs` | `SimhashRow` | Bulk `(id, simhash)` scan over live memories, shared by save + consolidate |
| `eval_runs.rs` | `insert` | `eval_runs` writer + newest-first reader — one row per eval/tune/bandit RUN, never per scored candidate |
| `feedback.rs` | `upsert_used` | `feedback` row CRUD: `used`/`irrelevant` upserts plus memory-tagged `feedback_events` inserts; moved out of `stats::feedback`, which still owns the provenance vocabulary and the transaction boundary; also `used_query_ids` (the "this query succeeded" scan behind `eval::mine`) and `used_events_for_golden` (the feedback-harvest join behind `eval::golden`) |
| `gc_runs.rs` | `insert` | `gc_runs` writer — one row per completed `comemory gc` sweep, with bytes freed — plus `newest` (`GcRunRow`), the last-run read behind `GET /api/v1/gc/policy` |
| `repo_drop.rs` | `drop_repo` | Drop every code-index row and file edge for one repo label in one transaction (`DELETE /api/v1/repos/{name}`), memories kept |
| `repo_marker.rs` | `last_mined_commit` | `repo_marker.last_mined_commit` read + upsert — the co-change mining cursor behind `graph::materialize`, kept separate from `code_row.rs`'s index-code fields and `repo_marker_roots.rs`'s serve-side reads; also `read_for_lazy_reindex` — the one-row `(last_mined_commit, root_path, archived)` probe behind `cli::lazy_reindex` |
| `repo_marker_roots.rs` | `all_roots` | Reads over `repo_marker.root_path`: `all_roots` enumerates every distinct working-tree root (the `serve` allowed-roots set), `root_path` looks up one repo's stored root, `Ok(None)` distinct from a genuine query `Err` |
| `retrieval_log.rs` | `insert` | `retrieval_log` insert (behind `retrieval::pipeline::log_retrieval`, shared by memory and code searches) plus the raw `returned_ids` window read (source pair, `at` range, optional repo) behind `graph::search_edit`'s search→edit lookback, and `queries_excluding_source` — the `(query_id, query, at)` scan behind `eval::mine`'s reformulation mining |
| `index_failures.rs` | `insert` | `index_failures` row CRUD: append + count + latest-row read, moved out of `stats::sqlite::StatsDb`, which still owns the ISO 8601 timestamp formatting and the `usize` clamp |
| `index_runs.rs` | `insert` | `index_runs` writer + newest-first readers — one row per `index-code` run, outcomes (`ok`/`error`/`cancelled`) included |
| `random_id.rs` | `random_hex` | Shared random-hex id helper, moved out of `serve::security` so non-HTTP callers can use it |
| `sync_log.rs` | `append` | Append-only cloud-sync change journal (`upsert`/`tombstone`/`restore`, origin `local`/`sync`) |
| `sync_state.rs` | `ensure` | Per-workspace pull/push cursors for cloud sync |
| `sync_binding.rs` | `bind_first` | First-push workspace binding + `--allow-secret` overrides |
| `sources.rs` | `SourceRootUpsert` | `source_roots` row CRUD — the SQLite mirror of `sources.toml` |
| `tokenizer.rs` | — | Parent declaration for the `tokenizer/` folder (see `store/tokenizer/README.md`) |
| `vector.rs` | `MemoryHit` | `vec0` insert and KNN against `memory_vec` / `code_vec` |

`sql/` (migration DDL), `tokenizer/` (FTS5 tokenizer FFI), and `migrate/`
(the `MIGRATIONS` slice plus the migration preflight/snapshot safety net) are
documented in their own `README.md` per the guardrails nested-folder rule.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/store.rs` (`pub mod
<name>;`) and callers import concrete paths.
