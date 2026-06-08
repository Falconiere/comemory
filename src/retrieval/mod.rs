//! Retrieval pipeline over the v0.2 SQLite + sqlite-vec store.
//!
//! [`router`] picks between ANN and FTS5 (or runs both with a corrective
//! top-up). [`fuse`] is the Reciprocal Rank Fusion helper used when a
//! caller wants to merge two ranked id lists. [`bundle`] shapes the JSON
//! emitted by `comemory context`.

pub mod bundle;
pub mod fuse;
pub mod router;
