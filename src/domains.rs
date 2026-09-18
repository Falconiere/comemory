//! Business capabilities.
//!
//! Each capability owns one area of behavior end to end — its models, its
//! algorithms, and the command cores both delivery adapters call — and is
//! declared here as a sibling module file plus its folder. A capability may
//! depend on `store`, `config`, `errors`, `prelude` and the named shared
//! primitives in [`crate::utilities`]; it must never import `cli`, `serve`,
//! `output`, or the legacy `api` command-core tree.
//!
//! Filled one slice at a time by the [#164 migration][m]; every module here is
//! a real capability with production code, never a placeholder.
//!
//! [m]: https://github.com/Falconiere/comemory/issues/164

/// Coding-session transcript capture, client redaction attestation, and
/// explicit-save distillation against the platform.
pub mod capture;

/// AST extraction, code indexing, the repository inventory, and Git hooks.
pub mod code;

/// Document extraction, the durable source registry, and document indexing.
pub mod documents;

/// The `edges` relation graph: derivation, mining, ranking and its queries.
pub mod graph;

/// The learning loop: memory and code feedback, golden sets and metrics,
/// reformulation mining, and the deterministic, sampled and bandit searches
/// over the ranking blend.
pub mod learning;

/// The memory lifecycle: markdown models and store, plus the save, delete,
/// list, show, update, restore, trash and reference-refresh cores.
pub mod memories;

/// Hybrid search across memories, code and documents: the candidate legs,
/// fusion, rerank, diversification, context bundles, code-reference freshness
/// and the search, context, find, suggest and retrieval-config cores.
pub mod retrieval;

/// Org authentication, platform push/pull, the workspace channel, and the
/// Git memory store.
pub mod sync;
