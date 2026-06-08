//! Shared output helpers for CLI commands. `tty` renders human-readable lines
//! with `owo-colors`; `json` writes a single line of JSON to stdout. Both
//! route through `writeln!` on the locked standard streams to keep the
//! `no-bypass-check` gate happy.

pub mod context;
pub mod json;
pub mod search;
pub mod tty;
