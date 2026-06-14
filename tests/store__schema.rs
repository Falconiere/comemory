//! Test mirror for `src/store/schema.rs`.
//!
//! Asserts that opening a fresh DB applies the full v2 DDL — every
//! base table, virtual table, and index named in
//! `docs/superpowers/specs/2026-06-07-lightweight-v2-design.md` §4.1 is
//! present in `sqlite_master` (except `search_stats`, dropped by the
//! v5 migration), plus the v5 learning-loop tables and the v6
//! `code_feedback` table.

use comemory::store::connection;
use tempfile::tempdir;

#[test]
fn fresh_db_has_all_v2_tables_and_vtabs() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','index')")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .filter_map(Result::ok)
        .collect();

    for expected in [
        "memories",
        "memory_tags",
        "memory_vec",
        "memory_fts",
        "code_symbols",
        "code_vec",
        "code_fts",
        "indexed_files",
        "edges",
        "feedback",
        "feedback_events",
        "query_expansions",
        "code_feedback",
        "schema_meta",
        "idx_memories_repo",
        "idx_edges_src",
        "idx_edges_dst",
        "idx_code_repo_path",
        "idx_code_blob",
        "idx_code_simhash",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing {expected} in sqlite_master"
        );
    }
}
