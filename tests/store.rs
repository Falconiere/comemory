//! Test-binary shim for the store module. Submodules live in tests/store/.

#[path = "common/vectors.rs"]
mod vectors;

#[path = "store/connection.rs"]
mod connection;

#[path = "store/embed.rs"]
mod embed;

#[path = "store/fts.rs"]
mod fts;

#[path = "store/migrate.rs"]
mod migrate;

#[path = "store/schema.rs"]
mod schema;

#[path = "store/vector.rs"]
mod vector;
