//! Indexing layer: embedders + LanceDB tables. The memory-layer entry point
//! is `MemoryIndex` (vector upsert/search keyed on memory id); the code-layer
//! indexer arrives in later tasks but reuses `Embedder::jina_code` from here.

pub mod embedder;
pub mod memory_index;
pub mod schema;

pub use embedder::Embedder;
pub use memory_index::{MemoryHit, MemoryIndex};
