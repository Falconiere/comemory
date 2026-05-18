//! Indexing layer: embedders + LanceDB tables. The memory-layer entry point
//! is `MemoryIndex` (vector upsert/search keyed on memory id); the code-layer
//! entry point is `CodeIndex` (vector upsert/walk keyed on `qualified` =
//! `<repo>:<path>:<symbol>`). Both share `Embedder` (nomic for memories,
//! jina-code for code).

pub mod code_index;
pub mod embedder;
pub mod memory_index;
pub mod schema;

pub use code_index::{CodeChunk, CodeIndex};
pub use embedder::Embedder;
pub use memory_index::{MemoryHit, MemoryIndex};

/// Convert a LanceDB L2 distance to a monotone similarity score so callers can
/// sort descending and apply a single threshold across vector queries against
/// any table. The mapping is `1 / (1 + d)` — `d = 0` ⇒ `score = 1.0`, larger
/// distances asymptote toward 0.
///
/// `pub(crate)` because every call site lives inside the qwick-memory crate (index +
/// retrieval modules); we don't want the helper leaking into the public API.
pub(crate) fn score_from_distance(d: f32) -> f32 {
    1.0 / (1.0 + d)
}
