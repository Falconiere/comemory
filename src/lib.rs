//! comemory — agentic dev memory + code-aware semantic search.

/// Crate-internal prelude: the `Error`/`Result` alias and common imports.
pub mod prelude;

// DEVIATION D10: lets colocated unit tests under `src/**/tests/` and the shared
// fixtures in `tests/common/` name this crate as `comemory::...` exactly as the
// crate-root integration tests do, so a test file reads the same wherever it
// lives. Zero cost — a self-alias, not a dependency.
extern crate self as comemory;

/// Shared test fixtures (`tests/common/`), reachable from colocated unit tests.
#[cfg(test)]
mod test_common;

/// `thiserror` error enum and the crate-wide `Result` alias.
pub mod errors;

/// Layered configuration (defaults → file → env) and the data-dir layout.
pub mod config;

/// Retrieval scoring loop: golden sets, metrics, mining, tuning, bandit.
pub mod eval;

/// Markdown source of truth: frontmatter, slug/id, atomic save and load.
pub mod memory;

/// Usage, feedback and repo-marker tables inside `comemory.db`.
pub mod stats;

/// The `edges` relation graph: upserts, walks, PageRank, derived refresh.
pub mod graph;

/// Hybrid retrieval: candidate legs, fusion, rerank, diversify, bundles.
pub mod retrieval;

/// Single-file SQLite layer backing memories, code rows, FTS and vectors.
pub mod store;

/// Shared command core: `Ctx` + `api::<cmd>::run`, called by both `cli::`
/// and `serve::routes::` so neither surface duplicates subcommand logic.
pub mod api;

/// Orphan / low-value / stale-code detection plus soft-delete and gc.
pub mod prune;

/// Advisory near-duplicate cluster report over live memories (read-only).
pub mod consolidate;

/// TTY and JSON emitters shared by the subcommands.
pub mod output;

/// Loopback web viewer (`comemory serve`).
pub mod serve;

/// `comemory upgrade`: resolve the newest release and swap the binary.
pub mod upgrade;

/// Cloud platform auth: device login and workspace-key credentials.
pub mod cloud;

/// Client-side session capture and distillation (platform Slices 3–4).
pub mod capture;

/// clap subcommand entry points and the top-level dispatcher.
pub mod cli;

/// Cloud-sync client helpers: match keys, redaction, auth file, allowlist cache.
pub mod sync;

/// Business capabilities, each owning one area of behavior end to end.
pub mod domains;

/// Transport-neutral shared primitives usable by domains and by both
/// delivery adapters.
pub mod utilities;

// Crate-root aliases for modules that were public root modules before the
// migration moved them: `ast` and `git_utils` under `domains::code` (#167),
// `document` and `source` under `domains::documents` (#168), the four shared
// primitives under `utilities` (#166). They preserve `comemory::<name>` for
// external consumers; in-crate code names the real path directly.
pub use domains::code::ast;
pub use domains::code::git_utils;
pub use domains::documents::document;
pub use domains::documents::source;
pub use utilities::embed;
pub use utilities::fetch;
pub use utilities::http_error;
pub use utilities::simhash;
