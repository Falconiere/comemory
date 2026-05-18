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
