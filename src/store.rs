//! Single-file SQLite storage: one `comemory.db` holding memories, code
//! rows, edges, FTS5 indexes and `sqlite-vec` vtabs. The extension is loaded
//! on every connection; FTS5 is bundled into rusqlite.

/// `code_ref` side table: version-anchor store for explicit code references.
pub mod code_ref;
/// `code_symbols` row upserts (insert, refresh, delete-by-file).
pub mod code_row;
/// Connection open: PRAGMAs, migrations, `sqlite-vec` auto-extension.
pub mod connection;
/// `document_fts` insert/delete helpers + the BM25 MATCH query leg.
pub mod document_fts;
/// `documents` + `document_chunks` row CRUD.
pub mod documents;
/// FTS5 triplet index over `edges` (rendering + refresh + lexical ladder).
pub mod edge_fts;
/// f32 ↔ `vec0` BLOB encoding plus the per-table dim guards.
pub mod embed;
/// `eval_runs` row insert + newest-first read — one row per `comemory
/// eval`/`tune`/`bandit` run.
pub mod eval_runs;
/// FTS5 insert/search helpers for the code leg.
pub mod fts;
/// Memory-leg FTS5 ladder (strict → relaxed → subtoken → expanded).
pub mod fts_memory;
/// `gc_runs` row insert — one row per `comemory gc` sweep.
pub mod gc_runs;
/// `index_runs` writer + readers — one row per `index-code` run.
pub mod index_runs;
/// Paginated listing of live memories.
pub mod memory_list;
/// Batched per-memory metadata (path, repo, kind, tags, references).
pub mod memory_meta;
/// `memories` row upserts and their edge materialization.
pub mod memory_row;
/// Versioned, idempotent schema migrations plus `schema_meta`.
pub mod migrate;
/// Shared random-hex id generation (`/dev/urandom`), the neutral home for
/// both `serve::security` and `api::gc`.
pub mod random_id;
/// Drop every code-index row and edge for one repo label.
pub mod repo_drop;
/// Enumerate distinct, canonicalized `repo_marker.root_path` values.
pub mod repo_marker_roots;
/// DDL strings for the tables, `vec0` vtabs and FTS5 indexes.
pub mod schema;
/// Bulk `(id, simhash)` scan over live memories, shared by save + consolidate.
pub mod simhash_scan;
/// `source_roots` row CRUD — the SQLite mirror of `sources.toml`.
pub mod sources;
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
