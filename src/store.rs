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

/// `bandit_arms` row CRUD: seed/load/record-outcome behind `eval::bandit`.
pub mod bandit_arms;
/// Whether an `Error` wraps SQLite's `SQLITE_BUSY` / `SQLITE_LOCKED`.
pub mod busy;
/// `code_feedback` row CRUD: per-symbol counter table + code-tagged
/// `feedback_events` inserts.
pub mod code_feedback;
/// Dynamic, paginated file→file `edges` window behind `comemory graph`.
pub mod code_graph_edges;
/// `code_symbols` node aggregation behind `comemory graph`'s node assembly.
pub mod code_graph_nodes;
/// `code_ref` side table: version-anchor store for explicit code references,
/// plus the live-symbol-ref scan behind `prune::stale_code`.
pub mod code_ref;
/// `code_symbols` row upserts (insert, refresh, delete-by-file), plus the
/// `symbol_row_exists` ghost-reference resolve check.
pub mod code_row;
/// Per-symbol ranking signals: the `code_symbols` + `code_feedback` join
/// behind `retrieval::code_prior`'s four-prior scorer.
pub mod code_signals;
/// Connection open: PRAGMAs, migrations, `sqlite-vec` auto-extension.
pub mod connection;
/// `document_fts` insert/delete helpers + the BM25 MATCH query leg.
pub mod document_fts;
/// `documents` + `document_chunks` row CRUD.
pub mod documents;
/// FTS5 triplet index over `edges` (rendering + refresh + lexical ladder).
pub mod edge_fts;
/// `edges` table CRUD: typed upserts, weighted accumulation, outgoing
/// neighbors, the `supersedes_chain` recursive walk, and delete-by-node.
pub mod edges;
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
/// `gc_runs` row insert — one row per `comemory gc` sweep.
pub mod gc_runs;
/// `index_failures` row CRUD: append-only log of swallowed indexing
/// failures, behind `crate::stats::sqlite::StatsDb`.
pub mod index_failures;
/// `index_runs` writer + readers — one row per `index-code` run.
pub mod index_runs;
/// Paginated listing of live memories.
pub mod memory_list;
/// Batched per-memory metadata (path, repo, kind, tags, references).
pub mod memory_meta;
/// Hard-delete of one soft-deleted memory's mirror rows (`comemory gc`),
/// plus the soft-delete mirror write behind `comemory delete`.
pub mod memory_purge;
/// `memories` row upserts and their edge materialization.
pub mod memory_row;
/// Versioned, idempotent schema migrations plus `schema_meta`.
pub mod migrate;
/// The low-quality/zero-incoming-edge and superseded-and-forgotten scans
/// behind `prune::low_value`.
pub mod prune_signals;
/// Mined `(term → expansion)` row CRUD behind `comemory mine --apply`.
pub mod query_expansions;
/// Shared random-hex id generation (`/dev/urandom`), the neutral home for
/// both `serve::security` and `api::gc`.
pub mod random_id;
/// Drop every code-index row and edge for one repo label.
pub mod repo_drop;
/// `repo_marker.last_mined_commit` — the co-change mining cursor, plus the
/// one-row lazy-reindex probe read.
pub mod repo_marker;
/// Enumerate distinct, canonicalized `repo_marker.root_path` values.
pub mod repo_marker_roots;
/// `retrieval_log` reads — the raw `returned_ids` window query behind the
/// search→edit lookback, plus the `(query_id, query, at)` scan behind
/// `eval::mine`.
pub mod retrieval_log;
/// DDL strings for the tables, `vec0` vtabs and FTS5 indexes.
pub mod schema;
/// Single-key `schema_meta` writers that do not belong to `migrate` or
/// `vector`'s dim guards, plus the generic keyed `get`/`upsert` behind
/// `cli::lazy_reindex`'s debounce marker.
pub mod schema_meta;
/// Bulk `(id, simhash)` scan over live memories, shared by save + consolidate.
pub mod simhash_scan;
/// `source_roots` row CRUD — the SQLite mirror of `sources.toml`.
pub mod sources;
/// First-push workspace binding + `--allow-secret` overrides.
pub mod sync_binding;
/// Append-only cloud-sync change journal.
pub mod sync_log;
/// Per-workspace pull/push cursors.
pub mod sync_state;
/// Custom FTS5 identifier tokenizer (camelCase/snake_case split + FFI).
pub mod tokenizer;
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
