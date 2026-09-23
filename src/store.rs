//! Single-file SQLite storage: one `comemory.db` holding memories, code
//! rows, edges, FTS5 indexes and `sqlite-vec` vtabs. The extension is loaded
//! on every connection; FTS5 is bundled into rusqlite.

/// The SQLite connection type every other module names.
///
/// Re-exported so `rusqlite` stays confined to this module: callers write
/// `store::Connection`, and the single line below is what changes if the
/// driver ever does.
pub use rusqlite::Connection;

/// The SQLite transaction handle a caller-owned `BEGIN`/`COMMIT` produces.
/// Re-exported for the same reason as [`Connection`]: a `store::`-helper
/// caller that opens its own transaction (`graph::materialize`,
/// `graph::memory_rank`) names `store::Transaction` rather than importing
/// `rusqlite` directly.
pub use rusqlite::Transaction;

/// `activity_log` insert + reads: the feed behind `GET /api/v1/activity`.
pub mod activity;
/// Per-command rollups over `activity_log`: runs, errors, p50/p95 duration.
pub mod activity_rollups;
/// `bandit_arms` row CRUD: seed/load/record-outcome behind `eval::bandit`.
pub mod bandit_arms;
/// Whether an `Error` wraps SQLite's `SQLITE_BUSY` / `SQLITE_LOCKED`.
pub mod busy;
/// Bulk windowed reads over the three candidate-observation tables: the input
/// side of the reviewed dataset export.
pub mod candidate_dataset;
/// `candidate_judgments` row CRUD: reviewed relevance verdicts resolved
/// against a captured observation.
pub mod candidate_judgments;
/// `candidate_query_observations` + `candidate_observations` row CRUD: the
/// persisted candidate observation contract and its retention/purge rules.
pub mod candidate_observations;
/// `code_feedback` row CRUD: per-symbol counter table + code-tagged
/// `feedback_events` inserts.
pub mod code_feedback;
/// `code_generation` row lifecycle: record a generation, activate one and
/// supersede the prior, and read what a repo is at.
pub mod code_generation;
/// Dynamic, paginated file→file `edges` window behind `comemory graph`.
pub mod code_graph_edges;
/// `code_symbols` node aggregation behind `comemory graph`'s node assembly.
pub mod code_graph_nodes;
/// `code_ref` side table: version-anchor store for explicit code references,
/// plus the live-symbol-ref scan behind `maintenance::retention::stale_code`.
pub mod code_ref;
/// `code_symbols` row upserts (insert, refresh, delete-by-file), plus the
/// `symbol_row_exists` ghost-reference resolve check.
pub mod code_row;
/// Per-symbol ranking signals: the `code_symbols` + `code_feedback` join
/// behind `retrieval::code_prior`'s four-prior scorer.
pub mod code_signals;
/// Reads and the pushed cursor behind the code-index sync (`domains::sync::code`).
pub mod code_sync;
/// Batched `code_symbols` identity, `blob_oid`, line range and snippet read.
pub mod code_text;
/// Connection open: PRAGMAs, migrations, `sqlite-vec` auto-extension.
pub mod connection;
/// The remaining `comemory doctor` health-check SQL: mirror-parity hashes,
/// the repo-root inventory, and the live memory count.
pub mod doctor_probes;
/// `document_fts` insert/delete helpers + the BM25 MATCH query leg.
pub mod document_fts;
/// `document_share` row CRUD — what a local document is called upstream,
/// and why it is withheld when it is.
pub mod document_share;
/// `documents` + `document_chunks` row CRUD.
pub mod documents;
/// FTS5 triplet index over `edges` (rendering + refresh + lexical ladder).
pub mod edge_fts;
/// `edges` table CRUD: typed upserts, weighted accumulation, outgoing
/// neighbors, the `supersedes_chain` recursive walk, and delete-by-node.
pub mod edges;
/// File-neighbor CTE query shared by the edge-store facade.
mod edges_neighbors;
/// Retrieval-side `edges` reads: the memory graph-expansion walk, the
/// context-bundle relation walk, working-set co-change affinity, and the
/// live-supersede lookup.
pub mod edges_retrieval;
/// f32 ↔ `vec0` BLOB encoding plus the per-table dim guards.
pub mod embed;
/// `eval_runs` row insert + newest-first read — one row per `comemory
/// eval`/`tune`/`bandit` run.
pub mod eval_runs;
/// `feedback` row CRUD: per-memory counter table + memory-tagged
/// `feedback_events` inserts, plus the `eval::mine`/`eval::golden` reads over
/// `feedback_events`.
pub mod feedback;
/// FTS5 insert/search helpers for the code leg.
pub mod fts;
/// Memory-leg FTS5 ladder (strict → relaxed → subtoken → expanded).
pub mod fts_memory;
/// `retrieval_log` / `feedback_events` eviction past the retention window,
/// behind `comemory gc`.
pub mod gc_learning;
/// `gc_runs` row insert — one row per `comemory gc` sweep.
pub mod gc_runs;
/// `index_failures` row CRUD: append-only log of swallowed indexing
/// failures, including its ISO 8601 timestamp and its `usize` count clamp.
pub mod index_failures;
/// `index_runs` writer + readers — one row per `index-code` run.
pub mod index_runs;
/// `indexed_files` cursor reads not already owned by `code_row`'s upsert:
/// the repo-wide cursor wipe and the per-`(repo, path)` blob-OID lookup.
pub mod indexed_files;
/// Paginated listing of live memories.
pub mod memory_intent;
pub mod memory_list;
/// Batched per-memory metadata (path, repo, kind, tags, references).
pub mod memory_meta;
/// Hard-delete of one soft-deleted memory's mirror rows (`comemory gc`),
/// plus the soft-delete mirror write behind `comemory delete`.
pub mod memory_purge;
/// Stored memory repository labels, including soft-deleted rows.
pub mod memory_repository;
/// `memories` row upserts and their edge materialization.
pub mod memory_row;
/// Memory activation counters and materialized graph-rank writes.
pub(crate) mod memory_signals;
/// Versioned, idempotent schema migrations plus `schema_meta`.
pub mod migrate;
pub mod needs_embedding;
/// Execute schema-generated statements while retaining store error semantics.
mod orm;
/// `maintenance::prune`'s own scan (orphan-edge count, stale-code-file list, one
/// memory's display fields) and apply-time cleanup deletes.
pub mod prune_apply;
/// The low-quality/zero-incoming-edge and superseded-and-forgotten scans
/// behind `maintenance::retention::low_value`.
pub mod prune_signals;
/// Mined `(term → expansion)` row CRUD behind `comemory mine --apply`.
pub mod query_expansions;
/// Shared random-hex id generation (`/dev/urandom`), the neutral home for
/// both `serve::security` and `maintenance::gc`.
pub mod random_id;
/// The `ATTACH`-based rebuild preservation copy: entry point, `DETACH`
/// guarantee, and the two attached-DB schema-probe helpers.
pub mod rebuild_copy;
/// Rebuild preservation copy: the code-index/mined-edge/repo-marker tables.
pub mod rebuild_copy_code;
/// Rebuild preservation copy: the document-domain tables.
pub mod rebuild_copy_documents;
/// Rebuild preservation copy: the run-history and sync tables.
pub mod rebuild_copy_history;
/// Rebuild preservation copy: feedback counters and the retrieval log.
pub mod rebuild_copy_learning;
/// Rebuild preservation copy: `feedback_events`, `query_expansions`,
/// `bandit_arms`.
pub mod rebuild_copy_learning_events;
/// The three pulled-projection tables a peer's code generation writes:
/// its manifest, its snippet-free symbols and its graph edges.
pub mod remote_code;
/// Repo-scoped reads over the ACTIVE pulled generation: which repos a peer
/// shared, and what the shared side contributes to the code graph.
pub mod remote_code_view;
/// The pulled document cache: the revision of a shared document this machine
/// holds, its passages, its links and its own FTS index.
pub mod remote_document;
/// `replica_cursor` row CRUD — per-workspace upstream position and epoch.
pub mod replica_cursor;
/// `replica-v1` journal writes: the payload row, the feed append and the
/// revision update one accepted mutation owes, in the caller's transaction.
pub mod replica_journal;
/// `replica_operation` row CRUD — the durable outbox of local mutations a
/// peer has not accepted yet.
pub mod replica_outbox;
/// `replica-v1` journal reads: the ordered feed page with the payload each
/// position named, the head, one entity's revision, and the manifest digests.
pub mod replica_read;
/// `replica_receipt` row CRUD — the decision written in the accept
/// transaction and read back on a replay.
pub mod replica_receipt;
/// `replica_staged_part` row CRUD — parts of an oversized revision and their
/// assembly, invisible until activation.
pub mod replica_staging;
/// The abandoned-stage sweep: staged parts and staged generations past their
/// window, and nothing an active generation or a receipt depends on.
pub mod replica_sweep;
/// Drop every code-index row and edge for one repo label.
pub mod repo_drop;
/// `repo_marker.last_mined_commit` — the co-change mining cursor, plus the
/// one-row lazy-reindex probe read.
pub mod repo_marker;
/// Enumerate distinct, canonicalized `repo_marker.root_path` values.
pub mod repo_marker_roots;
/// The `repo_marker` join behind `comemory repos`: one row per indexed repo
/// plus its per-repo file/symbol/memory counters.
pub mod repos_inventory;
/// The label to canonical-repository map the last policy load resolved, so an
/// offline run can tell whether a repository is approved.
pub mod repository_approval;
/// `retrieval_log` reads — the raw `returned_ids` window query behind the
/// search→edit lookback, plus the `(query_id, query, at)` scan behind
/// `eval::mine`.
pub mod retrieval_log;
/// The declared schema: `registry()` over the `#[table]` structs in the
/// `schema_*` siblings, plus `DECLARED_TABLES`.
pub mod schema;
/// Declared code-index tables: `code_symbols`, `code_fts`, `code_vec`, `repo_marker`.
pub mod schema_code;
/// Declared code-generation tables: `code_generation` and the three
/// `remote_code_*` projection tables a pulled generation writes.
pub mod schema_code_generation;
/// Declared tables no domain owns: `schema_meta`, `edge_fts`.
pub mod schema_core;
/// Declared document-replication tables: the `remote_document*` pulled
/// revision cache and the `document_share` local-to-portable mapping.
pub mod schema_document_revision;
/// Declared document tables: `source_roots`, `source_files`, `documents`, `document_chunks`, `document_fts`.
pub mod schema_documents;
/// Declared graph tables: `edges`, `code_ref`.
pub mod schema_graph;
/// Declared run-history tables: `eval_runs`, `gc_runs`, `index_runs`.
pub mod schema_history;
/// `just migration-journal` / `just migration-adopt`: journal a hand-written
/// migration, restate the newest snapshot from the registry.
pub mod schema_journal;
/// Declared learning-loop tables: `feedback`, `retrieval_log`, `bandit_arms`.
pub mod schema_learning;
/// Declared memory-leg virtual tables: `memory_fts`, `memory_vec`.
pub mod schema_memory;
/// Single-key `schema_meta` writers that do not belong to `migrate` or
/// `vector`'s dim guards, plus the generic keyed `get`/`upsert` behind
/// `cli::lazy_reindex`'s debounce marker.
pub mod schema_meta;
/// Declared `replica-v1` journal tables: `replica_stream`, `replica_payload`,
/// `replica_feed`, `replica_revision`, `replica_operation`, `replica_receipt`,
/// `replica_cursor`, `replica_staged_part`.
pub mod schema_replica;
/// Declared cloud-sync tables: `sync_state`, `sync_binding`.
pub mod schema_sync;
/// The ordered live-memory id scan journal seeding resumes from.
pub mod seed_scan;
/// Bulk `(id, simhash)` scan over live memories, shared by save + consolidate.
pub mod simhash_scan;
/// `source_roots` row CRUD — the SQLite mirror of `sources.toml`.
pub mod sources;
/// The corpus counters behind `comemory stats`: a generic scoped
/// `COUNT(*)`, a table-wide `COUNT(*)`, and the logical database size.
pub mod stats_counts;
/// First-push workspace binding + `--allow-secret` overrides.
pub mod sync_binding;
/// Append-only cloud-sync change journal.
pub mod sync_log;
/// The live `content_hash` set behind `GET /sync/manifest`.
pub mod sync_manifest;
/// Per-workspace pull/push cursors.
pub mod sync_state;
/// Custom FTS5 identifier tokenizer (camelCase/snake_case split + FFI).
pub mod tokenizer;
/// The `memories` scan behind `GET /api/v1/trash`: every soft-deleted row,
/// newest deletion first.
pub mod trash_list;
/// `vec0` insert and KNN against `memory_vec` / `code_vec`.
pub mod vector;

