//! Retrieval pipeline over the v0.2 SQLite + sqlite-vec store.
//!
//! [`router`] picks between the pure-vector, pure-lexical, and hybrid
//! (RRF-fused) branches based on whether the caller supplied a vector
//! and/or a non-empty query; [`code_route`] is its `code_symbols`-side
//! sibling (BM25 + thresholded ANN + RRF, no relaxation ladder).
//! [`fuse`] is the Reciprocal Rank Fusion
//! helper used when a caller wants to merge two ranked id lists.
//! [`bundle`] shapes the JSON emitted by `comemory context`. [`rerank`]
//! is the second pipeline stage: it multiplies the fused relevance by
//! bounded deterministic priors (activation, feedback, quality,
//! supersede) built from the [`score`] primitives; [`code_rerank`] is
//! its code-side sibling (PageRank, activation, working-set affinity,
//! feedback priors + chunk→parent coalescing). [`diversify`] is the
//! third stage: SimHash near-duplicate collapse followed by Jaccard-MMR
//! greedy selection up to top-k. [`pipeline`] chains all three stages
//! (route → rerank → diversify → top-k) and bumps access tracking; it is
//! the single retrieval entry point used by the CLI.

pub mod bundle;
pub mod code_rerank;
pub mod code_route;
pub mod diversify;
pub mod fuse;
pub mod pipeline;
pub mod rerank;
pub mod router;
pub mod score;
