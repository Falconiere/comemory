//! Shared output writers for CLI commands. `tty` renders human-readable lines
//! with `owo-colors`; `json` writes a single line of JSON to stdout. Both
//! route through `writeln!` on the locked standard streams to keep the
//! `no-bypass-check` gate happy.
//!
//! CLI presentation only. Nothing under `serve::` imports this module: a shape
//! both transports serialize belongs to the capability that produces it, so
//! every `envelope` builder lives under `domains::` (#171, and `edges` with
//! #178). `comemory::output` still resolves for library consumers through the
//! crate-root alias in `lib.rs`.

/// Rendering for `comemory consolidate` (cluster blocks + keeper marker).
pub mod consolidate;
/// Rendering for `comemory context` (headline bundle).
pub mod context;
/// Rendering for `comemory edges`; the envelope is `domains::graph::edges_result`'s.
pub mod edges;
/// Rendering for `comemory graph` (relation walks).
pub mod graph;
/// Single-line JSON writer shared by every `--json` surface.
pub mod json;
/// Rendering for `comemory prune` (candidate lists).
pub mod prune;
/// Rendering for `comemory search` (memory hits).
pub mod search;
/// Rendering for `comemory search-code` (code hits).
pub mod search_code;
/// Colored line builders and the shared page footer.
pub mod tty;
