//! Test binary for the `serve` module. Each submodule mirrors a file
//! under `src/serve/`. `#[path]` is required because Cargo treats this
//! file as a top-level integration-test binary, so unattributed
//! `mod state;` would search `tests/state.rs` rather than
//! `tests/serve/state.rs`.

#[path = "serve/state.rs"]
mod state;
