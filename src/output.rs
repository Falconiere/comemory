//! Shared output helpers for CLI commands. `tty` renders human-readable lines
//! with `owo-colors`; `json` writes a single line of JSON to stdout. Both
//! route through `writeln!` on the locked standard streams to keep the
//! `no-bypass-check` gate happy.

/// Rendering for `comemory consolidate` (cluster blocks + keeper marker).
pub mod consolidate;
/// Rendering for `comemory context` (headline bundle).
pub mod context;
/// Rendering for `comemory edges` (triplet rows + the shared page envelope).
pub mod edges;
/// The console explain strip derived from a hit's `score_parts`.
pub mod explain;
/// Rendering for `comemory graph` (relation walks).
pub mod graph;
/// Single-line JSON writer shared by every `--json` surface.
pub mod json;
/// The generic pagination envelope every paged command serializes.
pub mod page;
/// Rendering for `comemory prune` (candidate lists).
pub mod prune;
/// Rendering for `comemory search` (memory hits).
pub mod search;
/// Rendering for `comemory search-code` (code hits).
pub mod search_code;
/// Colored line builders and the shared page footer.
pub mod tty;
