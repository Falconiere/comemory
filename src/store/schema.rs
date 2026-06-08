//! DDL strings for the v0.2 SQLite schema: `memories`, `memory_tags`,
//! `memory_vec`, `memory_fts`, `code_symbols`, `code_vec`, `code_fts`,
//! `indexed_files`, `edges`, `search_stats`, `feedback`, and
//! `schema_meta`.
//!
//! Skeleton committed by Task 2 of the v0.2 plan so downstream tasks
//! can import `crate::store::schema`. Task 4 fills the full DDL in the
//! sibling `sql/` directory; this module re-exports the SQL via
//! `include_str!` once those files exist.
