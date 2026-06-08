//! Read-only graph queries used by the retrieval pipeline.
//!
//! Submodules each contribute an `impl Graph` block; Rust allows multiple
//! `impl` blocks across files within the same crate.

pub mod walk;
