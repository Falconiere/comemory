//! Retrieval pipeline over the v0.2 SQLite + sqlite-vec store.
//!
//! [`router`] picks between the pure-vector, pure-lexical, and hybrid
//! (RRF-fused) branches based on whether the caller supplied a vector
//! and/or a non-empty query. [`fuse`] is the Reciprocal Rank Fusion
//! helper used when a caller wants to merge two ranked id lists.
//! [`bundle`] shapes the JSON emitted by `comemory context`.

pub mod bundle;
pub mod fuse;
pub mod router;
