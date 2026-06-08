//! comemory — agentic dev memory + code-aware semantic search.

pub mod prelude;

#[path = "errors.rs"]
pub mod errors;

pub mod config;

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

pub mod cli;

pub mod simhash;
