#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Placeholder test binary for `comemory::index`. The v0.1 submodules
//! (`embedder`, `memory_index`, `code_index`, `schema`, `fts`) were
//! removed in v0.2; vector + lexical coverage now lives under
//! `tests/store/`. This shim stays so the per-module test binary set
//! remains stable for cargo nextest groupings.
