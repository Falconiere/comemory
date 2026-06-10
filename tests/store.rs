//! Test-binary shim for the store module. Submodules live in tests/store/.

#[path = "common/vectors.rs"]
mod vectors;

#[path = "store/code_row.rs"]
mod code_row;

#[path = "store/connection.rs"]
mod connection;

#[path = "store/embed.rs"]
mod embed;

#[path = "store/fts.rs"]
mod fts;

#[path = "store/memory_row.rs"]
mod memory_row;

#[path = "store/migrate.rs"]
mod migrate;

#[path = "store/schema.rs"]
mod schema;

#[path = "store/vector.rs"]
mod vector;

#[path = "store/tokenizer/ffi.rs"]
mod tokenizer_ffi;

#[path = "store/tokenizer/split.rs"]
mod tokenizer_split;
