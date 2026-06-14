//! Shared output helpers for CLI commands. `tty` renders human-readable lines
//! with `owo-colors`; `json` writes a single line of JSON to stdout. Both
//! route through `writeln!` on the locked standard streams to keep the
//! `no-bypass-check` gate happy.

pub mod context;
pub mod graph;
pub mod json;
pub mod page;
pub mod prune;
pub mod search;
pub mod search_code;
pub mod tty;
