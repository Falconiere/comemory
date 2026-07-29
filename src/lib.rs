//! comemory — agentic dev memory + code-aware semantic search.

pub mod prelude;

/// `thiserror` error enum and the crate-wide `Result` alias.
#[path = "errors.rs"]
pub mod errors;

/// Layered configuration (defaults → file → env) and the data-dir layout.
pub mod config;

/// Retrieval scoring loop: golden sets, metrics, mining, tuning, bandit.
pub mod eval;

/// Markdown source of truth: frontmatter, slug/id, atomic save and load.
pub mod memory;

/// Code indexing entry points over the AST extractor.
pub mod index;

/// Usage, feedback and repo-marker tables inside `comemory.db`.
pub mod stats;

/// The `edges` relation graph: upserts, walks, PageRank, derived refresh.
pub mod graph;

/// Hybrid retrieval: candidate legs, fusion, rerank, diversify, bundles.
pub mod retrieval;

/// Single-file SQLite layer backing memories, code rows, FTS and vectors.
pub mod store;

/// Symbol extraction and AST patterns via ast-grep.
pub mod ast;

/// Orphan / low-value / stale-code detection plus soft-delete and gc.
pub mod prune;

/// Advisory near-duplicate cluster report over live memories (read-only).
pub mod consolidate;

/// Repo and author detection, blob lookup, git-hook installation.
pub mod git_utils;

/// TTY and JSON emitters shared by the subcommands.
pub mod output;

/// Shared embed-command shell-out (`COMEMORY_EMBED_CMD`).
pub mod embed;

/// Loopback web viewer (`comemory serve`).
pub mod serve;

/// clap subcommand entry points and the top-level dispatcher.
pub mod cli;

/// Read-only interactive terminal explorer (`comemory tui`).
pub mod tui;

/// 64-bit SimHash and Hamming distance over tokenized memory bodies.
pub mod simhash;
