//! comemory — agentic dev memory + code-aware semantic search.

pub mod prelude;

#[path = "errors.rs"]
pub mod errors;

pub mod config;

pub mod eval;

pub mod memory;

pub mod index;

pub mod stats;

pub mod graph;

pub mod retrieval;

pub mod store;

pub mod ast;

pub mod prune;

pub mod git_utils;

pub mod output;

pub mod embed;

pub mod serve;

pub mod cli;

/// Read-only interactive terminal explorer (`comemory tui`).
pub mod tui;

pub mod simhash;
