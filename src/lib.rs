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

/// Single-file SQLite layer backing memories, code rows, FTS and vectors.
pub mod store;

/// Loopback `/api/v1` HTTP server (`comemory serve`). API-only: the embedded
/// web viewer was removed in 0.18.0.
pub mod serve;

/// clap subcommand entry points and the top-level dispatcher.
pub mod cli;

/// Business capabilities, each owning one area of behavior end to end.
pub mod domains;

/// Transport-neutral shared primitives usable by domains and by both
/// delivery adapters.
pub mod utilities;

// Crate-root aliases for modules that were public root modules before the
// migration moved them: `ast` and `git_utils` under `domains::code` (#167),
// `document` and `source` under `domains::documents` (#168), `memory` under
// `domains::memories` (#169), `graph` under `domains::graph` (#170),
// `retrieval` under `domains::retrieval` (#171), `sync` and `cloud` under
// `domains::sync` (#172), `eval` under `domains::learning::evaluation` (#173),
// `capture` under `domains::capture` (#174), `prune`, `consolidate` and
// `upgrade` under `domains::maintenance` (#176), `output` under `cli` (#178),
// the four shared primitives under `utilities` (#166). They preserve
// `comemory::<name>` for external consumers; in-crate production code names the
// real path directly. `stats` has no alias: #173 removes that tree outright in
// 0.34.0. `api` has none either: #178 deletes the shell, and 0.34.0 removes the
// `comemory::api` tree with it. The two report models are not aliased either —
// `domains::maintenance::retention_report` and `::consolidation_report` are
// siblings of their algorithms, so `comemory::{prune,consolidate}::report` is a
// 0.34.0 removal.
//
// Every alias is written `pub use crate::…`, never the uniform-path
// `pub use domains::…` these once used. The two spell the same export, but
// `scripts/architecture-check.sh`'s resolver rewrites an unqualified binding to
// a relative path and then DROPS the edge, so a domain reaching delivery or a
// sibling capability through an alias became invisible to the gate — which is
// exactly how one `crate::git_utils` call site survived #167 through #177.
// `unqualified root re-export` now fails the gate if this ever regresses.
pub use crate::cli::output;
pub use crate::domains::capture;
pub use crate::domains::code::ast;
pub use crate::domains::code::git_utils;
pub use crate::domains::documents::document;
pub use crate::domains::documents::source;
pub use crate::domains::graph;
pub use crate::domains::learning::evaluation as eval;
pub use crate::domains::maintenance::consolidation as consolidate;
pub use crate::domains::maintenance::retention as prune;
pub use crate::domains::maintenance::upgrade;
pub use crate::domains::memories as memory;
pub use crate::domains::retrieval;
pub use crate::domains::sync;
pub use crate::domains::sync::cloud;
pub use crate::utilities::embed;
pub use crate::utilities::fetch;
pub use crate::utilities::http_error;
pub use crate::utilities::simhash;