/// `?,?,...,?` — `n` comma-joined SQL placeholders for one `IN (...)` clause.
pub(crate) fn qmarks(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// Inclusive `created_at` window restricting a memory query.
///
/// Bounds are pre-normalized ISO-8601 strings compared through SQLite
/// `datetime()`, so mixed stored precision cannot invert the order the way
/// a lexicographic compare would. A `None` bound leaves that side open, so
/// [`CreatedWindow::default`] reproduces an unfiltered query exactly. The
/// borrow-only pair (rather than two parameters) keeps the query helpers
/// within clippy's argument budget and `store` independent of `retrieval`,
/// whose `scope::TimeScope` owns the equivalent strings.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreatedWindow<'a> {
    /// Inclusive lower bound: keep rows created at or after this instant.
    pub since: Option<&'a str>,
    /// Inclusive upper bound: keep rows created at or before this instant.
    pub cutoff: Option<&'a str>,
}

/// The reference edges one memory row owes, already derived and resolved.
///
/// The same borrow-only shape as [`CreatedWindow`], and for the same reason:
/// `store` writes these rows but must not derive them. The regex harvest and
/// the document-resolution policy belong to `domains::graph`, and
/// [`crate::domains::memories::mirror`] — the single seam every memory writer
/// goes through — hands the result here. An all-empty value (its
/// [`Default`]) writes no reference edge at all.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryLinks<'a> {
    /// Bare `<repo>:<path>` targets for `references_file`.
    pub files: &'a [String],
    /// Bare `<repo>:<path>:<symbol>` targets for `references_symbol`.
    pub symbols: &'a [String],
    /// Resolved `documents.id` targets for `references_document`.
    pub documents: &'a [String],
}
