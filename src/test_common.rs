//! Bridge to the shared test fixtures in `tests/common/`, so colocated unit
//! tests and crate-root integration tests share ONE copy of every helper
//! (Binding Rule 1: no duplication).
//!
//! Declared once here because all colocated tests compile into a single crate,
//! where two `#[path]` includes of the same file trip `clippy::duplicate_mod`.
//! `#[path]` on a non-inline module resolves relative to this file's directory
//! (`src/`), so `../tests/common/x.rs` is the crate-root fixture.
#![allow(dead_code)] // shared with the crate-root suite; not all helpers are used here

#[path = "../tests/common/code_rerank_support.rs"]
pub mod code_rerank_support;
#[path = "../tests/common/code_seed.rs"]
pub mod code_seed;
#[path = "../tests/common/document_writer_support.rs"]
pub mod document_writer_support;
#[path = "../tests/common/git_commit.rs"]
pub mod git_commit;
#[path = "../tests/common/git_repo.rs"]
pub mod git_repo;
#[path = "../tests/common/git_sample.rs"]
pub mod git_sample;
#[path = "../tests/common/runner.rs"]
pub mod runner;
#[path = "../tests/common/serve_state.rs"]
pub mod serve_state;
#[path = "../tests/common/vectors.rs"]
pub mod vectors;
